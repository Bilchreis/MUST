// SPDX-License-Identifier: LGPL-3.0-or-later

//! One task per module that receives its requests and starts them as tasks: one at a
//! time, or up to [`MAX_JOBS`] at once on a module with concurrent requests. Reads of one
//! parameter share a handler call.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use must_types::{ErrorClass, Qualifiers, SecopError, Value, data_report};
use serde_json::Value as Json;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{Hub, Report, report_text};
use crate::events::LogLevel;
use crate::handler::{Ctx, ModuleHandler, Reading};
use crate::protocol::now;

/// The result sent back for a request: a data report, or an error to reply with.
pub(crate) type JobResult = Result<Report, SecopError>;

/// The most jobs a module holds, waiting and running, and the most it runs at once when
/// it has concurrent requests.
const MAX_JOBS: usize = 64;

/// The start hook may take this many times the node's timeout: connecting to the hardware
/// may take longer than a request.
const START_TIMEOUT_FACTOR: u32 = 10;

/// Work for a module actor.
pub(crate) enum Job {
    Read {
        parameter: String,
        /// The module's epoch when the request was sent.
        since: u64,
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
    /// A dry run of a `change` or `do`: the handler decides, nothing is published.
    Check {
        accessible: String,
        value: Option<Value>,
        /// The validated value as sent back in `checked`.
        echo: Json,
        reply: oneshot::Sender<JobResult>,
    },
    /// A read whose result only goes to the cache and activated clients.
    Refresh { parameter: String },
    /// A poll tick: the handler's poll hook with the parameters due.
    Poll { parameters: Vec<String> },
}

/// Who waits for the result of a read.
enum Waiter {
    Client(oneshot::Sender<JobResult>),
    Poll(oneshot::Sender<Result<Reading, SecopError>>),
}

/// A read being done by the handler, shared by everyone in `waiters`.
struct RunningRead {
    id: u64,
    parameter: String,
    /// The module's epoch when the read started.
    start: u64,
    waiters: Vec<Waiter>,
}

#[derive(Default)]
struct ModuleState {
    waiting: VecDeque<Job>,
    /// The latest read of a parameter comes last: later requests can only join that one.
    running: Vec<RunningRead>,
    next_read: u64,
    /// Counts the `change` and `do` jobs that have finished.
    epoch: u64,
}

impl ModuleState {
    fn latest_read(&mut self, parameter: &str) -> Option<&mut RunningRead> {
        self.running
            .iter_mut()
            .rev()
            .find(|r| r.parameter == parameter)
    }

    /// Registers a new read of `parameter`, which takes every waiting request to read it.
    /// They arrived before it started, so its value is recent enough for all of them.
    fn start_read(&mut self, parameter: &str, mut waiters: Vec<Waiter>) -> u64 {
        let mut kept = VecDeque::with_capacity(self.waiting.len());
        for job in self.waiting.drain(..) {
            match job {
                Job::Read {
                    parameter: p,
                    reply,
                    ..
                } if p == parameter => waiters.push(Waiter::Client(reply)),
                Job::Refresh { parameter: p } if p == parameter => {}
                Job::Poll { mut parameters } => {
                    parameters.retain(|p| p != parameter);
                    if !parameters.is_empty() {
                        kept.push_back(Job::Poll { parameters });
                    }
                }
                other => kept.push_back(other),
            }
        }
        self.waiting = kept;
        self.next_read += 1;
        self.running.push(RunningRead {
            id: self.next_read,
            parameter: parameter.to_owned(),
            start: self.epoch,
            waiters,
        });
        self.next_read
    }

