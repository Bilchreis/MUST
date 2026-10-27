// SPDX-License-Identifier: LGPL-3.0-or-later

//! The process's monitoring endpoint (see the `must-monitor` crate for the protocol,
//! the registry and the client library).
//!
//! The endpoint opens when the process gets its first node and closes when the last one
//! is gone. Nodes started with [`Node::start`](crate::Node::start) or
//! [`Node::spawn`](crate::Node::spawn) register themselves; code that knows nodes before
//! they run (the SHALL C ABI) registers them with [`ExternalNode`]. Monitors get a
//! snapshot and then events, through bounded queues, so a slow monitor never delays a
//! node. The endpoint runs on its own thread, which ends with the endpoint.
//!
//! Monitoring is on by default; `MUST_MONITOR=0` or [`set_enabled`] turns it off.
//!
//! Log records: what nodes log about themselves is always captured. Other `tracing`
//! records of the process are captured by [`log_layer`], which hosts add to their
//! subscriber, or [`install_log_capture`] for hosts without one.

mod follow;
mod logs;
mod peer;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use indexmap::IndexMap;
use must_monitor::protocol::{
    ClientId, ClientInfo, Event, LogRecord, NodeId, NodeInfo, NodeState, ParameterState,
    ProcessInfo, ServerMessage, Snapshot,
};
use must_monitor::registry::{self, Registration, RegistrationFile};
use must_monitor::transport::Listener;
use serde_json::Value as Json;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

pub use logs::{LogLayer, install_log_capture, log_layer};

use crate::runtime::{CacheEntry, RunningNode};

/// Environment variable that turns monitoring off with `0`.
pub const ENABLE_VARIABLE: &str = "MUST_MONITOR";

/// How many log records the endpoint keeps for snapshots.
const LOG_BUFFER: usize = 2000;

static DISABLED_BY_API: AtomicBool = AtomicBool::new(false);

/// Turns monitoring on or off for nodes registered from now on (on by default, unless
/// `MUST_MONITOR=0`).
pub fn set_enabled(enabled: bool) {
    DISABLED_BY_API.store(!enabled, Ordering::Relaxed);
}

static REGISTRY_DIR: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

/// Uses `dir` instead of the default registry directory for endpoints opened from now
/// on (for tests and special setups; readers must look there too).
pub fn set_registry_dir(dir: Option<std::path::PathBuf>) {
    *REGISTRY_DIR.lock().unwrap_or_else(PoisonError::into_inner) = dir;
    OPEN_FAILED.store(false, Ordering::Relaxed);
}

fn registry_dir() -> Result<std::path::PathBuf, registry::RegistryError> {
    let chosen = REGISTRY_DIR
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    match chosen {
        Some(dir) => registry::prepare_dir(&dir).map(|()| dir),
        None => registry::registry_dir(),
    }
}

/// Whether nodes registered now get monitored.
pub fn is_enabled() -> bool {
    !DISABLED_BY_API.load(Ordering::Relaxed)
        && std::env::var(ENABLE_VARIABLE).map_or(true, |v| v.trim() != "0")
}

/// A node known to the monitor.
struct NodeEntry {
    equipment_id: String,
    context: Option<String>,
    state: NodeState,
    messages: Vec<String>,
    running: Option<RunningNode>,
    address: Option<String>,
    clients: IndexMap<ClientId, ClientInfo>,
    /// Messages received and sent, and their values one counter period ago.
    rx: u64,
    tx: u64,
    last_rx: u64,
    last_tx: u64,
    /// The task following the node's events.
    follower: Option<tokio::task::AbortHandle>,
}

/// State shared by the endpoint's tasks, guarded by one lock that is held only briefly
/// and never while calling out or logging.
struct Inner {
    next_node: NodeId,
    nodes: IndexMap<NodeId, NodeEntry>,
    peers: Vec<Arc<peer::Peer>>,
    logs: VecDeque<LogRecord>,
}

