// SPDX-License-Identifier: LGPL-3.0-or-later

//! The TUI's state and how messages change it. Rendering ([`crate::ui`]) only reads it.

use std::collections::VecDeque;

use indexmap::IndexMap;
use must_monitor::protocol::{
    ClientId, Direction, Event, LogRecord, NodeId, NodeInfo, NodeState, ParameterState,
    ProcessInfo, ServerMessage, Snapshot,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Identifies a monitored process: pid and start time (pids can be reused).
pub type ProcessKey = (u32, u64);

/// Gone processes are removed after this many seconds, unless pinned.
pub const GONE_REMOVAL: f64 = 60.0;
/// Messages kept per node while its traffic view is open.
pub const TRAFFIC_CAPACITY: usize = 10_000;
/// Log records kept of all processes together.
pub const LOG_CAPACITY: usize = 10_000;

/// Something that changes the state.
// endpoint messages are large, but each is moved through the channel only once
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// A key was pressed.
    Key(KeyEvent),
    /// The terminal was resized; the next draw adapts.
    Resize,
    /// Time passes: `now` is UNIX time in seconds.
    Tick {
        /// The current time.
        now: f64,
    },
    /// News from the connection to a process's endpoint.
    Endpoint {
        /// The process.
        key: ProcessKey,
        /// What happened.
        msg: EndpointMsg,
    },
}

/// News from a connection.
#[derive(Debug, Clone, PartialEq)]
pub enum EndpointMsg {
    /// A registration was found; connecting.
    Connecting {
        /// Executable name from the registration.
        name: String,
    },
    /// Connected; the snapshot follows the welcome.
    Connected {
        /// The process.
        process: ProcessInfo,
        /// Everything known at connection time.
        snapshot: Snapshot,
    },
    /// A message after the snapshot.
    Message(ServerMessage),
    /// Connecting failed; retried at the next rescan.
    Failed(String),
    /// The endpoint refused the connection, e.g. an unsupported protocol version; not
    /// retried.
    Refused(String),
    /// The connection ended or the process is gone.
    Closed,
}

/// Something the main loop does for the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Turn message events of a node on or off.
    Traffic {
        /// The process.
        key: ProcessKey,
        /// The node.
        node: NodeId,
        /// On or off.
        enabled: bool,
    },
}

/// State of the connection to a process.
#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    /// Not connected yet.
    Connecting,
    /// Connected and receiving events.
    Connected,
    /// Connecting failed; the reason.
    Failed(String),
    /// Refused by the endpoint; the reason.
    Refused(String),
}

/// A monitored process.
#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    /// Process id.
    pub pid: u32,
    /// Executable name.
    pub name: String,
    /// Version of its must library, once connected.
    pub library_version: String,
    /// The connection.
    pub link: Link,
    /// When the process was seen gone (UNIX time), if it is.
    pub gone_since: Option<f64>,
    /// Kept after it is gone.
    pub pinned: bool,
    /// Events the endpoint dropped for us, in total.
    pub lagged: u64,
    /// Its nodes, in creation order.
    pub nodes: IndexMap<NodeId, Node>,
}

/// A node of a process.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// What the endpoint reported (its `parameters` are kept in [`Node::parameters`]).
    pub info: NodeInfo,
    /// Last value or error per `(module, parameter)`.
    pub parameters: IndexMap<(String, String), ParameterState>,
    /// Messages received per second.
    pub rx_per_second: f64,
    /// Messages sent per second.
    pub tx_per_second: f64,
    /// Messages while the traffic view of this node is open.
    pub traffic: VecDeque<TrafficLine>,
}

/// A message on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct TrafficLine {
    /// When, as UNIX time.
    pub time: f64,
    /// The client.
    pub client: ClientId,
    /// Direction.
    pub direction: Direction,
    /// The text.
    pub text: String,
    /// Whether the endpoint shortened it.
    pub truncated: bool,
}

/// A log record and the process it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct LogLine {
    /// The process.
    pub key: ProcessKey,
    /// The record.
    pub record: LogRecord,
}

/// The views, switched with Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// All nodes of all processes.
    Overview,
    /// The selected node.
    Detail,
    /// The selected node's messages.
    Traffic,
    /// The log records of all processes.
    Logs,
}

/// The whole state.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    /// The processes, in discovery order.
    pub processes: IndexMap<ProcessKey, Process>,
    /// Log records of all processes, oldest first.
    pub logs: VecDeque<LogLine>,
    /// The current view.
    pub view: View,
    /// Whether the help overlay is shown.
    pub help: bool,
    /// The selected node.
    pub selected: Option<(ProcessKey, NodeId)>,
    /// Select this process's first node once it appears (`--focus-pid`).
    pub focus_pid: Option<u32>,
    /// The current time (UNIX seconds), from the last tick.
    pub now: f64,
    /// Whether the user asked to quit.
    pub quit: bool,
    /// The node whose traffic is subscribed.
    traffic_node: Option<(ProcessKey, NodeId)>,
}

