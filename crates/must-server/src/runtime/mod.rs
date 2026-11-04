// SPDX-License-Identifier: LGPL-3.0-or-later

//! The running node: shared state ([`Hub`]), module actors, connections.
//!
//! Ordering rests on one rule: every message for a connection is put into that
//! connection's outgoing queue while holding the hub lock, and module actors publish a
//! request's side effects before they hand the result to the connection, which only
//! then queues its reply. Updates therefore always precede the reply they belong to.

mod actor;
mod connection;
mod node;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use must_types::{ErrorClass, Qualifiers, SecopError, Value, data_report};
use tokio::sync::{Notify, broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::events::{ConnectionId, LogLevel, NodeEvent};
use crate::handler::{AppLogLevel, Reading};
use crate::model::{NodeModel, POLLINTERVAL};
use crate::protocol::{self, now};

pub(crate) use actor::ModuleShared;
pub use node::{RunningNode, StartError};

/// `tracing` target of the records a node logs about itself (also sent as
/// [`NodeEvent::Log`]), so that log collectors can tell them apart.
pub const NODE_LOG_TARGET: &str = "must_server::node";

/// A data report as JSON text. A value is serialized once, for the `update` line, the reply
/// and the log alike.
pub(crate) type Report = Arc<str>;

/// The last known state of a parameter.
#[derive(Debug, Clone)]
pub(crate) enum CacheEntry {
    /// Value and its data report as sent to clients.
    Value { value: Value, report: Report },
    /// The last attempt to obtain the value failed.
    Error { error: SecopError, t: f64 },
}

/// Which modules send updates to a connection.
#[derive(Debug, Default)]
struct Activation {
    all: bool,
    /// Modules activated one by one, when not `all`.
    modules: HashSet<String>,
    /// Modules deactivated one by one after a global activation.
    excluded: HashSet<String>,
}

impl Activation {
    fn includes(&self, module: &str) -> bool {
        if self.all {
            !self.excluded.contains(module)
        } else {
            self.modules.contains(module)
        }
    }
}

/// A level of SECoP's remote logging: what a connection asks for, and the level a log
/// record is sent as. Ordered from `Off` to `Debug`; a connection's level admits the
/// records at or below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum RemoteLevel {
    #[default]
    Off,
    Error,
    Info,
    Debug,
}

impl RemoteLevel {
    /// The level of a `logging` request: `"debug"`, `"info"`, `"error"`, `"off"` or
    /// `false`.
    pub(crate) fn from_json(level: &serde_json::Value) -> Option<Self> {
        match level {
            serde_json::Value::Bool(false) => Some(Self::Off),
            serde_json::Value::String(s) => match s.as_str() {
                "off" => Some(Self::Off),
                "error" => Some(Self::Error),
                "info" => Some(Self::Info),
                "debug" => Some(Self::Debug),
                _ => None,
            },
            _ => None,
        }
    }

    /// The level as it appears in a `log` event.
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

impl From<LogLevel> for RemoteLevel {
    /// SECoP has no warning level; must's warnings (failed polls and reads) are what a
    /// client asking for errors wants to see.
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Error | LogLevel::Warn => Self::Error,
            LogLevel::Info => Self::Info,
        }
    }
}

/// The remote logging levels of a connection: one for the whole node, overridden per
/// module.
#[derive(Debug, Default)]
struct LogFilter {
    node: RemoteLevel,
    modules: HashMap<String, RemoteLevel>,
}

impl LogFilter {
    /// Whether a record of `module` at `level` is sent.
    fn admits(&self, module: &str, level: RemoteLevel) -> bool {
        let wanted = self.modules.get(module).copied().unwrap_or(self.node);
        level != RemoteLevel::Off && level <= wanted
    }

    /// Whether nothing is sent at all.
    fn is_off(&self) -> bool {
        self.node == RemoteLevel::Off && self.modules.values().all(|l| *l == RemoteLevel::Off)
    }

    /// Sets the level of one module or, with `None`, of all modules.
    fn set(&mut self, module: Option<&str>, level: RemoteLevel) {
        match module {
            None => {
                self.node = level;
                self.modules.clear();
            }
            Some(m) => {
                self.modules.insert(m.to_owned(), level);
            }
        }
    }
}

/// A line queued for a connection.
pub(crate) struct Outgoing {
    /// The line without its LF, which the writer adds; shared with the
    /// [`NodeEvent::MessageSent`] announcing it.
    pub line: Arc<str>,
    /// Counted in [`QueueStats::updates`]: an update or a `log` event.
    pub is_update: bool,
    /// A `log` event: like the copies of an update, not announced as
    /// [`NodeEvent::MessageSent`], as its record is logged once (and debug records not
    /// at all).
    pub is_log: bool,
}

