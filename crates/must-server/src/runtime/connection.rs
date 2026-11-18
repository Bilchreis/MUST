// SPDX-License-Identifier: LGPL-3.0-or-later

//! One client connection: a reader that dispatches requests in order, and a writer that
//! drains the connection's outgoing queue.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};

use indexmap::IndexMap;
use must_types::{ErrorClass, Qualifiers, SecopError, Value, ValueError, data_report};
use serde_json::Value as Json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use super::actor::{ActorHandle, Job, JobResult};
use super::{CacheEntry, Hub, Outgoing, QueueStats, RemoteLevel};
use crate::events::{ConnectionId, NodeEvent};
use crate::model::{AccessibleModel, CommandModel, ModuleModel, ParameterModel};
use crate::protocol::{
    self, Frame, LineSplitter, Request, accessible_part, module_part, now, parse_request,
    split_specifier,
};
use crate::websocket::{self, Opened, Socket};

/// `line` and its LF, in `buf`.
fn with_lf<'b>(buf: &'b mut Vec<u8>, line: &str) -> &'b [u8] {
    buf.clear();
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    buf
}

/// Serves one client until it disconnects or `stop` is cancelled. With `websockets`, a
/// client whose first bytes are `GET /` is served over a WebSocket.
pub(crate) async fn serve(
    hub: Arc<Hub>,
    actors: Arc<HashMap<String, ActorHandle>>,
    stream: TcpStream,
    peer: SocketAddr,
    stop: CancellationToken,
    websockets: bool,
) {
    let _ = stream.set_nodelay(true);
    let (tx, rx) = mpsc::unbounded_channel::<Outgoing>();
    let stats = Arc::new(QueueStats::default());
    // cancelled by node stop, by the hub on overflow, or when the client goes away
    let close = stop.child_token();
    let id = hub.register(peer, tx, Arc::clone(&stats), close.clone());
    info!(%peer, id, "client connected");
    hub.emit(NodeEvent::ClientConnected { id, peer });

    let mut conn = Connection {
        hub: Arc::clone(&hub),
        actors,
        id,
        stats,
        close: close.clone(),
    };
    let opened = if websockets {
        let max_line_length = hub.model.options.max_line_length;
        tokio::select! {
            () = close.cancelled() => Err(conn.closed_reason()),
            opened = websocket::open(stream, max_line_length) => opened,
        }
    } else {
        Ok(Opened::Tcp(stream, Vec::new()))
    };
    let reason = match opened {
        Ok(Opened::Tcp(stream, first)) => {
            let (mut reader, writer) = stream.into_split();
            let source = Source::Raw(&mut reader);
            conn.serve_with(source, &first, Sink::Tcp(writer), rx, &stop)
                .await
        }
        Ok(Opened::WebSocket(socket)) => {
            let (sink, mut messages) = (*socket).split();
            // `R` is unused: any reader type will do
            let source = Source::<OwnedReadHalf>::WebSocket(&mut messages);
            conn.serve_with(source, &[], Sink::WebSocket(sink), rx, &stop)
                .await
        }
        Err(reason) => {
            close.cancel();
            hub.unregister(id);
            reason
        }
    };
    let reason = if stop.is_cancelled() {
        "node stopped".to_owned()
    } else {
        reason
    };
    info!(%peer, id, reason, "client disconnected");
    hub.emit(NodeEvent::ClientDisconnected { id, reason });
}

/// Where a connection's messages go.
enum Sink {
    /// Raw TCP: each message is a line.
    Tcp(OwnedWriteHalf),
    /// A WebSocket: each message is a TEXT frame, without a line ending.
    WebSocket(SplitSink<Socket, Message>),
}

impl Sink {
    /// Sends one message, with `buf` to build it in; `false` if the connection is gone.
    async fn send(&mut self, buf: &mut Vec<u8>, line: &str) -> bool {
        match self {
            Self::Tcp(writer) => writer.write_all(with_lf(buf, line)).await.is_ok(),
            Self::WebSocket(sink) => sink.send(Message::text(line)).await.is_ok(),
        }
    }

