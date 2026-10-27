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

use must_types::{ErrorClass, Qualifiers, SecopError, Value, data_report};
use serde_json::Value as Json;
use tokio::sync::{Notify, broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::events::{ConnectionId, LogLevel, NodeEvent};
use crate::handler::Reading;
use crate::model::NodeModel;
use crate::protocol::{self, now};

pub use node::{RunningNode, StartError};

/// `tracing` target of the records a node logs about itself (also sent as
/// [`NodeEvent::Log`]), so that log collectors can tell them apart.
pub const NODE_LOG_TARGET: &str = "must_server::node";

/// The last known state of a parameter.
#[derive(Debug, Clone)]
pub(crate) enum CacheEntry {
    /// Value and its data report as sent to clients.
    Value { value: Value, report: Json },
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

/// A line queued for a connection.
pub(crate) struct Outgoing {
    pub line: Arc<str>,
    pub is_update: bool,
}

/// Byte counters of a connection's outgoing queue, shared with its writer task.
#[derive(Default)]
pub(crate) struct QueueStats {
    /// All queued bytes; above the limit, the connection's requests are not read.
    pub total: AtomicUsize,
    /// Queued bytes of unsolicited updates; above the limit, the connection is closed.
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
    close: CancellationToken,
}

impl ConnSink {
    fn push(&self, line: Arc<str>, is_update: bool) {
        self.stats.total.fetch_add(line.len(), Ordering::AcqRel);
        if is_update {
            self.stats.updates.fetch_add(line.len(), Ordering::AcqRel);
        }
        // a send error means the writer is gone; the connection is being closed anyway
        let _ = self.tx.send(Outgoing { line, is_update });
    }
}

#[derive(Default)]
struct HubState {
    cache: HashMap<(String, String), CacheEntry>,
    conns: HashMap<ConnectionId, ConnSink>,
    next_id: ConnectionId,
}

/// Shared state of a running node.
pub(crate) struct Hub {
    pub model: Arc<NodeModel>,
    pub report: Arc<str>,
    state: Mutex<HubState>,
    pub events: broadcast::Sender<NodeEvent>,
}

impl Hub {
    pub(crate) fn new(model: Arc<NodeModel>, report: Arc<str>) -> Self {
        let mut state = HubState::default();
        for (mname, module) in &model.modules {
            for (pname, param) in module.parameters() {
                if let Some(value) = &param.initial
                    && let Ok(json) = param.datainfo.encode(value)
                {
                    let report = data_report(json, &Qualifiers::at(now()));
                    let entry = CacheEntry::Value {
                        value: value.clone(),
                        report,
                    };
                    state.cache.insert((mname.clone(), pname.clone()), entry);
                }
            }
        }
        let (events, _) = broadcast::channel(1024);
        Self {
            model,
            report,
            state: Mutex::new(state),
            events,
        }
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

    /// Logs a record about this node: to `tracing` (target `must_server::node`, with the
    /// node and module as fields) and as a [`NodeEvent::Log`].
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
        if self.observed() {
            self.emit(NodeEvent::Log {
                level,
                module: module.map(str::to_owned),
                message,
            });
        }
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
                close,
            },
        );
        id
    }

    pub(crate) fn unregister(&self, id: ConnectionId) {
        self.lock().conns.remove(&id);
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
                if let Some(entry) = cache.get(&(mname.clone(), pname.clone())) {
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

    /// The cached state of a parameter.
    pub(crate) fn cached(&self, module: &str, parameter: &str) -> Option<CacheEntry> {
        self.lock()
            .cache
            .get(&(module.to_owned(), parameter.to_owned()))
            .cloned()
    }

    /// The cached value of a parameter, if it has one.
    pub(crate) fn cached_value(&self, module: &str, parameter: &str) -> Option<Value> {
        match self.cached(module, parameter) {
            Some(CacheEntry::Value { value, .. }) => Some(value),
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
    ) -> Result<Json, SecopError> {
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
        let report = data_report(json, &qualifiers);
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
        let line: Arc<str> = update_line(module, parameter, &entry).into();
        let limit = self.model.options.max_outgoing_bytes;
        let mut closed = Vec::new();
        {
            let mut state = self.lock();
            state
                .cache
                .insert((module.to_owned(), parameter.to_owned()), entry);
            let mut overflowing = Vec::new();
            for (id, conn) in &state.conns {
                if !conn.activation.includes(module) {
                    continue;
                }
                if conn.stats.updates.load(Ordering::Acquire) + line.len() > limit {
                    overflowing.push(*id);
                } else {
                    conn.push(line.clone(), true);
                }
            }
            for id in &overflowing {
                if let Some(conn) = state.conns.remove(id) {
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
        if self.observed() {
            self.emit(NodeEvent::Updated {
                module: module.to_owned(),
                parameter: parameter.to_owned(),
                line: line.trim_end_matches('\n').into(),
            });
        }
    }
}

/// `update` or `error_update` line for a cache entry.
fn update_line(module: &str, parameter: &str, entry: &CacheEntry) -> String {
    let specifier = format!("{module}:{parameter}");
    match entry {
        CacheEntry::Value { report, .. } => protocol::data_message("update", &specifier, report),
        CacheEntry::Error { error, t } => {
            let report = must_types::error_report(error, Some(*t));
            format!("error_update {specifier} {report}\n")
        }
    }
}