impl Outgoing {
    /// Bytes on the wire, including the LF.
    pub fn size(&self) -> usize {
        wire_size(&self.line)
    }
}

/// Bytes a line takes on the wire, including its LF.
fn wire_size(line: &str) -> usize {
    line.len() + 1
}

/// Byte counters of a connection's outgoing queue, shared with its writer task.
#[derive(Default)]
pub(crate) struct QueueStats {
    /// All queued bytes; above the limit, the connection's requests are not read.
    pub total: AtomicUsize,
    /// Queued bytes of unsolicited updates and `log` events; an update that would exceed
    /// the limit closes the connection, a `log` event is dropped.
    pub updates: AtomicUsize,
    /// Signalled whenever the writer has sent something.
    pub drained: Notify,
    /// Set by the hub when an update would exceed the limit and the connection is closed.
    pub overflowed: std::sync::atomic::AtomicBool,
}

/// The hub's view of one connection.
struct ConnSink {
    peer: SocketAddr,
    tx: mpsc::UnboundedSender<Outgoing>,
    stats: Arc<QueueStats>,
    activation: Activation,
    logging: LogFilter,
    close: CancellationToken,
}

impl ConnSink {
    fn push(&self, line: Arc<str>, is_update: bool) {
        self.send(Outgoing {
            line,
            is_update,
            is_log: false,
        });
    }

    fn push_log(&self, line: Arc<str>) {
        self.send(Outgoing {
            line,
            is_update: true,
            is_log: true,
        });
    }

    fn send(&self, out: Outgoing) {
        self.stats.total.fetch_add(out.size(), Ordering::AcqRel);
        if out.is_update {
            self.stats.updates.fetch_add(out.size(), Ordering::AcqRel);
        }
        // a send error means the writer is gone; the connection is being closed anyway
        let _ = self.tx.send(out);
    }
}

/// Cached parameter states by module and parameter name. Nested so that lookups take
/// `&str`; entries are shared so that reading one does not copy its value.
type Cache = HashMap<String, HashMap<String, Arc<CacheEntry>>>;

#[derive(Default)]
struct HubState {
    cache: Cache,
    conns: HashMap<ConnectionId, ConnSink>,
    next_id: ConnectionId,
}

/// Shared state of a running node.
pub(crate) struct Hub {
    pub model: Arc<NodeModel>,
    pub report: Arc<str>,
    state: Mutex<HubState>,
    pub events: broadcast::Sender<NodeEvent>,
    /// The log file the node writes now (see [`crate::logfiles`]).
    log_file: Mutex<Option<std::path::PathBuf>>,
    /// Fast poll interval of each module driven by a `pollinterval` parameter, `None`
    /// while stopped, fed by every published value of that parameter.
    poll_intervals: HashMap<String, watch::Sender<Option<Duration>>>,
    /// Connections that asked for log events of some module; changed under the hub lock,
    /// read without it so that a record nobody asked for costs one check.
    logging: AtomicUsize,
}

/// Longest interval a `pollinterval` value sets, as in SHALL.
const MAX_POLL_INTERVAL: f64 = 3600.0;

/// The poll interval a `pollinterval` value sets: `None` for 0 (stopped).
fn poll_interval_of(value: &Value) -> Option<Duration> {
    value
        .as_f64()
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| Duration::from_secs_f64(s.min(MAX_POLL_INTERVAL)))
}

impl Hub {
    pub(crate) fn new(model: Arc<NodeModel>, report: Arc<str>) -> Self {
        let mut state = HubState::default();
        for (mname, module) in &model.modules {
            let entries = state.cache.entry(mname.clone()).or_default();
            for (pname, param) in module.parameters() {
                if let Some(value) = &param.initial
                    && let Ok(json) = param.datainfo.encode(value)
                {
                    let report = report_text(&data_report(json, &Qualifiers::at(now())));
                    let entry = CacheEntry::Value {
                        value: value.clone(),
                        report,
                    };
                    entries.insert(pname.clone(), Arc::new(entry));
                }
            }
        }
        let poll_intervals = model
            .modules
            .iter()
            .filter(|(_, m)| m.poll.driven)
            .map(|(name, m)| {
                let start = match cache_get(&state.cache, name, POLLINTERVAL) {
                    Some(CacheEntry::Value { value, .. }) => poll_interval_of(value),
                    _ => m.poll.fast,
                };
                (name.clone(), watch::Sender::new(start))
            })
            .collect();
        let (events, _) = broadcast::channel(1024);
        Self {
            model,
            report,
            state: Mutex::new(state),
            events,
            log_file: Mutex::new(None),
            poll_intervals,
            logging: AtomicUsize::new(0),
        }
    }

