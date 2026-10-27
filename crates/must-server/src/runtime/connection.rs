// SPDX-License-Identifier: LGPL-3.0-or-later

//! One client connection: a reader that dispatches requests in order, and a writer that
//! drains the connection's outgoing queue.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use must_types::{ErrorClass, Qualifiers, SecopError, data_report};
use serde_json::Value as Json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use super::actor::{ActorHandle, Job, JobResult};
use super::{CacheEntry, Hub, Outgoing, QueueStats};
use crate::events::{ConnectionId, NodeEvent};
use crate::model::{AccessibleModel, ModuleModel};
use crate::protocol::{self, Frame, LineSplitter, Request, now, parse_request, split_specifier};

/// Serves one client until it disconnects or `stop` is cancelled.
pub(crate) async fn serve(
    hub: Arc<Hub>,
    actors: Arc<HashMap<String, ActorHandle>>,
    stream: TcpStream,
    peer: SocketAddr,
    stop: CancellationToken,
) {
    let _ = stream.set_nodelay(true);
    let (mut reader, mut writer) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Outgoing>();
    let stats = Arc::new(QueueStats::default());
    // cancelled by node stop, by the hub on overflow, or when the client goes away
    let close = stop.child_token();
    let id = hub.register(peer, tx, Arc::clone(&stats), close.clone());
    info!(%peer, id, "client connected");
    hub.emit(NodeEvent::ClientConnected { id, peer });

    let writer_task = {
        let (hub, stats, close) = (Arc::clone(&hub), Arc::clone(&stats), close.clone());
        tokio::spawn(async move {
            loop {
                let out = tokio::select! {
                    () = close.cancelled() => break,
                    out = rx.recv() => match out {
                        Some(out) => out,
                        None => break,
                    },
                };
                let written = tokio::select! {
                    () = close.cancelled() => break,
                    r = writer.write_all(out.line.as_bytes()) => r,
                };
                stats.total.fetch_sub(out.line.len(), Ordering::AcqRel);
                if out.is_update {
                    stats.updates.fetch_sub(out.line.len(), Ordering::AcqRel);
                }
                stats.drained.notify_waiters();
                if written.is_err() {
                    close.cancel();
                    break;
                }
                let line = out.line.trim_end_matches('\n');
                debug!(id, "> {line}");
                hub.emit(NodeEvent::MessageSent {
                    id,
                    line: line.into(),
                });
            }
            let _ = writer.shutdown().await;
        })
    };

    let mut conn = Connection {
        hub: Arc::clone(&hub),
        actors,
        id,
        stats,
        close: close.clone(),
    };
    let reason = conn.read_loop(&mut reader).await;
    close.cancel();
    hub.unregister(id);
    let _ = writer_task.await;
    let reason = if stop.is_cancelled() {
        "node stopped".to_owned()
    } else {
        reason
    };
    info!(%peer, id, reason, "client disconnected");
    hub.emit(NodeEvent::ClientDisconnected { id, reason });
}

struct Connection {
    hub: Arc<Hub>,
    actors: Arc<HashMap<String, ActorHandle>>,
    id: ConnectionId,
    stats: Arc<QueueStats>,
    close: CancellationToken,
}

