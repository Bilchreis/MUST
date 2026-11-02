// SPDX-License-Identifier: LGPL-3.0-or-later

//! One client connection: a reader that dispatches requests in order, and a writer that
//! drains the connection's outgoing queue.

use std::collections::{HashMap, VecDeque};
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
                if hub.observed() {
                    hub.emit(NodeEvent::MessageSent {
                        id,
                        line: line.into(),
                    });
                }
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

/// A connection's incoming bytes, split into requests.
struct Incoming<'r, R> {
    reader: &'r mut R,
    splitter: LineSplitter,
    buf: Vec<u8>,
    /// A frame taken while looking for repeated reads, to be handled next.
    held: Option<Frame>,
    /// Why no more bytes will come.
    ended: Option<String>,
}

impl<R: AsyncReadExt + Unpin> Incoming<'_, R> {
    /// Reads more bytes from the client; cancel safe.
    async fn fill(&mut self) {
        match self.reader.read(&mut self.buf).await {
            Ok(0) => self.ended = Some("closed by client".into()),
            Ok(n) => self.splitter.feed(&self.buf[..n]),
            Err(e) => self.ended = Some(format!("read error: {e}")),
        }
    }
}

/// How many replies a connection waits for at most while it takes repeated reads, so that
/// a client flooding a slow read cannot grow its queue without limit.
const MAX_REPEATED_READS: usize = 64;

impl Connection {
    /// Reads and handles requests; returns why it stopped.
    async fn read_loop(&mut self, reader: &mut (impl AsyncReadExt + Unpin)) -> String {
        let mut incoming = Incoming {
            reader,
            splitter: LineSplitter::new(self.hub.model.options.max_line_length),
            buf: vec![0u8; 16 * 1024],
            held: None,
            ended: None,
        };
        loop {
            while let Some(frame) = incoming
                .held
                .take()
                .or_else(|| incoming.splitter.next_frame())
            {
                // backpressure: do not take the next request while too much is queued
                if !self.drained().await {
                    return self.closed_reason();
                }
                match frame {
                    Frame::Line(line) => self.handle(&line, &mut incoming).await,
                    Frame::TooLong => self.error(
                        "",
                        None,
                        SecopError::new(ErrorClass::ProtocolError, "message too long"),
                    ),
                }
            }
            if let Some(reason) = incoming.ended.take() {
                return reason;
            }
            tokio::select! {
                () = self.close.cancelled() => return self.closed_reason(),
                () = incoming.fill() => {}
            }
        }
    }

