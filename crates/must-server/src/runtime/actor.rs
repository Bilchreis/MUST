// SPDX-License-Identifier: LGPL-3.0-or-later

//! One task per module that calls the module's handler for one request at a time.

use std::sync::Arc;
use std::time::Duration;

use must_types::{ErrorClass, Qualifiers, SecopError, Value, data_report};
use serde_json::Value as Json;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::Hub;
use crate::events::LogLevel;
use crate::handler::{Ctx, ModuleHandler, Reading};
use crate::protocol::now;

/// The result sent back for a request: a data report, or an error to reply with.
pub(crate) type JobResult = Result<Json, SecopError>;

/// Work for a module actor.
pub(crate) enum Job {
    Read {
        parameter: String,
        reply: oneshot::Sender<JobResult>,
    },
    Change {
        parameter: String,
        value: Value,
        reply: oneshot::Sender<JobResult>,
    },
    Do {
        command: String,
        argument: Option<Value>,
        reply: oneshot::Sender<JobResult>,
    },
    /// A read whose result only goes to the cache and activated clients.
    Poll { parameter: String },
}

/// Handle to send jobs to a module actor.
pub(crate) type ActorHandle = mpsc::Sender<Job>;

/// Starts the actor of `module` and returns its handle.
pub(crate) fn spawn_actor(
    hub: &Arc<Hub>,
    module: &str,
    tracker: &tokio_util::task::TaskTracker,
    stop: CancellationToken,
) -> ActorHandle {
    let (tx, mut rx) = mpsc::channel::<Job>(64);
    let hub = Arc::clone(hub);
    let module = module.to_owned();
    let handler = Arc::clone(&hub.model.modules[&module].handler);
    tracker.spawn(async move {
        loop {
            let job = tokio::select! {
                () = stop.cancelled() => break,
                job = rx.recv() => match job {
                    Some(job) => job,
                    None => break,
                },
            };
            run_job(&hub, &module, &handler, job).await;
        }
    });
    tx
}

async fn run_job(hub: &Arc<Hub>, module: &str, handler: &Arc<dyn ModuleHandler>, job: Job) {
    match job {
        Job::Read { parameter, reply } => {
            let result = read(hub, module, handler, &parameter).await;
            let _ = reply.send(result);
        }
        Job::Poll { parameter } => {
            let _ = read(hub, module, handler, &parameter).await;
        }
        Job::Change {
            parameter,
            value,
            reply,
        } => {
            let result = {
                let (handler, ctx, p) = (Arc::clone(handler), ctx(hub, module), parameter.clone());
                guarded(hub, async move { handler.change(&ctx, &p, value).await }).await
            };
            let result = result.and_then(|reading| hub.publish_value(module, &parameter, reading));
            let _ = reply.send(result);
        }
        Job::Do {
            command,
            argument,
            reply,
        } => {
            let result = {
                let (handler, ctx, c) = (Arc::clone(handler), ctx(hub, module), command.clone());
                guarded(hub, async move { handler.call(&ctx, &c, argument).await }).await
            };
            let result = result.and_then(|reading| done_report(hub, module, &command, reading));
            let _ = reply.send(result);
        }
    }
}

/// Reads through the handler and publishes the result (value or error).
async fn read(
    hub: &Arc<Hub>,
    module: &str,
    handler: &Arc<dyn ModuleHandler>,
    parameter: &str,
) -> JobResult {
    let result = {
        let (handler, ctx, p) = (Arc::clone(handler), ctx(hub, module), parameter.to_owned());
        guarded(hub, async move { handler.read(&ctx, &p).await }).await
    };
    match result {
        Ok(reading) => hub.publish_value(module, parameter, reading),
        Err(error) => {
            hub.log(
                LogLevel::Warn,
                Some(module),
                format!("reading `{parameter}` failed: {error}"),
            );
            hub.publish_error(module, parameter, error.clone());
            Err(error)
        }
    }
}

fn ctx(hub: &Arc<Hub>, module: &str) -> Ctx {
    Ctx {
        hub: Arc::clone(hub),
        module: module.to_owned(),
    }
}

/// Runs a handler call as its own task, so that a panic becomes `InternalError` and a
/// call exceeding the node's timeout becomes `TimeoutError`.
async fn guarded<T: Send + 'static>(
    hub: &Hub,
    call: impl Future<Output = Result<T, SecopError>> + Send + 'static,
) -> Result<T, SecopError> {
    let timeout: Duration = hub.model.timeout;
    let task = tokio::spawn(call);
    let abort = task.abort_handle();
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(join_error)) => {
            hub.log(
                LogLevel::Error,
                None,
                format!("handler panicked: {join_error}"),
            );
            Err(SecopError::new(
                ErrorClass::InternalError,
                "the handler panicked",
            ))
        }
        Err(_) => {
            abort.abort();
            Err(SecopError::new(
                ErrorClass::TimeoutError,
                format!("no answer within {} s", timeout.as_secs_f64()),
            ))
        }
    }
}

/// The data report of a `done` reply.
fn done_report(hub: &Hub, module: &str, command: &str, reading: Option<Reading>) -> JobResult {
    let info = hub.model.modules[module]
        .command(command)
        .expect("dispatched as command")
        .info();
    let (value, t, e) = match reading {
        Some(r) => (Some(r.value), r.t, r.e),
        None => (None, None, None),
    };
    let json = info.encode_result(value.as_ref()).map_err(|err| {
        SecopError::new(
            ErrorClass::InternalError,
            format!("the node produced an invalid result: {err}"),
        )
    })?;
    Ok(data_report(
        json,
        &Qualifiers {
            t: Some(t.unwrap_or_else(now)),
            e,
        },
    ))
}