    /// Watches the fast poll interval of a module driven by its `pollinterval`.
    pub(crate) fn watch_poll_interval(
        &self,
        module: &str,
    ) -> Option<watch::Receiver<Option<Duration>>> {
        self.poll_intervals
            .get(module)
            .map(watch::Sender::subscribe)
    }

    fn lock(&self) -> MutexGuard<'_, HubState> {
        // a panic while holding the lock leaves consistent data (all updates are
        // single map operations), so poisoning is ignored
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn emit(&self, event: NodeEvent) {
        // no receivers is fine
        let _ = self.events.send(event);
    }

    /// Whether anyone listens to the node's events; lets callers skip building events.
    pub(crate) fn observed(&self) -> bool {
        self.events.receiver_count() > 0
    }

    /// The log file the node writes now.
    pub(crate) fn log_file(&self) -> Option<std::path::PathBuf> {
        self.log_file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Records the log file the node writes now and announces it as
    /// [`NodeEvent::LogFile`].
    pub(crate) fn set_log_file(&self, path: Option<std::path::PathBuf>) {
        self.log_file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&path);
        self.emit(NodeEvent::LogFile { path });
    }

    /// Logs a record about this node: to `tracing` (target `must_server::node`, with the
    /// node and module as fields), as a [`NodeEvent::Log`] and, for a module's record, as
    /// a `log` event to the connections that asked for it.
    ///
    /// Must not be called while holding the hub lock.
    pub(crate) fn log(&self, level: LogLevel, module: Option<&str>, message: String) {
        let node = self.model.equipment_id.as_str();
        let m = module.unwrap_or_default();
        match level {
            LogLevel::Error => {
                tracing::error!(target: NODE_LOG_TARGET, node, module = m, "{message}");
            }
            LogLevel::Warn => {
                tracing::warn!(target: NODE_LOG_TARGET, node, module = m, "{message}");
            }
            LogLevel::Info => {
                tracing::info!(target: NODE_LOG_TARGET, node, module = m, "{message}");
            }
        }
        if let Some(m) = module {
            self.send_log(m, level.into(), &message);
        }
        if self.observed() {
            self.emit(NodeEvent::Log {
                level,
                module: module.map(str::to_owned),
                message,
            });
        }
    }

    /// Logs a debug record of a module: to `tracing` and as a `log` event, but not as a
    /// [`NodeEvent::Log`], so that debug output stays out of the status display and the
    /// log files.
    ///
    /// Must not be called while holding the hub lock.
    pub(crate) fn log_debug(&self, module: &str, message: &str) {
        let node = self.model.equipment_id.as_str();
        tracing::debug!(target: NODE_LOG_TARGET, node, module, "{message}");
        self.send_log(module, RemoteLevel::Debug, message);
    }

    /// Logs a record of the application for a module, see [`crate::Ctx::log`].
    pub(crate) fn app_log(&self, module: &str, level: AppLogLevel, message: String) {
        let level = match level {
            AppLogLevel::Error => LogLevel::Error,
            AppLogLevel::Warning => LogLevel::Warn,
            AppLogLevel::Info => LogLevel::Info,
            AppLogLevel::Debug => return self.log_debug(module, &message),
        };
        self.log(level, Some(module), message);
    }

    /// Sends a `log` event to every connection whose level for `module` admits `level`.
    /// Best effort: a connection whose unread updates and log events would exceed the
    /// slow-client limit does not get it, so log events never close a connection.
    fn send_log(&self, module: &str, level: RemoteLevel, message: &str) {
        if self.logging.load(Ordering::Acquire) == 0 {
            return;
        }
        let limit = self.model.options.max_outgoing_bytes;
        let mut line: Option<Arc<str>> = None;
        let state = self.lock();
        for conn in state.conns.values() {
            if !conn.logging.admits(module, level) {
                continue;
            }
            let line = line
                .get_or_insert_with(|| protocol::log_event(module, level.as_str(), message).into());
            if conn.stats.updates.load(Ordering::Acquire) + wire_size(line) <= limit {
                conn.push_log(line.clone());
            }
        }
    }