    /// Ends the connection; a WebSocket is closed with `frame` first.
    async fn finish(self, frame: CloseFrame) {
        match self {
            Self::Tcp(mut writer) => {
                let _ = writer.shutdown().await;
            }
            Self::WebSocket(mut sink) => {
                // fails at once when the client closed already
                let _ = tokio::time::timeout(Duration::from_secs(1), async {
                    if sink.send(Message::Close(Some(frame))).await.is_ok() {
                        let _ = sink.close().await;
                    }
                })
                .await;
            }
        }
    }
}

struct Connection {
    hub: Arc<Hub>,
    actors: Arc<HashMap<String, ActorHandle>>,
    id: ConnectionId,
    stats: Arc<QueueStats>,
    close: CancellationToken,
}

/// Where a connection's requests come from.
enum Source<'r, R> {
    /// Raw TCP (or a test's stream): the bytes are lines.
    Raw(&'r mut R),
    /// A WebSocket: each message holds lines, the last one with an optional line ending.
    WebSocket(&'r mut SplitStream<Socket>),
}

/// A connection's incoming bytes, split into requests.
struct Incoming<'r, R> {
    source: Source<'r, R>,
    splitter: LineSplitter,
    buf: Vec<u8>,
    /// A frame taken while looking for repeated reads, to be handled next.
    held: Option<Frame>,
    /// Why no more bytes will come.
    ended: Option<String>,
    /// The close frame for the client, when its WebSocket failed.
    failure: Option<CloseFrame>,
}

impl<R: AsyncReadExt + Unpin> Incoming<'_, R> {
    /// Reads more from the client; cancel safe.
    async fn fill(&mut self) {
        match &mut self.source {
            Source::Raw(reader) => match reader.read(&mut self.buf).await {
                Ok(0) => self.ended = Some("closed by client".into()),
                Ok(n) => self.splitter.feed(&self.buf[..n]),
                Err(e) => self.ended = Some(format!("read error: {e}")),
            },
            Source::WebSocket(messages) => {
                let message = messages.next().await;
                self.receive(message);
            }
        }
    }

    /// Passes a WebSocket message on.
    fn receive(&mut self, message: Option<Result<Message, WsError>>) {
        match message {
            None => self.ended = Some("closed by client".into()),
            Some(Ok(Message::Text(text))) => self.feed_message(text.as_bytes()),
            Some(Ok(Message::Binary(data))) => self.feed_message(&data),
            // tungstenite answers pings and a close itself; after a close the stream ends
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                let (reason, frame) = websocket::failure(&e);
                self.ended = Some(reason);
                self.failure = frame;
            }
        }
    }

    /// One message per frame: the line ending is optional.
    fn feed_message(&mut self, data: &[u8]) {
        self.splitter.feed(data);
        if !data.ends_with(b"\n") {
            self.splitter.feed(b"\n");
        }
    }
}

/// How many replies a connection waits for at most while it takes repeated reads, so that
/// a client flooding a slow read cannot grow its queue without limit.
const MAX_REPEATED_READS: usize = 64;

impl Connection {
    /// Serves the client with a writer task draining `rx` into `sink`; returns why it
    /// stopped.
    async fn serve_with<R: AsyncReadExt + Unpin>(
        &mut self,
        source: Source<'_, R>,
        first: &[u8],
        sink: Sink,
        rx: mpsc::UnboundedReceiver<Outgoing>,
        stop: &CancellationToken,
    ) -> String {
        let (failure_tx, failure) = oneshot::channel();
        let writer = self.spawn_writer(sink, rx, failure, stop.clone());
        let (reason, frame) = self.read_loop(source, first).await;
        match frame {
            Some(frame) => drop(failure_tx.send(frame)),
            None => drop(failure_tx),
        }
        self.close.cancel();
        self.hub.unregister(self.id);
        let _ = writer.await;
        reason
    }