struct Shared {
    process: ProcessInfo,
    inner: Mutex<Inner>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Inner {
    /// Sends a message to every connected monitor.
    fn broadcast(&self, message: &ServerMessage) {
        for peer in &self.peers {
            peer.push(message.clone());
        }
    }

    fn event(&self, event: Event) {
        self.broadcast(&ServerMessage::Event(event));
    }

    fn log(&mut self, record: LogRecord) {
        if self.logs.len() >= LOG_BUFFER {
            self.logs.pop_front();
        }
        self.logs.push_back(record.clone());
        self.event(Event::Log(record));
    }

    fn snapshot(&self, process: &ProcessInfo) -> Snapshot {
        Snapshot {
            process: process.clone(),
            nodes: self
                .nodes
                .iter()
                .map(|(id, entry)| node_info(*id, entry))
                .collect(),
            logs: self.logs.iter().cloned().collect(),
        }
    }
}

/// A node as listed in snapshots, with values from the node's cache (never read from
/// the hardware).
fn node_info(id: NodeId, entry: &NodeEntry) -> NodeInfo {
    let (structure, parameters) = match &entry.running {
        Some(node) => (Some(structure(node)), parameters(node)),
        None => (None, Vec::new()),
    };
    NodeInfo {
        id,
        equipment_id: entry.equipment_id.clone(),
        address: entry.address.clone(),
        state: entry.state,
        context: entry.context.clone(),
        messages: entry.messages.clone(),
        structure,
        parameters,
        clients: entry.clients.values().cloned().collect(),
    }
}

fn structure(node: &RunningNode) -> Json {
    serde_json::from_str(&node.hub().report).unwrap_or(Json::Null)
}

fn parameters(node: &RunningNode) -> Vec<ParameterState> {
    let hub = node.hub();
    let mut states = Vec::new();
    for (module, m) in &hub.model.modules {
        for (parameter, _) in m.parameters() {
            if let Some(entry) = hub.cached(module, parameter) {
                states.push(parameter_state(module, parameter, &entry));
            }
        }
    }
    states
}

fn parameter_state(module: &str, parameter: &str, entry: &CacheEntry) -> ParameterState {
    let (report, error) = match entry {
        CacheEntry::Value { report, .. } => (report.clone(), false),
        CacheEntry::Error { error, t } => (must_types::error_report(error, Some(*t)), true),
    };
    ParameterState {
        module: module.to_owned(),
        parameter: parameter.to_owned(),
        report,
        error,
    }
}

/// The open endpoint.
struct Monitor {
    shared: Arc<Shared>,
    handle: Handle,
    shutdown: CancellationToken,
    thread: Option<JoinHandle<()>>,
    _registration: RegistrationFile,
}

static MONITOR: Mutex<Option<Monitor>> = Mutex::new(None);

fn monitor() -> MutexGuard<'static, Option<Monitor>> {
    MONITOR.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Opens the endpoint: its thread with a runtime, the listener and the registration.
fn open() -> std::io::Result<Monitor> {
    let process = must_monitor::platform::current_process()?;
    let dir = registry_dir().map_err(std::io::Error::other)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let listener = {
        let _guard = runtime.enter();
        Listener::bind(&dir)
    };
    let (listener, endpoint) = listener?;
    let registration = registry::register(&dir, &Registration::new(&process, endpoint))
        .map_err(std::io::Error::other)?;
    let shared = Arc::new(Shared {
        process: ProcessInfo {
            pid: process.pid,
            name: process.name.clone(),
            library_version: env!("CARGO_PKG_VERSION").to_owned(),
            start_time: process.start_unix,
        },
        inner: Mutex::new(Inner {
            next_node: 0,
            nodes: IndexMap::new(),
            peers: Vec::new(),
            logs: VecDeque::new(),
        }),
    });
    let shutdown = CancellationToken::new();
    let handle = runtime.handle().clone();
    handle.spawn(peer::accept_loop(
        Arc::clone(&shared),
        listener,
        shutdown.clone(),
    ));
    handle.spawn(follow::counters(Arc::clone(&shared), shutdown.clone()));
    handle.spawn(logs::forward(Arc::clone(&shared), shutdown.clone()));
    let thread = {
        let shutdown = shutdown.clone();
        std::thread::Builder::new()
            .name("must-monitor".into())
            .spawn(move || {
                runtime.block_on(shutdown.cancelled());
                // dropping the runtime ends all endpoint tasks and closes connections
                drop(runtime);
            })?
    };
    Ok(Monitor {
        shared,
        handle,
        shutdown,
        thread: Some(thread),
        _registration: registration,
    })
}

static OPEN_FAILED: AtomicBool = AtomicBool::new(false);

/// Runs `f` on the open monitor, opening it first if needed. Returns `None` when
/// monitoring is off or the endpoint cannot be opened.
fn with_monitor<R>(f: impl FnOnce(&Monitor) -> R) -> Option<R> {
    if !is_enabled() {
        return None;
    }
    let mut guard = monitor();
    if guard.is_none() {
        if OPEN_FAILED.load(Ordering::Relaxed) {
            return None;
        }
        match open() {
            Ok(m) => *guard = Some(m),
            Err(e) => {
                OPEN_FAILED.store(true, Ordering::Relaxed);
                drop(guard);
                tracing::warn!(error = %e, "monitoring is disabled for this process");
                return None;
            }
        }
    }
    guard.as_ref().map(f)
}

/// Removes a node; closes the endpoint when it was the last one.
fn remove(id: NodeId) {
    let closing = {
        let mut guard = monitor();
        let Some(m) = guard.as_ref() else { return };
        let empty = {
            let mut inner = m.shared.lock();
            if let Some(entry) = inner.nodes.shift_remove(&id) {
                if let Some(follower) = entry.follower {
                    follower.abort();
                }
                inner.event(Event::NodeRemoved { node: id });
            }
            inner.nodes.is_empty()
        };
        if empty { guard.take() } else { None }
    };
    if let Some(mut m) = closing {
        m.shutdown.cancel();
        if let Some(thread) = m.thread.take()
            && thread.thread().id() != std::thread::current().id()
        {
            let _ = thread.join();
        }
        // the registration file is removed when `m` is dropped
    }
}

/// Adds a node entry and announces it.
fn add(m: &Monitor, entry: NodeEntry) -> NodeId {
    let mut inner = m.shared.lock();
    inner.next_node += 1;
    let id = inner.next_node;
    let info = node_info(id, &entry);
    inner.nodes.insert(id, entry);
    inner.event(Event::NodeCreated { node: info });
    id
}

fn new_entry(equipment_id: &str, context: Option<&str>, state: NodeState) -> NodeEntry {
    NodeEntry {
        equipment_id: equipment_id.to_owned(),
        context: context.map(str::to_owned),
        state,
        messages: Vec::new(),
        running: None,
        address: None,
        clients: IndexMap::new(),
        rx: 0,
        tx: 0,
        last_rx: 0,
        last_tx: 0,
        follower: None,
    }
}

/// Starts following a running node's events and marks it running.
fn attach(m: &Monitor, id: NodeId, node: &RunningNode, messages: Vec<String>) {
    // subscribe before anything else can happen, so no event is missed
    let events = node.subscribe();
    let follower = m
        .handle
        .spawn(follow::follow(Arc::clone(&m.shared), id, events));
    let mut inner = m.shared.lock();
    let Some(entry) = inner.nodes.get_mut(&id) else {
        follower.abort();
        return;
    };
    let address = node.local_addr().to_string();
    entry.state = NodeState::Running;
    entry.messages.clone_from(&messages);
    entry.running = Some(node.clone());
    entry.address = Some(address.clone());
    entry.follower = Some(follower.abort_handle());
    let structure = structure(node);
    inner.event(Event::NodeStarted {
        node: id,
        address,
        structure,
        messages,
    });
}

/// Registers a node started through the Rust API; returns its monitor id.
pub(crate) fn register_running(node: &RunningNode) -> Option<NodeId> {
    with_monitor(|m| {
        let equipment_id = node.hub().model.equipment_id.clone();
        let id = add(m, new_entry(&equipment_id, None, NodeState::Building));
        attach(m, id, node, Vec::new());
        id
    })
}

/// Marks a Rust node stopped and removes it.
pub(crate) fn unregister_running(id: NodeId) {
    {
        let guard = monitor();
        if let Some(m) = guard.as_ref() {
            let mut inner = m.shared.lock();
            if let Some(entry) = inner.nodes.get_mut(&id) {
                entry.state = NodeState::Stopped;
                inner.event(Event::NodeStopped { node: id });
            }
        }
    }
    remove(id);
}

/// A node that the monitor lists before it runs, from the moment it is created (the
/// SHALL C ABI's `SECoP_S_createNode`). Dropping it removes the node from the monitor.
#[derive(Debug)]
pub struct ExternalNode {
    id: NodeId,
}

impl ExternalNode {
    /// Lists a node being built; `None` when monitoring is off or unavailable.
    pub fn new(equipment_id: &str, context: Option<&str>) -> Option<Self> {
        with_monitor(|m| {
            let id = add(m, new_entry(equipment_id, context, NodeState::Building));
            Self { id }
        })
    }

    /// The node's monitor id.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// Building or starting the node failed with these errors and warnings.
    pub fn failed(&self, messages: Vec<String>) {
        let guard = monitor();
        let Some(m) = guard.as_ref() else { return };
        let mut inner = m.shared.lock();
        if let Some(entry) = inner.nodes.get_mut(&self.id) {
            entry.state = NodeState::Failed;
            entry.messages.clone_from(&messages);
            inner.event(Event::NodeFailed {
                node: self.id,
                messages,
            });
        }
    }

    /// The node runs; `warnings` are the remarks of building it.
    pub fn running(&self, node: &RunningNode, warnings: Vec<String>) {
        let guard = monitor();
        if let Some(m) = guard.as_ref() {
            attach(m, self.id, node, warnings);
        }
    }
}

impl Drop for ExternalNode {
    fn drop(&mut self) {
        remove(self.id);
    }
}