    /// `logging`: sets a connection's level for one module or, with `None`, for all, and
    /// queues the reply, which echoes the level as received.
    pub(crate) fn set_logging(
        &self,
        id: ConnectionId,
        module: Option<&str>,
        level: RemoteLevel,
        echo: &serde_json::Value,
    ) {
        let mut state = self.lock();
        let Some(conn) = state.conns.get_mut(&id) else {
            return;
        };
        let was_off = conn.logging.is_off();
        conn.logging.set(module, level);
        match (was_off, conn.logging.is_off()) {
            (true, false) => {
                self.logging.fetch_add(1, Ordering::AcqRel);
            }
            (false, true) => {
                self.logging.fetch_sub(1, Ordering::AcqRel);
            }
            _ => {}
        }
        conn.push(protocol::logging(module, echo).into(), false);
    }

    // --- connections

    pub(crate) fn register(
        &self,
        peer: SocketAddr,
        tx: mpsc::UnboundedSender<Outgoing>,
        stats: Arc<QueueStats>,
        close: CancellationToken,
    ) -> ConnectionId {
        let mut hub_state = self.lock();
        hub_state.next_id += 1;
        let id = hub_state.next_id;
        hub_state.conns.insert(
            id,
            ConnSink {
                peer,
                tx,
                stats,
                activation: Activation::default(),
                logging: LogFilter::default(),
                close,
            },
        );
        id
    }

    pub(crate) fn unregister(&self, id: ConnectionId) {
        let removed = self.lock().conns.remove(&id);
        self.forget_logging(removed.as_ref());
    }

