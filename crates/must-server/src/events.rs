// SPDX-License-Identifier: LGPL-3.0-or-later

//! Events of a running node, for monitoring.

use std::net::SocketAddr;
use std::sync::Arc;

/// Identifies a client connection within a node.
pub type ConnectionId = u64;

/// Something that happened in a running node. Subscribe with
/// [`RunningNode::subscribe`](crate::RunningNode::subscribe).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum NodeEvent {
    /// The node listens on `addr`.
    Started {
        /// The bound address.
        addr: SocketAddr,
    },
    /// The node was stopped.
    Stopped,
    /// A client connected.
    ClientConnected {
        /// The connection.
        id: ConnectionId,
        /// The client's address.
        peer: SocketAddr,
    },
    /// A client connection ended.
    ClientDisconnected {
        /// The connection.
        id: ConnectionId,
        /// Why it ended, e.g. "closed by client" or "too much unread data".
        reason: String,
    },
    /// A line was received from a client (without line ending).
    MessageReceived {
        /// The connection.
        id: ConnectionId,
        /// The message.
        line: Arc<str>,
    },
    /// A line was sent to a client (without line ending). Updates are sent as
    /// `MessageSent` to each activated client, besides one [`Updated`](Self::Updated).
    MessageSent {
        /// The connection.
        id: ConnectionId,
        /// The message.
        line: Arc<str>,
    },
    /// A parameter got a new value or error; sent once, however many clients are
    /// activated.
    Updated {
        /// The module.
        module: String,
        /// The parameter.
        parameter: String,
        /// The `update` or `error_update` message for it (without line ending), as
        /// activated clients receive it.
        line: Arc<str>,
    },
    /// A client activated updates of one module or, with `None`, of the whole node.
    ClientActivated {
        /// The connection.
        id: ConnectionId,
        /// The module, `None` for all.
        module: Option<String>,
    },
    /// A client deactivated updates of one module or, with `None`, of the whole node.
    ClientDeactivated {
        /// The connection.
        id: ConnectionId,
        /// The module, `None` for all.
        module: Option<String>,
    },
    /// The node logged something about itself (also sent to `tracing`).
    Log {
        /// Severity.
        level: LogLevel,
        /// The module concerned, if any.
        module: Option<String>,
        /// The message.
        message: String,
    },
}

/// Severity of a [`NodeEvent::Log`] record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    /// Something failed.
    Error,
    /// Something is wrong but the node goes on.
    Warn,
    /// Normal operation worth noting.
    Info,
}

impl LogLevel {
    /// `ERROR`, `WARN` or `INFO`.
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
        }
    }
}
