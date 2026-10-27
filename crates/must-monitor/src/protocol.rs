// SPDX-License-Identifier: LGPL-3.0-or-later

//! The monitoring protocol: UTF-8 JSON objects, one per line, each with a `type` field.
//!
//! A client opens with [`ClientMessage::Hello`]; the endpoint answers with
//! [`ServerMessage::Welcome`] and a [`ServerMessage::Snapshot`], then streams
//! [`ServerMessage::Event`]s, per-node [`ServerMessage::Counters`] every second, and
//! [`ServerMessage::Lagged`] when the client fell behind and events were dropped.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Major version of the protocol this crate speaks; a hello with another major version
/// is rejected.
pub const PROTOCOL_MAJOR: u32 = 1;
/// Minor version; minor additions are optional fields that older readers ignore.
pub const PROTOCOL_MINOR: u32 = 0;

/// The protocol version as sent in hello and welcome, e.g. `"1.0"`.
pub fn protocol_version() -> String {
    format!("{PROTOCOL_MAJOR}.{PROTOCOL_MINOR}")
}

/// The major version of a `major.minor` version text.
pub fn major_version(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

/// Identifies a node within one monitored process.
pub type NodeId = u64;
/// Identifies a SECoP client connection within one node.
pub type ClientId = u64;

/// A message from a monitor to the endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Must be the first message of a connection.
    Hello {
        /// Protocol version, `major.minor`.
        version: String,
        /// Name of the monitoring program, for logs.
        client: String,
        /// Interval of coalesced parameter updates in seconds (0.1 to 60, default 1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        update_interval: Option<f64>,
    },
    /// Turns message events of a node on or off (off by default).
    SubscribeTraffic {
        /// The node.
        node: NodeId,
        /// Whether to receive its messages.
        enabled: bool,
    },
    /// Chooses how parameter updates of a node are sent.
    SubscribeUpdates {
        /// The node.
        node: NodeId,
        /// Coalesced (default) or every update.
        mode: UpdateMode,
    },
}

/// How parameter updates of a node reach a monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// At most one update per parameter per interval, with the latest value.
    Coalesced,
    /// One event per update.
    All,
}

/// A message from the endpoint to a monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Answer to a valid hello.
    Welcome {
        /// Protocol version of the endpoint.
        version: String,
        /// The monitored process.
        process: ProcessInfo,
    },
    /// Everything known at connection time; sent once, after the welcome.
    Snapshot(Snapshot),
    /// Something happened.
    Event(Event),
    /// Message counters of a node, sent every second.
    Counters {
        /// The node.
        node: NodeId,
        /// Messages received since the node started.
        rx: u64,
        /// Messages sent since the node started.
        tx: u64,
        /// Messages received per second, over the last second.
        rx_per_second: f64,
        /// Messages sent per second, over the last second.
        tx_per_second: f64,
    },
    /// This many events were dropped because the monitor did not read fast enough.
    Lagged {
        /// Number of dropped events.
        dropped: u64,
    },
    /// The endpoint refuses the connection, e.g. for a version mismatch; it closes the
    /// connection after this.
    Error {
        /// Why.
        message: String,
    },
}

/// The monitored process.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    /// Process id.
    pub pid: u32,
    /// Executable name, e.g. `LabVIEW.exe`.
    pub name: String,
    /// Version of the must library serving the endpoint.
    pub library_version: String,
    /// When the process started, as UNIX time in seconds (0 if unknown).
    pub start_time: f64,
}

/// What a monitor gets on connect.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The process.
    pub process: ProcessInfo,
    /// Its nodes, in creation order.
    pub nodes: Vec<NodeInfo>,
    /// The most recent log records, oldest first.
    pub logs: Vec<LogRecord>,
}

/// State of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// Created through the C ABI, not complete yet.
    Building,
    /// `SECoP_S_nodeComplete` or the start failed; see the node's messages.
    Failed,
    /// Serving clients.
    Running,
    /// Stopped (about to be removed).
    Stopped,
}

/// A node as listed in a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Identifies the node within the process.
    pub id: NodeId,
    /// The node's `equipment_id`.
    pub equipment_id: String,
    /// The SECoP listen address, once running.
    pub address: Option<String>,
    /// Its state.
    pub state: NodeState,
    /// The SHALL context ID, for nodes created through the C ABI.
    pub context: Option<String>,
    /// Errors (when failed) and warnings of building the node.
    pub messages: Vec<String>,
    /// The structure report (`describing` data), once running.
    pub structure: Option<Json>,
    /// The cached state of every parameter that has one, once running.
    pub parameters: Vec<ParameterState>,
    /// Connected SECoP clients.
    pub clients: Vec<ClientInfo>,
}