    /// Takes a job that was just received: what can share a running read does so, the
    /// rest goes to the waiting list.
    fn receive(&mut self, job: Job) {
        match job {
            Job::Read {
                parameter,
                since,
                reply,
            } => match self.latest_read(&parameter) {
                Some(read) if may_join(read.start, since) => {
                    read.waiters.push(Waiter::Client(reply));
                }
                _ => self.waiting.push_back(Job::Read {
                    parameter,
                    since,
                    reply,
                }),
            },
            Job::Refresh { parameter } => {
                if self.latest_read(&parameter).is_none() {
                    self.waiting.push_back(Job::Refresh { parameter });
                }
            }
            Job::Poll { mut parameters } => {
                parameters.retain(|p| !self.running.iter().any(|r| &r.parameter == p));
                if !parameters.is_empty() {
                    self.waiting.push_back(Job::Poll { parameters });
                }
            }
            job => self.waiting.push_back(job),
        }
    }
}

/// Whether a client's read sent at epoch `since` may share a read started at epoch
/// `start`: only if that read started after every `change` and `do` that had finished
/// when the client sent its request.
fn may_join(start: u64, since: u64) -> bool {
    start >= since
}

/// What a module's actor shares with its jobs and with [`Ctx`].
pub(crate) struct ModuleShared {
    hub: Arc<Hub>,
    module: String,
    handler: Arc<dyn ModuleHandler>,
    concurrent: bool,
    tracker: TaskTracker,
    /// One permit per job that may run: one, or [`MAX_JOBS`] with concurrent requests.
    permits: Arc<Semaphore>,
    state: Mutex<ModuleState>,
    /// Tells the actor to look at the state again.
    changed: Notify,
    /// Signalled when a read has finished.
    read_finished: Notify,
}

/// Handle to send jobs to a module actor.
#[derive(Clone)]
pub(crate) struct ActorHandle {
    pub jobs: mpsc::Sender<Job>,
    shared: Arc<ModuleShared>,
}

impl ActorHandle {
    /// The module's epoch now, to record when sending a [`Job::Read`].
    pub(crate) fn epoch(&self) -> u64 {
        self.shared.lock().epoch
    }
}

/// Starts the actor of `module` and returns its handle.
pub(crate) fn spawn_actor(
    hub: &Arc<Hub>,
    module: &str,
    tracker: &TaskTracker,
    stop: CancellationToken,
) -> ActorHandle {
    let (jobs, mut rx) = mpsc::channel::<Job>(MAX_JOBS);
    let model = &hub.model.modules[module];
    let shared = Arc::new(ModuleShared {
        hub: Arc::clone(hub),
        module: module.to_owned(),
        handler: Arc::clone(&model.handler),
        concurrent: model.concurrent,
        tracker: tracker.clone(),
        permits: Arc::new(Semaphore::new(if model.concurrent { MAX_JOBS } else { 1 })),
        state: Mutex::default(),
        changed: Notify::new(),
        read_finished: Notify::new(),
    });
    let actor = Arc::clone(&shared);
    tracker.spawn(async move {
        // the start hook runs before the actor takes any job: requests, startup reads and
        // polls wait in the channel meanwhile
        let started = tokio::select! {
            biased;
            () = stop.cancelled() => false,
            () = actor.run_start() => true,
        };
        if started {
            loop {
                let (has_waiting, has_room) = actor.load();
                // biased: take everything received first, so that a job about to start sees
                // all the requests it can share with
                tokio::select! {
                    biased;
                    () = stop.cancelled() => break,
                    job = rx.recv(), if has_room => match job {
                        Some(job) => actor.lock().receive(job),
                        None => break,
                    },
                    permit = Arc::clone(&actor.permits).acquire_owned(), if has_waiting => {
                        actor.start_next(permit.expect("the semaphore is never closed"));
                    }
                    () = actor.changed.notified() => {}
                }
            }
        }
        // the connections waiting for these are told that the node is shutting down
        actor.lock().waiting.clear();
    });
    ActorHandle { jobs, shared }
}

impl ModuleShared {
    fn lock(&self) -> MutexGuard<'_, ModuleState> {
        // a panic while holding the lock leaves consistent data: handlers run outside it
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn concurrent(&self) -> bool {
        self.concurrent
    }

    /// Whether jobs wait to start, and whether the module can hold another job.
    fn load(&self) -> (bool, bool) {
        let limit = if self.concurrent { MAX_JOBS } else { 1 };
        let running = limit - self.permits.available_permits();
        let waiting = self.lock().waiting.len();
        (waiting > 0, waiting + running < MAX_JOBS)
    }

    /// Starts the job that waited longest, if any is left.
    fn start_next(self: &Arc<Self>, permit: OwnedSemaphorePermit) {
        let mut state = self.lock();
        match state.waiting.pop_front() {
            Some(Job::Read {
                parameter, reply, ..
            }) => {
                let id = state.start_read(&parameter, vec![Waiter::Client(reply)]);
                drop(state);
                self.spawn_read(id, parameter, Some(permit));
            }
            Some(Job::Refresh { parameter }) => {
                let id = state.start_read(&parameter, Vec::new());
                drop(state);
                self.spawn_read(id, parameter, Some(permit));
            }
            Some(job) => {
                drop(state);
                let shared = Arc::clone(self);
                self.tracker.spawn(async move {
                    shared.run_job(job).await;
                    if !shared.concurrent {
                        shared.reads_finished().await;
                    }
                    drop(permit);
                    shared.changed.notify_one();
                });
            }
            None => {}
        }
    }

    /// Runs a registered read as a task of its own, which goes on when its waiters leave.
    fn spawn_read(
        self: &Arc<Self>,
        id: u64,
        parameter: String,
        permit: Option<OwnedSemaphorePermit>,
    ) {
        let shared = Arc::clone(self);
        self.tracker.spawn(async move {
            let (result, report) = shared.read(&parameter).await;
            let waiters = {
                let mut state = shared.lock();
                let i = state.running.iter().position(|r| r.id == id);
                i.map_or_else(Vec::new, |i| state.running.remove(i).waiters)
            };
            for waiter in waiters {
                match waiter {
                    Waiter::Client(reply) => {
                        let _ = reply.send(report.clone());
                    }
                    Waiter::Poll(reply) => {
                        let _ = reply.send(result.clone());
                    }
                }
            }
            drop(permit);
            shared.read_finished.notify_waiters();
            shared.changed.notify_one();
        });
    }

