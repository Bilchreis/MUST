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
    /// A line was sent to a client (without line ending).
    MessageSent {
        /// The connection.
        id: ConnectionId,
        /// The message.
        line: Arc<str>,
    },
    /// A parameter got a new value or error.
    Updated {
        /// The module.
        module: String,
        /// The parameter.
        parameter: String,
    },
}