    /// Writes the outgoing messages until the connection closes, then ends it.
    fn spawn_writer(
        &self,
        mut sink: Sink,
        mut rx: mpsc::UnboundedReceiver<Outgoing>,
        failure: oneshot::Receiver<CloseFrame>,
        stop: CancellationToken,
    ) -> JoinHandle<()> {
        let (hub, stats, close) = (
            Arc::clone(&self.hub),
            Arc::clone(&self.stats),
            self.close.clone(),
        );
        let id = self.id;
        tokio::spawn(async move {
            // a line and its LF, written at once; reused for every line
            let mut buf = Vec::new();
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
                    written = sink.send(&mut buf, &out.line) => written,
                };
                stats.total.fetch_sub(out.size(), Ordering::AcqRel);
                if out.is_update {
                    stats.updates.fetch_sub(out.size(), Ordering::AcqRel);
                }
                stats.drained.notify_waiters();
                if !written {
                    close.cancel();
                    break;
                }
                debug!(id, "> {}", out.line);
                if !out.is_log && hub.observed() {
                    hub.emit(NodeEvent::MessageSent { id, line: out.line });
                }
            }
            let frame = match failure.await {
                Ok(frame) => frame,
                Err(_) if stop.is_cancelled() => {
                    websocket::close_frame(CloseCode::Away, "node stopped")
                }
                Err(_) if stats.overflowed.load(Ordering::Acquire) => {
                    websocket::close_frame(CloseCode::Policy, "too much unread data")
                }
                Err(_) => websocket::close_frame(CloseCode::Normal, ""),
            };
            sink.finish(frame).await;
        })
    }

    /// Reads and handles requests, after the bytes already read in `first`; returns why
    /// it stopped, and the close frame for a failed WebSocket.
    async fn read_loop<R: AsyncReadExt + Unpin>(
        &mut self,
        source: Source<'_, R>,
        first: &[u8],
    ) -> (String, Option<CloseFrame>) {
        let mut incoming = Incoming {
            source,
            splitter: LineSplitter::new(self.hub.model.options.max_line_length),
            buf: vec![0u8; 16 * 1024],
            held: None,
            ended: None,
            failure: None,
        };
        incoming.splitter.feed(first);
        let reason = self.handle_requests(&mut incoming).await;
        (reason, incoming.failure.take())
    }

    async fn handle_requests<R: AsyncReadExt + Unpin>(
        &mut self,
        incoming: &mut Incoming<'_, R>,
    ) -> String {
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
                    Frame::Line(line) => self.handle(&line, incoming).await,
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
            Request::Activate(spec) => {
                let module = spec.map(module_part);
                if self.known_module("activate", spec, module) {
                    self.hub.activate(self.id, module);
                }
            }
            Request::Deactivate(spec) => {
                let module = spec.map(module_part);
                if self.known_module("deactivate", spec, module) {
                    self.hub.deactivate(self.id, module);
                }
            }
            Request::Read(spec) => self.read(spec, incoming).await,
            Request::Change(spec, data) => self.change(spec, data).await,
            Request::Do(spec, data) => self.call(spec, data).await,
            Request::Check(spec, data) => self.check(spec, data).await,
            Request::Logging(spec, data) => self.logging(spec, data),
            Request::Unsupported { action, specifier } => {
                let message = format!("unknown action `{action}`");
                self.error(
                    action,
                    specifier,
                    SecopError::new(ErrorClass::ProtocolError, message),
                );
            }
        }
    }

    /// Whether `module` (if given) exists; replies with an error for `action` naming `spec`,
    /// the specifier as sent, if not.
    fn known_module(&self, action: &str, spec: Option<&str>, module: Option<&str>) -> bool {
        match module {
            Some(m) if !self.hub.model.modules.contains_key(m) => {
                self.error(action, spec, no_such_module(m));
                false
            }
            _ => true,
        }
    }

    /// Resolves `module:accessible`, replying with an error if that fails.
    fn resolve<'s>(&self, action: &str, spec: Option<&'s str>) -> Option<(Target<'s>, Arc<Hub>)> {
        let used = spec.map(accessible_part);
        let Some((module, accessible)) = used.and_then(split_specifier) else {
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
        let target = Target {
            sent: spec.unwrap_or_default(),
            used: used.unwrap_or_default(),
            module,
            accessible,
        };
        Some((target, Arc::clone(&self.hub)))
    }

    async fn read<R: AsyncReadExt + Unpin>(
        &self,
        spec: Option<&str>,
        incoming: &mut Incoming<'_, R>,
    ) {
        let Some((target, hub)) = self.resolve("read", spec) else {
            return;
        };
        let (module, parameter) = (target.module, target.accessible);
        let Some(param) = hub.model.modules[module].parameter(parameter) else {
            return self.error(
                "read",
                Some(target.sent),
                no_such_parameter(&hub.model.modules[module], parameter),
            );
        };
        // SECoP: a constant "can neither be read nor written". The read is still answered,
        // as Frappy does: the specification names no error for it (sec-node-runtime spec)
        if let Some(constant) = &param.constant {
            let json = param.datainfo.encode(constant).unwrap_or(Json::Null);
            let report = data_report(json, &Qualifiers::at(now()));
            return self.reply(protocol::data_message("reply", target.used, &report));
        }
        if !param.cached {
            return self.read_with_repeats(&target, incoming).await;
        }
        let result = match hub.cached(module, parameter).as_deref() {
            Some(CacheEntry::Value { report, .. }) => Ok(report.clone()),
            Some(CacheEntry::Error { error, .. }) => Err(error.clone()),
            None => Err(SecopError::new(
                ErrorClass::ReadFailed,
                "no value available yet",
            )),
        };
        self.answer("read", "reply", &target, result);
    }

    /// Reads through the module. Until the replies are sent it keeps taking the client's
    /// requests as long as each is a read of the same parameter with the same specifier as
    /// sent: the module may share those with the running read. The first other request is
    /// held for after the replies.
    async fn read_with_repeats<R: AsyncReadExt + Unpin>(
        &self,
        target: &Target<'_>,
        incoming: &mut Incoming<'_, R>,
    ) {
        let (module, parameter) = (target.module, target.accessible);
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
                    self.answer("read", "reply", target, result);
                }
                () = self.close.cancelled() => return,
                frame = next_frame_drained(self, incoming), if looking => match frame {
                    Some(Frame::Line(line)) if is_read_of(&line, target.sent) => {
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

    /// `logging [<module>[:<parameter>]] <level>`. A parameter specifier stands for its
    /// module, which the reply names.
    fn logging(&self, spec: Option<&str>, data: Option<&str>) {
        let module = spec.map(|s| s.split_once(':').map_or(s, |(m, _)| m));
        if let Some(m) = module
            && !self.hub.model.modules.contains_key(m)
        {
            return self.error("logging", spec, no_such_module(m));
        }
        let Some(data) = data else {
            let error = SecopError::new(ErrorClass::ProtocolError, "missing level");
            return self.error("logging", spec, error);
        };
        let json = match parse_json(data) {
            Ok(json) => json,
            Err(e) => return self.error("logging", spec, e),
        };
        let Some(level) = RemoteLevel::from_json(&json) else {
            let error = SecopError::new(
                ErrorClass::WrongType,
                format!(
                    "the level must be \"debug\", \"info\", \"error\", \"off\" or false, not {json}"
                ),
            );
            return self.error("logging", spec, error);
        };
        self.hub.set_logging(self.id, module, level, &json);
    }

    async fn change(&self, spec: Option<&str>, data: Option<&str>) {
        let Some((target, hub)) = self.resolve("change", spec) else {
            return;
        };
        let (module, parameter) = (target.module, target.accessible);
        let m = &hub.model.modules[module];
        let value = m
            .parameter(parameter)
            .ok_or_else(|| no_such_parameter(m, parameter))
            .and_then(|param| {
                writable(param, parameter)?;
                request_value(&hub, module, param, data)
            });
        let value = match value {
            Ok(value) => value,
            Err(e) => return self.error("change", Some(target.sent), e),
        };
        let result = self
            .run(module, |reply| Job::Change {
                parameter: parameter.to_owned(),
                value,
                reply,
            })
            .await;
        self.answer("change", "changed", &target, result);
    }

    async fn call(&self, spec: Option<&str>, data: Option<&str>) {
        let Some((target, hub)) = self.resolve("do", spec) else {
            return;
        };
        let (module, command) = (target.module, target.accessible);
        let m = &hub.model.modules[module];
        let argument = m
            .command(command)
            .ok_or_else(|| no_such_command(m, command))
            .and_then(|cmd| argument(cmd, data));
        let argument = match argument {
            Ok(argument) => argument,
            Err(e) => return self.error("do", Some(target.sent), e),
        };
        let result = self
            .run(module, |reply| Job::Do {
                command: command.to_owned(),
                argument,
                reply,
            })
            .await;
        self.answer("do", "done", &target, result);
    }

    /// `check`: validated like the matching `change` or `do`, then decided by the handler.
    /// Only this connection gets the reply; nothing is cached or published.
    async fn check(&self, spec: Option<&str>, data: Option<&str>) {
        let Some((target, hub)) = self.resolve("check", spec) else {
            return;
        };
        let (module, accessible) = (target.module, target.accessible);
        let m = &hub.model.modules[module];
        let validated = match m.accessibles.get(accessible) {
            None => Err(no_such_parameter(m, accessible)),
            Some(AccessibleModel::Parameter(param)) => writable(param, accessible)
                .and_then(|()| checkable(&param.props, accessible))
                .and_then(|()| request_value(&hub, module, param, data))
                .and_then(|value| Ok((encode(param.datainfo.encode(&value))?, Some(value)))),
            Some(AccessibleModel::Command(cmd)) => checkable(&cmd.props, accessible)
                .and_then(|()| argument(cmd, data))
                .and_then(|value| Ok((encode(cmd.info().encode_argument(value.as_ref()))?, value))),
        };
        let (echo, value) = match validated {
            Ok(validated) => validated,
            Err(e) => return self.error("check", Some(target.sent), e),
        };
        let result = self
            .run(module, |reply| Job::Check {
                accessible: accessible.to_owned(),
                value,
                echo,
                reply,
            })
            .await;
        self.answer("check", "checked", &target, result);
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

    /// Replies naming the part of the specifier used, or with an error naming it as sent.
    fn answer(&self, action: &str, reply_action: &str, target: &Target, result: JobResult) {
        match result {
            Ok(report) => self.reply(protocol::data_message(reply_action, target.used, &report)),
            Err(error) => self.error(action, Some(target.sent), error),
        }
    }
}

/// A resolved `<module>:<accessible>` specifier.
struct Target<'s> {
    /// The specifier as sent, which error replies name.
    sent: &'s str,
    /// Its `<module>:<accessible>` part, which other replies name.
    used: &'s str,
    module: &'s str,
    accessible: &'s str,
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

/// Fails for a readonly or constant parameter.
fn writable(param: &ParameterModel, name: &str) -> Result<(), SecopError> {
    if param.readonly || param.constant.is_some() {
        return Err(SecopError::new(
            ErrorClass::ReadOnly,
            format!("`{name}` is readonly"),
        ));
    }
    Ok(())
}

/// Fails unless the accessible is declared `checkable: true`.
fn checkable(props: &IndexMap<String, Json>, name: &str) -> Result<(), SecopError> {
    if props.get("checkable") == Some(&Json::Bool(true)) {
        return Ok(());
    }
    Err(SecopError::new(
        ErrorClass::NotCheckable,
        format!("`{name}` is not checkable"),
    ))
}

/// The value of a `change` (or `check`) of `param`: parsed and checked against its
/// datainfo and current dynamic limits.
fn request_value(
    hub: &Hub,
    module: &str,
    param: &ParameterModel,
    data: Option<&str>,
) -> Result<Value, SecopError> {
    let data = data.ok_or_else(|| SecopError::new(ErrorClass::ProtocolError, "missing value"))?;
    let json = parse_json(data)?;
    let limits = param.dyn_limits(|name| hub.cached_value(module, name));
    Ok(param.datainfo.decode_request(&json, limits.as_ref())?)
}

/// The argument of a `do` (or `check`) of `cmd`, checked against its datainfo.
fn argument(cmd: &CommandModel, data: Option<&str>) -> Result<Option<Value>, SecopError> {
    let json = data.map(parse_json).transpose()?;
    Ok(cmd.info().decode_argument(json.as_ref())?)
}

/// Encodes a value that was just decoded with the same datainfo.
fn encode(json: Result<Json, ValueError>) -> Result<Json, SecopError> {
    json.map_err(|e| {
        SecopError::new(
            ErrorClass::InternalError,
            format!("cannot encode the checked value: {e}"),
        )
    })
}

fn no_such_module(module: &str) -> SecopError {
    SecopError::new(
        ErrorClass::NoSuchModule,
        format!("`{module}` is not configured on this node"),
    )
}

fn no_such_command(module: &ModuleModel, command: &str) -> SecopError {
    SecopError::new(
        ErrorClass::NoSuchCommand,
        format!("`{}` has no command `{command}`", module.name),
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
        let serving = tokio::spawn(async move {
            let source = Source::Raw(&mut reader);
            connection.read_loop(source, &[]).await.0
        });
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