    /// Waits until no read is running: on a module with one call at a time, a read started
    /// through [`Ctx`] may outlive the job that started it, which timed out.
    async fn reads_finished(&self) {
        loop {
            let finished = self.read_finished.notified();
            if self.lock().running.is_empty() {
                return;
            }
            finished.await;
        }
    }

    /// See [`Ctx::read`].
    pub(crate) async fn read_for_poll(
        self: &Arc<Self>,
        parameter: &str,
    ) -> Result<Reading, SecopError> {
        let (tx, rx) = oneshot::channel();
        let gone = || SecopError::new(ErrorClass::InternalError, "the read was dropped");
        let mut waiter = Some(Waiter::Poll(tx));
        if let Some(read) = self.lock().latest_read(parameter) {
            read.waiters.extend(waiter.take());
        }
        if waiter.is_some() {
            // a module with one call at a time runs this within the job holding its permit
            let permit = if self.concurrent {
                let permit = Arc::clone(&self.permits).acquire_owned().await;
                Some(permit.expect("the semaphore is never closed"))
            } else {
                None
            };
            let mut state = self.lock();
            if let Some(read) = state.latest_read(parameter) {
                read.waiters.extend(waiter.take());
            } else {
                let id = state.start_read(parameter, waiter.take().into_iter().collect());
                drop(state);
                self.spawn_read(id, parameter.to_owned(), permit);
            }
            self.changed.notify_one();
        }
        rx.await.map_err(|_| gone())?
    }

    /// Runs the handler's start hook, logging a failure or a hang.
    async fn run_start(self: &Arc<Self>) {
        let (handler, ctx) = (Arc::clone(&self.handler), self.ctx());
        let limit = self.hub.model.timeout * START_TIMEOUT_FACTOR;
        let result = guarded_for(&self.hub, limit, async move { handler.start(&ctx).await }).await;
        if let Err(error) = result {
            self.hub.log(
                LogLevel::Error,
                Some(&self.module),
                format!("starting the module failed: {error}"),
            );
        }
    }

    async fn run_job(self: &Arc<Self>, job: Job) {
        let (hub, module, handler) = (&self.hub, self.module.as_str(), &self.handler);
        match job {
            // these are started as shared reads
            Job::Read { .. } | Job::Refresh { .. } => {}
            Job::Poll { parameters } => {
                let (handler, ctx) = (Arc::clone(handler), self.ctx());
                let result = guarded(hub, async move {
                    let due: Vec<&str> = parameters.iter().map(String::as_str).collect();
                    handler.poll(&ctx, &due).await;
                    Ok(())
                })
                .await;
                if let Err(error) = result {
                    hub.log(
                        LogLevel::Warn,
                        Some(module),
                        format!("polling failed: {error}"),
                    );
                }
            }
            Job::Change {
                parameter,
                value,
                reply,
            } => {
                let result = {
                    let (handler, ctx, p) = (Arc::clone(handler), self.ctx(), parameter.clone());
                    guarded(hub, async move { handler.change(&ctx, &p, value).await }).await
                };
                let result =
                    result.and_then(|reading| hub.publish_value(module, &parameter, reading));
                self.lock().epoch += 1;
                let _ = reply.send(result);
            }
            Job::Do {
                command,
                argument,
                reply,
            } => {
                let result = {
                    let (handler, ctx, c) = (Arc::clone(handler), self.ctx(), command.clone());
                    guarded(hub, async move { handler.call(&ctx, &c, argument).await }).await
                };
                let result = result.and_then(|reading| done_report(hub, module, &command, reading));
                self.lock().epoch += 1;
                let _ = reply.send(result);
            }
            Job::Check {
                accessible,
                value,
                echo,
                reply,
            } => {
                // changes nothing: no epoch, no published value
                let result = {
                    let (handler, ctx) = (Arc::clone(handler), self.ctx());
                    guarded(
                        hub,
                        async move { handler.check(&ctx, &accessible, value).await },
                    )
                    .await
                };
                let result =
                    result.map(|()| report_text(&data_report(echo, &Qualifiers::at(now()))));
                let _ = reply.send(result);
            }
        }
    }