/// The last known value or error of a parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterState {
    /// Module.
    pub module: String,
    /// Parameter.
    pub parameter: String,
    /// The data report (`[value, {"t": ...}]`) or, if `error`, the error report.
    pub report: Json,
    /// Whether `report` is an error report.
    pub error: bool,
}

/// A SECoP client connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientInfo {
    /// Identifies the connection within the node.
    pub id: ClientId,
    /// The client's address.
    pub peer: String,
    /// When it connected, as UNIX time in seconds.
    pub connected_at: f64,
    /// Whether updates of all modules are activated.
    pub active_all: bool,
    /// Modules activated one by one.
    pub active_modules: Vec<String>,
}

/// Direction of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// From the client to the node.
    Received,
    /// From the node to the client.
    Sent,
}

/// A log record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRecord {
    /// When, as UNIX time in seconds.
    pub time: f64,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: String,
    /// The node it concerns, if known.
    pub node: Option<NodeId>,
    /// The module it concerns, if known.
    pub module: Option<String>,
    /// The text.
    pub message: String,
}

/// Something that happened, tagged with `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A node was added (created through the C ABI, or started).
    NodeCreated {
        /// The node as it is now.
        node: NodeInfo,
    },
    /// A node started serving.
    NodeStarted {
        /// The node.
        node: NodeId,
        /// The SECoP listen address.
        address: String,
        /// The structure report.
        structure: Json,
        /// Warnings of building the node.
        messages: Vec<String>,
    },
    /// A node stopped.
    NodeStopped {
        /// The node.
        node: NodeId,
    },
    /// Building or starting a node failed.
    NodeFailed {
        /// The node.
        node: NodeId,
        /// The errors and warnings.
        messages: Vec<String>,
    },
    /// A node is gone.
    NodeRemoved {
        /// The node.
        node: NodeId,
    },
    /// A SECoP client connected.
    ClientConnected {
        /// The node.
        node: NodeId,
        /// The client.
        client: ClientInfo,
    },
    /// A SECoP client disconnected.
    ClientDisconnected {
        /// The node.
        node: NodeId,
        /// The client.
        client: ClientId,
        /// Why, e.g. "closed by client".
        reason: String,
    },
    /// A client activated updates of a module, or of all with `module` absent.
    ClientActivated {
        /// The node.
        node: NodeId,
        /// The client.
        client: ClientId,
        /// The module, if not all.
        module: Option<String>,
    },
    /// A client deactivated updates of a module, or of all with `module` absent.
    ClientDeactivated {
        /// The node.
        node: NodeId,
        /// The client.
        client: ClientId,
        /// The module, if not all.
        module: Option<String>,
    },
    /// A message on the wire (only with traffic subscribed for the node).
    Message {
        /// The node.
        node: NodeId,
        /// The client.
        client: ClientId,
        /// Direction.
        direction: Direction,
        /// When, as UNIX time in seconds.
        time: f64,
        /// The message, without line ending, cut to 4 KiB if `truncated`.
        text: String,
        /// Whether `text` was cut.
        truncated: bool,
    },
    /// A parameter's value or error (coalesced unless every update was requested).
    Update {
        /// The node.
        node: NodeId,
        /// The parameter's state.
        #[serde(flatten)]
        state: ParameterState,
    },
    /// A log record.
    Log(LogRecord),
}

/// Messages longer than this are cut in message events.
pub const MAX_MESSAGE_TEXT: usize = 4096;