    /// Updates the logging count for a connection that was removed.
    fn forget_logging(&self, removed: Option<&ConnSink>) {
        if removed.is_some_and(|conn| !conn.logging.is_off()) {
            self.logging.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// Queues a reply for a connection.
    pub(crate) fn reply(&self, id: ConnectionId, line: String) {
        if let Some(conn) = self.lock().conns.get(&id) {
            conn.push(line.into(), false);
        }
    }

    /// `*IDN?`: deactivates all updates and replies with the identification.
    pub(crate) fn identify(&self, id: ConnectionId) {
        let mut state = self.lock();
        if let Some(conn) = state.conns.get_mut(&id) {
            conn.activation = Activation::default();
            conn.push(protocol::identification().into(), false);
        }
    }

    /// Activates updates for one or all modules, queueing the initial updates from the
    /// cache and then the `active` reply.
    pub(crate) fn activate(&self, id: ConnectionId, module: Option<&str>) {
        let mut state = self.lock();
        let HubState { cache, conns, .. } = &mut *state;
        let Some(conn) = conns.get_mut(&id) else {
            return;
        };
        match module {
            None => {
                conn.activation.all = true;
                conn.activation.excluded.clear();
            }
            Some(m) => {
                conn.activation.modules.insert(m.to_owned());
                conn.activation.excluded.remove(m);
            }
        }
        for (mname, m) in &self.model.modules {
            if module.is_some_and(|only| only != mname) {
                continue;
            }
            for (pname, param) in m.parameters() {
                if param.constant.is_some() {
                    continue;
                }
                if let Some(entry) = cache_get(cache, mname, pname) {
                    conn.push(update_line(mname, pname, entry).into(), false);
                }
            }
        }
        conn.push(protocol::activation(true, module).into(), false);
        drop(state);
        self.emit(NodeEvent::ClientActivated {
            id,
            module: module.map(str::to_owned),
        });
    }

    /// Deactivates updates for one or all modules and queues the `inactive` reply.
    pub(crate) fn deactivate(&self, id: ConnectionId, module: Option<&str>) {
        let mut state = self.lock();
        let Some(conn) = state.conns.get_mut(&id) else {
            return;
        };
        match module {
            None => conn.activation = Activation::default(),
            Some(m) => {
                conn.activation.modules.remove(m);
                if conn.activation.all {
                    conn.activation.excluded.insert(m.to_owned());
                }
            }
        }
        conn.push(protocol::activation(false, module).into(), false);
        drop(state);
        self.emit(NodeEvent::ClientDeactivated {
            id,
            module: module.map(str::to_owned),
        });
    }

    // --- values

    /// The cached state of a parameter, shared with the cache.
    pub(crate) fn cached(&self, module: &str, parameter: &str) -> Option<Arc<CacheEntry>> {
        self.lock().cache.get(module)?.get(parameter).cloned()
    }

    /// The cached value of a parameter, if it has one.
    pub(crate) fn cached_value(&self, module: &str, parameter: &str) -> Option<Value> {
        match cache_get(&self.lock().cache, module, parameter) {
            Some(CacheEntry::Value { value, .. }) => Some(value.clone()),
            _ => None,
        }
    }

    /// Stores a new value and sends it to activated connections. Returns the data report,
    /// or an error if the value does not fit the parameter.
    pub(crate) fn publish_value(
        &self,
        module: &str,
        parameter: &str,
        reading: Reading,
    ) -> Result<Report, SecopError> {
        let Some(param) = self
            .model
            .modules
            .get(module)
            .and_then(|m| m.parameter(parameter))
        else {
            self.log(
                LogLevel::Error,
                Some(module),
                format!("update for unknown parameter `{parameter}` ignored"),
            );
            return Err(SecopError::new(
                ErrorClass::InternalError,
                format!("`{module}:{parameter}` is not a parameter"),
            ));
        };
        let json = match param.datainfo.encode(&reading.value) {
            Ok(json) => json,
            Err(e) => {
                self.log(
                    LogLevel::Error,
                    Some(module),
                    format!("value of `{parameter}` does not fit its datainfo: {e}"),
                );
                let error = SecopError::new(
                    ErrorClass::InternalError,
                    format!("the node produced an invalid value: {e}"),
                );
                self.publish_error(module, parameter, error.clone());
                return Err(error);
            }
        };
        let qualifiers = Qualifiers {
            t: Some(reading.t.unwrap_or_else(now)),
            e: reading.e,
        };
        let report = report_text(&data_report(json, &qualifiers));
        if parameter == POLLINTERVAL
            && let Some(interval) = self.poll_intervals.get(module)
        {
            interval.send_replace(poll_interval_of(&reading.value));
        }
        self.store(
            module,
            parameter,
            CacheEntry::Value {
                value: reading.value,
                report: report.clone(),
            },
        );
        Ok(report)
    }

    /// Stores a read error and sends it as `error_update` to activated connections.
    pub(crate) fn publish_error(&self, module: &str, parameter: &str, error: SecopError) {
        let known = self
            .model
            .modules
            .get(module)
            .and_then(|m| m.parameter(parameter))
            .is_some();
        if !known {
            self.log(
                LogLevel::Error,
                Some(module),
                format!("error update for unknown parameter `{parameter}` ignored"),
            );
            return;
        }
        self.store(module, parameter, CacheEntry::Error { error, t: now() });
    }

    fn store(&self, module: &str, parameter: &str, entry: CacheEntry) {
        let observed = self.observed();
        let mut line: Option<Arc<str>> =
            observed.then(|| update_line(module, parameter, &entry).into());
        let limit = self.model.options.max_outgoing_bytes;
        let mut closed = Vec::new();
        {
            let mut state = self.lock();
            let mut overflowing = Vec::new();
            for (id, conn) in &state.conns {
                if !conn.activation.includes(module) {
                    continue;
                }
                let line =
                    line.get_or_insert_with(|| update_line(module, parameter, &entry).into());
                if conn.stats.updates.load(Ordering::Acquire) + wire_size(line) > limit {
                    overflowing.push(*id);
                } else {
                    conn.push(line.clone(), true);
                }
            }
            let entries = match state.cache.get_mut(module) {
                Some(entries) => entries,
                None => state.cache.entry(module.to_owned()).or_default(),
            };
            match entries.get_mut(parameter) {
                Some(slot) => *slot = Arc::new(entry),
                None => {
                    entries.insert(parameter.to_owned(), Arc::new(entry));
                }
            }
            for id in &overflowing {
                if let Some(conn) = state.conns.remove(id) {
                    self.forget_logging(Some(&conn));
                    conn.stats.overflowed.store(true, Ordering::Release);
                    conn.close.cancel();
                    closed.push(conn.peer);
                }
            }
        }
        for peer in closed {
            self.log(
                LogLevel::Warn,
                None,
                format!("closing connection of {peer}: more than {limit} bytes of unread updates"),
            );
        }
        debug!(module, parameter, "updated");
        if observed && let Some(line) = line {
            self.emit(NodeEvent::Updated {
                module: module.to_owned(),
                parameter: parameter.to_owned(),
                line,
            });
        }
    }
}

/// The cached state of `module:parameter`.
fn cache_get<'c>(cache: &'c Cache, module: &str, parameter: &str) -> Option<&'c CacheEntry> {
    cache.get(module)?.get(parameter).map(|e| &**e)
}

/// The text of a data report; faster than through `Display`, and a JSON value cannot fail.
pub(crate) fn report_text(report: &serde_json::Value) -> Report {
    serde_json::to_string(report).unwrap_or_default().into()
}

/// `update` or `error_update` line for a cache entry.
fn update_line(module: &str, parameter: &str, entry: &CacheEntry) -> String {
    match entry {
        CacheEntry::Value { report, .. } => format!("update {module}:{parameter} {report}"),
        CacheEntry::Error { error, t } => {
            let report = must_types::error_report(error, Some(*t));
            format!("error_update {module}:{parameter} {report}")
        }
    }
}