impl App {
    /// An empty state at time `now`.
    pub fn new(now: f64, focus_pid: Option<u32>) -> Self {
        Self {
            processes: IndexMap::new(),
            logs: VecDeque::new(),
            view: View::Overview,
            help: false,
            selected: None,
            focus_pid,
            now,
            quit: false,
            traffic_node: None,
        }
    }

    /// Applies a message; returns what the main loop must do.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Key(key) => self.key(key),
            Msg::Resize => {}
            Msg::Tick { now } => self.tick(now),
            Msg::Endpoint { key, msg } => self.endpoint(key, msg),
        }
        self.repair_selection();
        self.traffic_effects()
    }

    /// All nodes in display order.
    pub fn node_keys(&self) -> Vec<(ProcessKey, NodeId)> {
        self.processes
            .iter()
            .flat_map(|(key, p)| p.nodes.keys().map(move |id| (*key, *id)))
            .collect()
    }

    /// The selected node.
    pub fn selected_node(&self) -> Option<&Node> {
        let (key, id) = self.selected?;
        self.processes.get(&key)?.nodes.get(&id)
    }

    fn tick(&mut self, now: f64) {
        self.now = now;
        self.processes
            .retain(|_, p| p.pinned || p.gone_since.is_none_or(|since| now - since < GONE_REMOVAL));
    }

    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Down => self.move_selection(1),
            KeyCode::Up => self.move_selection(-1),
            _ => {}
        }
    }

    fn move_selection(&mut self, step: isize) {
        let keys = self.node_keys();
        if keys.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|s| keys.iter().position(|k| *k == s))
            .unwrap_or(0);
        let next = current.saturating_add_signed(step).min(keys.len() - 1);
        self.selected = Some(keys[next]);
    }

    /// Keeps the selection on an existing node; applies `--focus-pid`.
    fn repair_selection(&mut self) {
        if let Some(pid) = self.focus_pid
            && let Some((key, process)) = self.processes.iter().find(|(_, p)| p.pid == pid)
            && let Some(id) = process.nodes.keys().next()
        {
            self.selected = Some((*key, *id));
            self.focus_pid = None;
            return;
        }
        let exists = self.selected.is_some_and(|(key, id)| {
            self.processes
                .get(&key)
                .is_some_and(|p| p.nodes.contains_key(&id))
        });
        if !exists {
            self.selected = self.node_keys().first().copied();
        }
    }

    /// Subscribes the traffic of the selected node while the traffic view is open.
    fn traffic_effects(&mut self) -> Vec<Effect> {
        let wanted = match self.view {
            View::Traffic => self.selected.filter(|(key, _)| {
                self.processes
                    .get(key)
                    .is_some_and(|p| p.link == Link::Connected && p.gone_since.is_none())
            }),
            _ => None,
        };
        if wanted == self.traffic_node {
            return Vec::new();
        }
        let mut effects = Vec::new();
        if let Some((key, node)) = self.traffic_node.take() {
            if let Some(n) = self
                .processes
                .get_mut(&key)
                .and_then(|p| p.nodes.get_mut(&node))
            {
                n.traffic.clear();
            }
            effects.push(Effect::Traffic {
                key,
                node,
                enabled: false,
            });
        }
        if let Some((key, node)) = wanted {
            effects.push(Effect::Traffic {
                key,
                node,
                enabled: true,
            });
        }
        self.traffic_node = wanted;
        effects
    }

    fn endpoint(&mut self, key: ProcessKey, msg: EndpointMsg) {
        let now = self.now;
        match msg {
            EndpointMsg::Connecting { name } => {
                let process = self.processes.entry(key).or_insert_with(|| Process {
                    pid: key.0,
                    name: name.clone(),
                    library_version: String::new(),
                    link: Link::Connecting,
                    gone_since: None,
                    pinned: false,
                    lagged: 0,
                    nodes: IndexMap::new(),
                });
                if process.link != Link::Connected {
                    process.link = Link::Connecting;
                }
                process.gone_since = None;
            }
            EndpointMsg::Connected { process, snapshot } => {
                let entry = self.processes.entry(key).or_insert_with(|| Process {
                    pid: key.0,
                    name: String::new(),
                    library_version: String::new(),
                    link: Link::Connecting,
                    gone_since: None,
                    pinned: false,
                    lagged: 0,
                    nodes: IndexMap::new(),
                });
                entry.name = process.name;
                entry.library_version = process.library_version;
                entry.link = Link::Connected;
                entry.gone_since = None;
                entry.nodes = snapshot
                    .nodes
                    .into_iter()
                    .map(|info| (info.id, Node::new(info)))
                    .collect();
                // the snapshot's log records, merged with the others by time
                let mut merged: Vec<LogLine> = self.logs.drain(..).collect();
                merged.extend(
                    snapshot
                        .logs
                        .into_iter()
                        .map(|record| LogLine { key, record }),
                );
                merged.sort_by(|a, b| a.record.time.total_cmp(&b.record.time));
                let skip = merged.len().saturating_sub(LOG_CAPACITY);
                self.logs = merged.into_iter().skip(skip).collect();
                // a reconnect loses any traffic subscription
                if self.traffic_node.is_some_and(|(k, _)| k == key) {
                    self.traffic_node = None;
                }
            }
            EndpointMsg::Message(message) => {
                if let Some(process) = self.processes.get_mut(&key) {
                    match message {
                        ServerMessage::Event(Event::Log(record)) => {
                            push_bounded(&mut self.logs, LogLine { key, record }, LOG_CAPACITY);
                        }
                        other => process.apply(other),
                    }
                }
            }
            EndpointMsg::Failed(reason) => {
                if let Some(p) = self.processes.get_mut(&key)
                    && p.link != Link::Connected
                {
                    p.link = Link::Failed(reason);
                }
            }
            EndpointMsg::Refused(reason) => {
                if let Some(p) = self.processes.get_mut(&key) {
                    p.link = Link::Refused(reason);
                }
            }
            EndpointMsg::Closed => {
                if let Some(p) = self.processes.get_mut(&key) {
                    if p.gone_since.is_none() {
                        p.gone_since = Some(now);
                    }
                    if self.traffic_node.is_some_and(|(k, _)| k == key) {
                        self.traffic_node = None;
                    }
                }
            }
        }
    }
}