/// Cuts a message text to [`MAX_MESSAGE_TEXT`] bytes at a character boundary; returns
/// whether it was cut.
pub fn truncate_text(text: &str) -> (String, bool) {
    if text.len() <= MAX_MESSAGE_TEXT {
        return (text.to_owned(), false);
    }
    let mut end = MAX_MESSAGE_TEXT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

/// One message as a line (JSON plus `\n`).
pub fn to_line<T: Serialize>(message: &T) -> String {
    let mut line = serde_json::to_string(message).unwrap_or_else(|_| "{}".to_owned());
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn round_trip_server(message: &ServerMessage) {
        let line = to_line(message);
        assert!(line.ends_with('\n') && !line.trim_end().contains('\n'));
        let back: ServerMessage = serde_json::from_str(&line).unwrap();
        assert_eq!(&back, message);
    }

    fn process() -> ProcessInfo {
        ProcessInfo {
            pid: 4242,
            name: "LabVIEW.exe".into(),
            library_version: "0.1.0".into(),
            start_time: 1_793_118_000.5,
        }
    }

    fn node() -> NodeInfo {
        NodeInfo {
            id: 1,
            equipment_id: "cryo".into(),
            address: Some("0.0.0.0:2055".into()),
            state: NodeState::Running,
            context: Some("vi_cryo".into()),
            messages: vec!["cryo:t1: no description given".into()],
            structure: Some(json!({"equipment_id": "cryo", "modules": {}})),
            parameters: vec![ParameterState {
                module: "t1".into(),
                parameter: "value".into(),
                report: json!([4.2, {"t": 1.5}]),
                error: false,
            }],
            clients: vec![ClientInfo {
                id: 3,
                peer: "192.168.1.20:51234".into(),
                connected_at: 2.5,
                active_all: false,
                active_modules: vec!["t1".into()],
            }],
        }
    }

    #[test]
    fn protocol_client_messages() {
        let hello = ClientMessage::Hello {
            version: protocol_version(),
            client: "must-tui".into(),
            update_interval: None,
        };
        let line = to_line(&hello);
        assert_eq!(
            line,
            "{\"type\":\"hello\",\"version\":\"1.0\",\"client\":\"must-tui\"}\n"
        );
        for message in [
            hello,
            ClientMessage::Hello {
                version: "1.3".into(),
                client: "x".into(),
                update_interval: Some(0.5),
            },
            ClientMessage::SubscribeTraffic {
                node: 1,
                enabled: true,
            },
            ClientMessage::SubscribeUpdates {
                node: 1,
                mode: UpdateMode::All,
            },
        ] {
            let back: ClientMessage = serde_json::from_str(&to_line(&message)).unwrap();
            assert_eq!(back, message);
        }
        // unknown fields from newer minor versions are ignored
        let newer: ClientMessage =
            serde_json::from_str(r#"{"type":"hello","version":"1.7","client":"x","future":true}"#)
                .unwrap();
        assert!(matches!(newer, ClientMessage::Hello { .. }));
    }

    #[test]
    fn protocol_server_messages() {
        round_trip_server(&ServerMessage::Welcome {
            version: protocol_version(),
            process: process(),
        });
        round_trip_server(&ServerMessage::Snapshot(Snapshot {
            process: process(),
            nodes: vec![node()],
            logs: vec![LogRecord {
                time: 3.0,
                level: "WARN".into(),
                node: Some(1),
                module: Some("t1".into()),
                message: "reading `value` failed".into(),
            }],
        }));
        round_trip_server(&ServerMessage::Counters {
            node: 1,
            rx: 10,
            tx: 12,
            rx_per_second: 1.0,
            tx_per_second: 2.0,
        });
        round_trip_server(&ServerMessage::Lagged { dropped: 7 });
        round_trip_server(&ServerMessage::Error {
            message: "unsupported version".into(),
        });
    }

    #[test]
    fn protocol_events() {
        let events = [
            Event::NodeCreated { node: node() },
            Event::NodeStarted {
                node: 1,
                address: "0.0.0.0:2055".into(),
                structure: json!({}),
                messages: vec![],
            },
            Event::NodeStopped { node: 1 },
            Event::NodeFailed {
                node: 1,
                messages: vec!["no datainfo".into()],
            },
            Event::NodeRemoved { node: 1 },
            Event::ClientConnected {
                node: 1,
                client: node().clients[0].clone(),
            },
            Event::ClientDisconnected {
                node: 1,
                client: 3,
                reason: "closed by client".into(),
            },
            Event::ClientActivated {
                node: 1,
                client: 3,
                module: None,
            },
            Event::ClientDeactivated {
                node: 1,
                client: 3,
                module: Some("t1".into()),
            },
            Event::Message {
                node: 1,
                client: 3,
                direction: Direction::Received,
                time: 4.0,
                text: "read t1:value".into(),
                truncated: false,
            },
            Event::Update {
                node: 1,
                state: node().parameters[0].clone(),
            },
            Event::Log(LogRecord {
                time: 3.0,
                level: "INFO".into(),
                node: None,
                module: None,
                message: "hello".into(),
            }),
        ];
        for event in events {
            round_trip_server(&ServerMessage::Event(event));
        }
        let line = to_line(&ServerMessage::Event(Event::NodeStopped { node: 1 }));
        assert_eq!(
            line,
            "{\"type\":\"event\",\"kind\":\"node_stopped\",\"node\":1}\n"
        );
    }

    #[test]
    fn protocol_versions_and_truncation() {
        assert_eq!(major_version("1.0"), Some(1));
        assert_eq!(major_version("99.2"), Some(99));
        assert_eq!(major_version("x"), None);
        let long = "é".repeat(3000); // 6000 bytes
        let (text, cut) = truncate_text(&long);
        assert!(cut && text.len() <= MAX_MESSAGE_TEXT && text.len() >= MAX_MESSAGE_TEXT - 1);
        assert_eq!(truncate_text("short"), ("short".to_owned(), false));
    }
}