impl Connection {
    /// Reads and handles requests; returns why it stopped.
    async fn read_loop(&mut self, reader: &mut (impl AsyncReadExt + Unpin)) -> String {
        let limit = self.hub.model.options.max_outgoing_bytes;
        let mut splitter = LineSplitter::new(self.hub.model.options.max_line_length);
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            while let Some(frame) = splitter.next_frame() {
                // backpressure: do not take the next request while too much is queued
                while self.stats.total.load(Ordering::Acquire) > limit {
                    let drained = self.stats.drained.notified();
                    if self.stats.total.load(Ordering::Acquire) <= limit {
                        break;
                    }
                    tokio::select! {
                        () = self.close.cancelled() => return self.closed_reason(),
                        () = drained => {}
                    }
                }
                match frame {
                    Frame::Line(line) => self.handle(&line).await,
                    Frame::TooLong => self.error(
                        "",
                        None,
                        SecopError::new(ErrorClass::ProtocolError, "message too long"),
                    ),
                }
            }
            let n = tokio::select! {
                () = self.close.cancelled() => return self.closed_reason(),
                n = reader.read(&mut buf) => n,
            };
            match n {
                Ok(0) => return "closed by client".into(),
                Ok(n) => splitter.feed(&buf[..n]),
                Err(e) => return format!("read error: {e}"),
            }
        }
    }

    fn closed_reason(&self) -> String {
        if self.stats.overflowed.load(Ordering::Acquire) {
            "too much unread data".into()
        } else {
            "connection lost".into()
        }
    }

    fn reply(&self, line: String) {
        self.hub.reply(self.id, line);
    }

    #[allow(clippy::needless_pass_by_value)]
    fn error(&self, action: &str, specifier: Option<&str>, error: SecopError) {
        self.reply(protocol::error_message(action, specifier, &error));
    }

    async fn handle(&mut self, line: &str) {
        if !line.is_empty() {
            debug!(id = self.id, "< {line}");
            self.hub.emit(NodeEvent::MessageReceived {
                id: self.id,
                line: line.into(),
            });
        }
        match parse_request(line) {
            Request::Empty => {}
            Request::Identify => self.hub.identify(self.id),
            Request::Describe => self.reply(protocol::describing(&self.hub.report)),
            Request::Ping(token) => self.reply(protocol::pong(token, now())),
            Request::Activate(module) | Request::Deactivate(module)
                if module.is_some_and(|m| !self.hub.model.modules.contains_key(m)) =>
            {
                let action = if line.starts_with("activate") {
                    "activate"
                } else {
                    "deactivate"
                };
                let m = module.unwrap_or_default();
                self.error(action, module, no_such_module(m));
            }
            Request::Activate(module) => self.hub.activate(self.id, module),
            Request::Deactivate(module) => self.hub.deactivate(self.id, module),
            Request::Read(spec) => self.read(spec).await,
            Request::Change(spec, data) => self.change(spec, data).await,
            Request::Do(spec, data) => self.call(spec, data).await,
            Request::Unsupported { action, specifier } => {
                let message = if matches!(action, "check" | "logging") {
                    format!("`{action}` is not supported by this node")
                } else {
                    format!("unknown action `{action}`")
                };
                self.error(
                    action,
                    specifier,
                    SecopError::new(ErrorClass::ProtocolError, message),
                );
            }
        }
    }

    /// Resolves `module:accessible`, replying with an error if that fails.
    fn resolve<'s>(
        &self,
        action: &str,
        spec: Option<&'s str>,
    ) -> Option<(&'s str, &'s str, Arc<Hub>)> {
        let Some((module, accessible)) = spec.and_then(split_specifier) else {
            let error = SecopError::new(
                ErrorClass::ProtocolError,
                "expected `<module>:<accessible>`",
            );
            self.error(action, spec, error);
            return None;
        };
        if !self.hub.model.modules.contains_key(module) {
            self.error(action, spec, no_such_module(module));
            return None;
        }
        Some((module, accessible, Arc::clone(&self.hub)))
    }

    async fn read(&self, spec: Option<&str>) {
        let Some((module, parameter, hub)) = self.resolve("read", spec) else {
            return;
        };
        let spec = spec.unwrap_or_default();
        let Some(param) = hub.model.modules[module].parameter(parameter) else {
            return self.error(
                "read",
                Some(spec),
                no_such_parameter(&hub.model.modules[module], parameter),
            );
        };
        if let Some(constant) = &param.constant {
            let json = param.datainfo.encode(constant).unwrap_or(Json::Null);
            let report = data_report(json, &Qualifiers::at(now()));
            return self.reply(protocol::data_message("reply", spec, &report));
        }
        let result = if param.cached {
            match hub.cached(module, parameter) {
                Some(CacheEntry::Value { report, .. }) => Ok(report),
                Some(CacheEntry::Error { error, .. }) => Err(error),
                None => Err(SecopError::new(
                    ErrorClass::ReadFailed,
                    "no value available yet",
                )),
            }
        } else {
            self.run(module, |reply| Job::Read {
                parameter: parameter.to_owned(),
                reply,
            })
            .await
        };
        self.answer("read", "reply", spec, result);
    }

    async fn change(&self, spec: Option<&str>, data: Option<&str>) {
        let Some((module, parameter, hub)) = self.resolve("change", spec) else {
            return;
        };
        let spec = spec.unwrap_or_default();
        let m = &hub.model.modules[module];
        let Some(param) = m.parameter(parameter) else {
            return self.error("change", Some(spec), no_such_parameter(m, parameter));
        };
        if param.readonly || param.constant.is_some() {
            let error = SecopError::new(ErrorClass::ReadOnly, format!("`{parameter}` is readonly"));
            return self.error("change", Some(spec), error);
        }
        let Some(data) = data else {
            let error = SecopError::new(ErrorClass::ProtocolError, "missing value");
            return self.error("change", Some(spec), error);
        };
        let json = match parse_json(data) {
            Ok(json) => json,
            Err(e) => return self.error("change", Some(spec), e),
        };
        let limits = param.dyn_limits(|name| hub.cached_value(module, name));
        let value = match param.datainfo.decode_request(&json, limits.as_ref()) {
            Ok(value) => value,
            Err(e) => return self.error("change", Some(spec), e.into()),
        };
        let result = self
            .run(module, |reply| Job::Change {
                parameter: parameter.to_owned(),
                value,
                reply,
            })
            .await;
        self.answer("change", "changed", spec, result);
    }

    async fn call(&self, spec: Option<&str>, data: Option<&str>) {
        let Some((module, command, hub)) = self.resolve("do", spec) else {
            return;
        };
        let spec = spec.unwrap_or_default();
        let Some(cmd) = hub.model.modules[module].command(command) else {
            let error = SecopError::new(
                ErrorClass::NoSuchCommand,
                format!("`{module}` has no command `{command}`"),
            );
            return self.error("do", Some(spec), error);
        };
        let json = match data.map(parse_json).transpose() {
            Ok(json) => json,
            Err(e) => return self.error("do", Some(spec), e),
        };
        let argument = match cmd.info().decode_argument(json.as_ref()) {
            Ok(argument) => argument,
            Err(e) => return self.error("do", Some(spec), e.into()),
        };
        let result = self
            .run(module, |reply| Job::Do {
                command: command.to_owned(),
                argument,
                reply,
            })
            .await;
        self.answer("do", "done", spec, result);
    }

    /// Sends a job to a module actor and waits for its result.
    async fn run(
        &self,
        module: &str,
        job: impl FnOnce(oneshot::Sender<JobResult>) -> Job,
    ) -> JobResult {
        let (tx, rx) = oneshot::channel();
        let gone = || SecopError::new(ErrorClass::InternalError, "the node is shutting down");
        self.actors[module]
            .send(job(tx))
            .await
            .map_err(|_| gone())?;
        rx.await.map_err(|_| gone())?
    }

    fn answer(&self, action: &str, reply_action: &str, spec: &str, result: JobResult) {
        match result {
            Ok(report) => self.reply(protocol::data_message(reply_action, spec, &report)),
            Err(error) => self.error(action, Some(spec), error),
        }
    }
}

fn parse_json(data: &str) -> Result<Json, SecopError> {
    serde_json::from_str(data).map_err(|e| SecopError::new(ErrorClass::BadJSON, e.to_string()))
}

fn no_such_module(module: &str) -> SecopError {
    SecopError::new(
        ErrorClass::NoSuchModule,
        format!("`{module}` is not configured on this node"),
    )
}

fn no_such_parameter(module: &ModuleModel, parameter: &str) -> SecopError {
    let message = match module.accessibles.get(parameter) {
        Some(AccessibleModel::Command(_)) => format!("`{parameter}` is a command, not a parameter"),
        _ => format!("`{}` has no parameter `{parameter}`", module.name),
    };
    SecopError::new(ErrorClass::NoSuchParameter, message)
}