impl Node {
    fn new(mut info: NodeInfo) -> Self {
        let parameters = std::mem::take(&mut info.parameters)
            .into_iter()
            .map(|p| ((p.module.clone(), p.parameter.clone()), p))
            .collect();
        Self {
            info,
            parameters,
            rx_per_second: 0.0,
            tx_per_second: 0.0,
            traffic: VecDeque::new(),
        }
    }
}

impl Process {
    /// Applies an event or other message of this process's endpoint.
    fn apply(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Event(event) => self.event(event),
            ServerMessage::Counters {
                node,
                rx_per_second,
                tx_per_second,
                ..
            } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.rx_per_second = rx_per_second;
                    n.tx_per_second = tx_per_second;
                }
            }
            ServerMessage::Lagged { dropped } => self.lagged += dropped,
            ServerMessage::Welcome { .. }
            | ServerMessage::Snapshot(_)
            | ServerMessage::Error { .. }
            | ServerMessage::Unknown => {}
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::NodeCreated { node } => {
                self.nodes.insert(node.id, Node::new(node));
            }
            Event::NodeStarted {
                node,
                address,
                structure,
                messages,
            } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.info.state = NodeState::Running;
                    n.info.address = Some(address);
                    n.info.structure = Some(structure);
                    n.info.messages = messages;
                }
            }
            Event::NodeStopped { node } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.info.state = NodeState::Stopped;
                    n.info.clients.clear();
                    n.rx_per_second = 0.0;
                    n.tx_per_second = 0.0;
                }
            }
            Event::NodeFailed { node, messages } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.info.state = NodeState::Failed;
                    n.info.messages = messages;
                }
            }
            Event::NodeRemoved { node } => {
                self.nodes.shift_remove(&node);
            }
            Event::ClientConnected { node, client } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.info.clients.push(client);
                }
            }
            Event::ClientDisconnected { node, client, .. } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.info.clients.retain(|c| c.id != client);
                }
            }
            Event::ClientActivated {
                node,
                client,
                module,
            } => self.activation(node, client, module, true),
            Event::ClientDeactivated {
                node,
                client,
                module,
            } => self.activation(node, client, module, false),
            Event::Message {
                node,
                client,
                direction,
                time,
                text,
                truncated,
            } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    let line = TrafficLine {
                        time,
                        client,
                        direction,
                        text,
                        truncated,
                    };
                    push_bounded(&mut n.traffic, line, TRAFFIC_CAPACITY);
                }
            }
            Event::Update { node, state } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.parameters
                        .insert((state.module.clone(), state.parameter.clone()), state);
                }
            }
            Event::LogFile { node, path } => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.info.log_file = path;
                }
            }
            // handled by the app
            Event::Log(_) | Event::Unknown => {}
        }
    }

    /// A client activated (`on`) or deactivated a module, or all with `module` `None`.
    fn activation(&mut self, node: NodeId, client: ClientId, module: Option<String>, on: bool) {
        let Some(c) = self.client(node, client) else {
            return;
        };
        match (module, on) {
            (None, true) => c.active_all = true,
            (None, false) => {
                c.active_all = false;
                c.active_modules.clear();
            }
            (Some(m), true) => {
                if !c.active_modules.contains(&m) {
                    c.active_modules.push(m);
                }
            }
            (Some(m), false) => c.active_modules.retain(|x| *x != m),
        }
    }

    fn client(
        &mut self,
        node: NodeId,
        client: ClientId,
    ) -> Option<&mut must_monitor::protocol::ClientInfo> {
        self.nodes
            .get_mut(&node)?
            .info
            .clients
            .iter_mut()
            .find(|c| c.id == client)
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, item: T, capacity: usize) {
    if queue.len() == capacity {
        queue.pop_front();
    }
    queue.push_back(item);
}