    /// Waits until the outgoing queue is within its limit; `false` if the connection closed.
    async fn drained(&self) -> bool {
        let limit = self.hub.model.options.max_outgoing_bytes;
        while self.stats.total.load(Ordering::Acquire) > limit {
            let drained = self.stats.drained.notified();
            if self.stats.total.load(Ordering::Acquire) <= limit {
                break;
            }
            tokio::select! {
                () = self.close.cancelled() => return false,
                () = drained => {}
            }
        }
        true
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

    fn note_received(&self, line: &str) {
        if !line.is_empty() {
            debug!(id = self.id, "< {line}");
            if self.hub.observed() {
                self.hub.emit(NodeEvent::MessageReceived {
                    id: self.id,
                    line: line.into(),
                });
            }
        }
    }

    async fn handle<R: AsyncReadExt + Unpin>(
        &mut self,
        line: &str,
        incoming: &mut Incoming<'_, R>,
    ) {
        self.note_received(line);
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
            Request::Read(spec) => self.read(spec, incoming).await,
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

    async fn read<R: AsyncReadExt + Unpin>(
        &self,
        spec: Option<&str>,
        incoming: &mut Incoming<'_, R>,
    ) {
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
        if !param.cached {
            return self
                .read_with_repeats(module, parameter, spec, incoming)
                .await;
        }
        let result = match hub.cached(module, parameter) {
            Some(CacheEntry::Value { report, .. }) => Ok(report),
            Some(CacheEntry::Error { error, .. }) => Err(error),
            None => Err(SecopError::new(
                ErrorClass::ReadFailed,
                "no value available yet",
            )),
        };
        self.answer("read", "reply", spec, result);
    }

    /// Reads through the module. Until the replies are sent it keeps taking the client's
    /// requests as long as each is a read of the same parameter: the module may share those
    /// with the running read. The first other request is held for after the replies.
    async fn read_with_repeats<R: AsyncReadExt + Unpin>(
        &self,
        module: &str,
        parameter: &str,
        spec: &str,
        incoming: &mut Incoming<'_, R>,
    ) {
        let mut replies = VecDeque::new();
        replies.push_back(self.send_read(module, parameter).await);
        loop {
            let looking = incoming.held.is_none()
                && incoming.ended.is_none()
                && replies.len() < MAX_REPEATED_READS;
            let Some(reply) = replies.front_mut() else {
                return;
            };
            tokio::select! {
                result = reply => {
                    let result = result.unwrap_or_else(|_| Err(shutting_down()));
                    replies.pop_front();
                    self.answer("read", "reply", spec, result);
                }
                () = self.close.cancelled() => return,
                frame = next_frame_drained(self, incoming), if looking => match frame {
                    Some(Frame::Line(line)) if is_read_of(&line, spec) => {
                        self.note_received(&line);
                        replies.push_back(self.send_read(module, parameter).await);
                    }
                    Some(frame) => incoming.held = Some(frame),
                    None => {}
                },
            }
        }
    }

    /// Sends a read to a module; the receiver gets its result.
    async fn send_read(&self, module: &str, parameter: &str) -> oneshot::Receiver<JobResult> {
        let (reply, result) = oneshot::channel();
        let actor = &self.actors[module];
        let job = Job::Read {
            parameter: parameter.to_owned(),
            since: actor.epoch(),
            reply,
        };
        // on failure the sender is dropped, which the receiver reports as shutting down
        let _ = actor.jobs.send(job).await;
        result
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
        self.actors[module]
            .jobs
            .send(job(tx))
            .await
            .map_err(|_| shutting_down())?;
        rx.await.map_err(|_| shutting_down())?
    }

    fn answer(&self, action: &str, reply_action: &str, spec: &str, result: JobResult) {
        match result {
            Ok(report) => self.reply(protocol::data_message(reply_action, spec, &report)),
            Err(error) => self.error(action, Some(spec), error),
        }
    }
}

/// The next frame once the outgoing queue is within its limit; `None` if there was none
/// yet and bytes were read instead. Cancel safe.
async fn next_frame_drained<R: AsyncReadExt + Unpin>(
    connection: &Connection,
    incoming: &mut Incoming<'_, R>,
) -> Option<Frame> {
    if !connection.drained().await {
        return std::future::pending().await;
    }
    if let Some(frame) = incoming.splitter.next_frame() {
        return Some(frame);
    }
    incoming.fill().await;
    None
}

fn is_read_of(line: &str, spec: &str) -> bool {
    matches!(parse_request(line), Request::Read(Some(s)) if s == spec)
}

fn shutting_down() -> SecopError {
    SecopError::new(ErrorClass::InternalError, "the node is shutting down")
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use async_trait::async_trait;
    use must_types::{DataInfo, Value};
    use tokio::io::{AsyncWriteExt, duplex};
    use tokio::sync::Semaphore;
    use tokio_util::task::TaskTracker;

    use super::super::actor::spawn_actor;
    use super::*;
    use crate::handler::{Ctx, HandlerResult, ModuleHandler};
    use crate::{Module, NodeBuilder, Parameter};

    /// Counts its reads; each waits at the gate.
    struct Gated {
        gate: Arc<Semaphore>,
        reads: AtomicUsize,
    }

    #[async_trait]
    impl ModuleHandler for Gated {
        async fn read(&self, _ctx: &Ctx, _parameter: &str) -> HandlerResult {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.gate.acquire().await.unwrap().forget();
            Ok(Value::Double(1.0).into())
        }
    }

    /// The outgoing queue is set directly: how much an unread socket absorbs depends on
    /// the operating system.
    #[tokio::test]
    async fn slow_client_gets_no_following_request_taken() {
        let handler = Arc::new(Gated {
            gate: Arc::new(Semaphore::new(0)),
            reads: AtomicUsize::new(0),
        });
        let value = DataInfo::double().build().unwrap();
        let node = NodeBuilder::new("n", "a node")
            .max_outgoing_bytes(1000)
            .module(
                Module::new("m", "a module")
                    .interface_classes(["Readable"])
                    .parameter(Parameter::new("value", value.clone(), "value"))
                    .unwrap()
                    .parameter(Parameter::new(
                        "status",
                        DataInfo::tuple(&[
                            DataInfo::enumeration([("IDLE", 100)]).build().unwrap(),
                            DataInfo::string().build().unwrap(),
                        ])
                        .build()
                        .unwrap(),
                        "status",
                    ))
                    .unwrap()
                    .no_polling()
                    .handler_arc(Arc::clone(&handler) as Arc<dyn ModuleHandler>),
            )
            .unwrap()
            .build()
            .unwrap();
        let hub = Arc::new(Hub::new(Arc::clone(&node.model), "{}".into()));
        let close = CancellationToken::new();
        let tracker = TaskTracker::new();
        let actor = spawn_actor(&hub, "m", &tracker, close.clone());
        let (tx, mut outgoing) = mpsc::unbounded_channel();
        let stats = Arc::new(QueueStats::default());
        let peer = SocketAddr::from(([127, 0, 0, 1], 1));
        let id = hub.register(peer, tx, Arc::clone(&stats), close.clone());
        let mut connection = Connection {
            hub: Arc::clone(&hub),
            actors: Arc::new(HashMap::from([("m".to_owned(), actor)])),
            id,
            stats: Arc::clone(&stats),
            close: close.clone(),
        };
        let (mut client, mut reader) = duplex(1024);
        let serving = tokio::spawn(async move { connection.read_loop(&mut reader).await });
        let pause = || tokio::time::sleep(Duration::from_millis(100));

        client.write_all(b"read m:value\n").await.unwrap();
        pause().await;
        assert_eq!(handler.reads.load(Ordering::SeqCst), 1);

        // too much is queued while the read is handled: its repeat is not taken
        stats.total.store(2000, Ordering::Release);
        client.write_all(b"read m:value\n").await.unwrap();
        pause().await;
        handler.gate.add_permits(1);
        let first = outgoing.recv().await.unwrap();
        assert!(first.line.starts_with("reply m:value "), "{}", first.line);
        pause().await;
        assert_eq!(
            handler.reads.load(Ordering::SeqCst),
            1,
            "the second read is not taken"
        );

        // the client has received enough: it is taken, after the first reply, so not shared
        stats.total.store(0, Ordering::Release);
        stats.drained.notify_waiters();
        handler.gate.add_permits(1);
        let second = outgoing.recv().await.unwrap();
        assert!(second.line.starts_with("reply m:value "), "{}", second.line);
        assert_eq!(handler.reads.load(Ordering::SeqCst), 2);

        drop(client);
        assert_eq!(serving.await.unwrap(), "closed by client");
        close.cancel();
    }
}