    /// Reads through the handler and publishes the result (value or error): returns the
    /// reading and the data report.
    async fn read(self: &Arc<Self>, parameter: &str) -> (Result<Reading, SecopError>, JobResult) {
        let (hub, module) = (&self.hub, self.module.as_str());
        let result = {
            let (handler, ctx, p) = (Arc::clone(&self.handler), self.ctx(), parameter.to_owned());
            guarded(hub, async move { handler.read(&ctx, &p).await }).await
        };
        match result {
            Ok(reading) => match hub.publish_value(module, parameter, reading.clone()) {
                Ok(report) => (Ok(reading), Ok(report)),
                Err(error) => (Err(error.clone()), Err(error)),
            },
            Err(error) => {
                hub.log(
                    LogLevel::Warn,
                    Some(module),
                    format!("reading `{parameter}` failed: {error}"),
                );
                hub.publish_error(module, parameter, error.clone());
                (Err(error.clone()), Err(error))
            }
        }
    }

    fn ctx(self: &Arc<Self>) -> Ctx {
        Ctx {
            hub: Arc::clone(&self.hub),
            module: self.module.clone(),
            shared: Arc::clone(self),
        }
    }
}

/// Runs a handler call as its own task, so that a panic becomes `InternalError` and a
/// call exceeding the node's timeout becomes `TimeoutError`.
async fn guarded<T: Send + 'static>(
    hub: &Hub,
    call: impl Future<Output = Result<T, SecopError>> + Send + 'static,
) -> Result<T, SecopError> {
    guarded_for(hub, hub.model.timeout, call).await
}

/// [`guarded`] with another time limit.
async fn guarded_for<T: Send + 'static>(
    hub: &Hub,
    timeout: Duration,
    call: impl Future<Output = Result<T, SecopError>> + Send + 'static,
) -> Result<T, SecopError> {
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
    Ok(report_text(&data_report(
        json,
        &Qualifiers {
            t: Some(t.unwrap_or_else(now)),
            e,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_joins_only_reads_from_after_the_last_finished_change() {
        assert!(may_join(3, 3), "no change finished since the read started");
        assert!(may_join(4, 3), "the read started after a later change");
        assert!(!may_join(3, 4), "a change finished after the read started");
    }

    fn read_job(parameter: &str, since: u64) -> (Job, oneshot::Receiver<JobResult>) {
        let (reply, rx) = oneshot::channel();
        let job = Job::Read {
            parameter: parameter.to_owned(),
            since,
            reply,
        };
        (job, rx)
    }

    #[test]
    fn received_requests_join_by_the_rule() {
        let mut state = ModuleState::default();
        state.start_read("p", Vec::new());

        let (fresh, _rx) = read_job("p", 0);
        state.receive(fresh);
        assert_eq!(state.running[0].waiters.len(), 1);
        assert!(state.waiting.is_empty());

        state.epoch = 1;
        let (late, _rx) = read_job("p", 1);
        state.receive(late);
        assert_eq!(state.running[0].waiters.len(), 1);
        assert_eq!(
            state.waiting.len(),
            1,
            "a change finished: it needs a new read"
        );

        // a poll always joins, whatever the epoch
        state.receive(Job::Poll {
            parameters: vec!["p".into(), "q".into()],
        });
        match state.waiting.back() {
            Some(Job::Poll { parameters }) => assert_eq!(parameters, &["q".to_owned()]),
            _ => panic!("the poll of q waits"),
        }
        state.receive(Job::Poll {
            parameters: vec!["p".into()],
        });
        assert_eq!(state.waiting.len(), 2, "nothing left of this poll");
        state.receive(Job::Refresh {
            parameter: "p".into(),
        });
        assert_eq!(state.waiting.len(), 2, "a refresh joins too");
    }

    #[test]
    fn a_new_read_takes_waiting_reads_and_polls() {
        let mut state = ModuleState::default();
        let (a, _ra) = read_job("p", 0);
        let (other, _ro) = read_job("q", 0);
        let (c, _rc) = read_job("p", 0);
        let (change, _rx) = oneshot::channel();
        state.waiting.push_back(a);
        state.waiting.push_back(other);
        state.waiting.push_back(Job::Change {
            parameter: "p".into(),
            value: Value::Double(1.0),
            reply: change,
        });
        state.waiting.push_back(c);
        state.waiting.push_back(Job::Refresh {
            parameter: "p".into(),
        });
        state.waiting.push_back(Job::Poll {
            parameters: vec!["p".into()],
        });
        state.waiting.push_back(Job::Poll {
            parameters: vec!["p".into(), "q".into()],
        });
        state.start_read("p", Vec::new());
        assert_eq!(state.running[0].waiters.len(), 2, "both reads");
        assert_eq!(state.waiting.len(), 3, "q, the change, and the poll of q");
        assert!(matches!(state.waiting[0], Job::Read { .. }));
        assert!(matches!(state.waiting[1], Job::Change { .. }));
        assert!(matches!(&state.waiting[2], Job::Poll { parameters } if parameters.len() == 1));
    }
}
