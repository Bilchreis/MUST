// SPDX-License-Identifier: LGPL-3.0-or-later

//! Discovering and reading monitoring endpoints.
//!
//! ```no_run
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use must_monitor::client::{self, Options};
//! use must_monitor::registry::EndpointStatus;
//!
//! for listed in client::list_endpoints()? {
//!     if listed.status != EndpointStatus::Live {
//!         continue;
//!     }
//!     let mut connection = client::connect(&listed.registration, &Options::new("example")).await?;
//!     for node in &connection.snapshot().nodes {
//!         println!("{} {:?}", node.equipment_id, node.state);
//!     }
//!     while let Some(message) = connection.next().await? {
//!         println!("{message:?}");
//!     }
//! }
//! # Ok(())
//! # }
//! ```

use std::io;
use std::path::Path;

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};

/// The default for [`Options::max_line_length`]: 64 MiB, room for the snapshot of a
/// process with many large nodes.
pub const DEFAULT_MAX_LINE_LENGTH: usize = 64 << 20;

use crate::protocol::{
    ClientMessage, Event, NodeId, ProcessInfo, ServerMessage, Snapshot, UpdateMode,
    protocol_version, to_line,
};
use crate::registry::{self, Listed, Registration, RegistryError};
use crate::transport::{self, Stream};

/// Reading an endpoint failed.
#[derive(Debug, Error)]
pub enum ClientError {
    /// The connection failed or broke.
    #[error("connection: {0}")]
    Io(#[from] io::Error),
    /// The endpoint sent something that does not fit the protocol.
    #[error("unexpected message from the endpoint: {0}")]
    Protocol(String),
    /// The endpoint refused the connection (e.g. version mismatch).
    #[error("refused by the endpoint: {0}")]
    Refused(String),
}

/// Lists the registrations of the user's processes (see [`registry::list`]) in the
/// default registry directory.
pub fn list_endpoints() -> Result<Vec<Listed>, RegistryError> {
    list_endpoints_in(&registry::registry_dir()?)
}

/// Lists the registrations in `dir`.
pub fn list_endpoints_in(dir: &Path) -> Result<Vec<Listed>, RegistryError> {
    registry::list(dir)
}

/// How to introduce oneself to an endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Name of the monitoring program.
    pub client: String,
    /// Interval of coalesced parameter updates in seconds; `None` for the default 1 s.
    pub update_interval: Option<f64>,
    /// The longest message line accepted from the endpoint, in bytes. A longer one is a
    /// [`ClientError::Protocol`] and ends the connection, instead of growing a buffer
    /// without limit.
    pub max_line_length: usize,
}

impl Options {
    /// Options with the default update interval.
    pub fn new(client: &str) -> Self {
        Self {
            client: client.to_owned(),
            update_interval: None,
            max_line_length: DEFAULT_MAX_LINE_LENGTH,
        }
    }
}

/// A connection to an endpoint, after the welcome and the snapshot.
pub struct Connection {
    process: ProcessInfo,
    snapshot: Snapshot,
    events: Events,
    requests: Requests,
}

/// The reading half of a [`Connection`] (see [`Connection::into_parts`]).
///
/// Reading is cancellation safe: a [`next`](Self::next) dropped before it completes, for
/// example as the losing branch of `tokio::select!`, keeps the part of a line it read,
/// and the next call continues with it.
pub struct Events {
    reader: BufReader<ReadHalf<Stream>>,
    /// The line read so far; cleared only once it is complete.
    line: Vec<u8>,
    max_line_length: usize,
    /// Set once a line was too long: the rest of the stream cannot be framed.
    overlong: bool,
}

/// The writing half of a [`Connection`] (see [`Connection::into_parts`]).
pub struct Requests {
    writer: WriteHalf<Stream>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("process", &self.process)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Events {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Events").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Requests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Requests").finish_non_exhaustive()
    }
}

/// Connects to the endpoint of a registration and reads its welcome and snapshot.
///
/// This waits as long as the endpoint takes to answer; wrap it in
/// [`tokio::time::timeout`] to bound it.
pub async fn connect(
    registration: &Registration,
    options: &Options,
) -> Result<Connection, ClientError> {
    let stream = transport::connect(&registration.endpoint, registration.pid).await?;
    connect_stream(stream, options).await
}

/// Like [`connect`], on an already open stream.
pub async fn connect_stream(stream: Stream, options: &Options) -> Result<Connection, ClientError> {
    let (reader, writer) = tokio::io::split(stream);
    let mut requests = Requests { writer };
    let mut events = Events {
        reader: BufReader::new(reader),
        line: Vec::new(),
        max_line_length: options.max_line_length,
        overlong: false,
    };
    requests
        .send(&ClientMessage::Hello {
            version: protocol_version(),
            client: options.client.clone(),
            update_interval: options.update_interval,
        })
        .await?;
    let process = match events.read().await? {
        Some(ServerMessage::Welcome { process, .. }) => process,
        Some(ServerMessage::Error { message }) => return Err(ClientError::Refused(message)),
        other => {
            return Err(ClientError::Protocol(format!(
                "{other:?} instead of welcome"
            )));
        }
    };
    let snapshot = match events.read().await? {
        Some(ServerMessage::Snapshot(snapshot)) => snapshot,
        other => {
            return Err(ClientError::Protocol(format!(
                "{other:?} instead of snapshot"
            )));
        }
    };
    Ok(Connection {
        process,
        snapshot,
        events,
        requests,
    })
}

impl Connection {
    /// The monitored process.
    pub fn process(&self) -> &ProcessInfo {
        &self.process
    }

    /// The snapshot received on connect.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The next message (events, counters, lagged notices); `None` when the endpoint
    /// closed the connection, e.g. because its process stopped its last node.
    ///
    /// Messages of a newer protocol version that this library does not know are skipped.
    ///
    /// Cancellation safe: see [`Events`].
    pub async fn next(&mut self) -> Result<Option<ServerMessage>, ClientError> {
        self.events.next().await
    }

    /// Turns message events of a node on or off.
    pub async fn subscribe_traffic(
        &mut self,
        node: NodeId,
        enabled: bool,
    ) -> Result<(), ClientError> {
        self.requests.subscribe_traffic(node, enabled).await
    }

    /// Chooses coalesced or every update for a node.
    pub async fn subscribe_updates(
        &mut self,
        node: NodeId,
        mode: UpdateMode,
    ) -> Result<(), ClientError> {
        self.requests.subscribe_updates(node, mode).await
    }

    /// Splits the connection, to read events and send requests from different tasks:
    /// the process, the snapshot, the reading and the writing half.
    pub fn into_parts(self) -> (ProcessInfo, Snapshot, Events, Requests) {
        (self.process, self.snapshot, self.events, self.requests)
    }
}

impl Events {
    /// See [`Connection::next`].
    pub async fn next(&mut self) -> Result<Option<ServerMessage>, ClientError> {
        loop {
            match self.read().await? {
                Some(ServerMessage::Unknown | ServerMessage::Event(Event::Unknown)) => {}
                other => return Ok(other),
            }
        }
    }

    /// Reads one line and parses it. Only `fill_buf` is awaited, and what it returned is
    /// moved into `self.line` before the next await, so cancelling loses nothing.
    async fn read(&mut self) -> Result<Option<ServerMessage>, ClientError> {
        if self.overlong {
            return Err(self.overlong_error());
        }
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if self.line.is_empty() {
                    return Ok(None);
                }
                // the endpoint closed in the middle of a line: parse what came
                break;
            }
            let (used, complete) = match available.iter().position(|&b| b == b'\n') {
                Some(end) => (end + 1, true),
                None => (available.len(), false),
            };
            if self.line.len() + used > self.max_line_length {
                self.overlong = true;
                self.line = Vec::new();
                return Err(self.overlong_error());
            }
            self.line.extend_from_slice(&available[..used]);
            self.reader.consume(used);
            if complete {
                break;
            }
        }
        let parsed = serde_json::from_slice(&self.line).map(Some).map_err(|e| {
            let line = String::from_utf8_lossy(&self.line);
            ClientError::Protocol(format!("{e}: {}", shorten(line.trim_end())))
        });
        self.line.clear();
        parsed
    }

    fn overlong_error(&self) -> ClientError {
        ClientError::Protocol(format!(
            "a message line is longer than {} bytes",
            self.max_line_length
        ))
    }
}

/// The start of `text`, for an error message.
fn shorten(text: &str) -> std::borrow::Cow<'_, str> {
    const MAX: usize = 200;
    match text.char_indices().nth(MAX) {
        Some((end, _)) => format!("{}…", &text[..end]).into(),
        None => text.into(),
    }
}

impl Requests {
    /// See [`Connection::subscribe_traffic`].
    pub async fn subscribe_traffic(
        &mut self,
        node: NodeId,
        enabled: bool,
    ) -> Result<(), ClientError> {
        self.send(&ClientMessage::SubscribeTraffic { node, enabled })
            .await
    }

    /// See [`Connection::subscribe_updates`].
    pub async fn subscribe_updates(
        &mut self,
        node: NodeId,
        mode: UpdateMode,
    ) -> Result<(), ClientError> {
        self.send(&ClientMessage::SubscribeUpdates { node, mode })
            .await
    }

    async fn send(&mut self, message: &ClientMessage) -> Result<(), ClientError> {
        self.writer.write_all(to_line(message).as_bytes()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncWriteExt, DuplexStream, duplex};

    use super::*;

    fn events(max_line_length: usize) -> (Events, DuplexStream) {
        let (ours, theirs) = duplex(1024);
        let (reader, _writer) = tokio::io::split(Box::new(ours) as Stream);
        let events = Events {
            reader: BufReader::new(reader),
            line: Vec::new(),
            max_line_length,
            overlong: false,
        };
        (events, theirs)
    }

    fn lagged_line(dropped: u64) -> String {
        to_line(&ServerMessage::Lagged { dropped })
    }

    #[tokio::test]
    async fn a_cancelled_read_keeps_the_partial_line() {
        let (mut events, mut endpoint) = events(DEFAULT_MAX_LINE_LENGTH);
        let line = lagged_line(7);
        let (first, rest) = line.split_at(line.len() / 2);
        endpoint.write_all(first.as_bytes()).await.unwrap();

        // the read is cancelled after it took in the first half of the line
        let cancelled = tokio::time::timeout(Duration::from_millis(50), events.next()).await;
        assert!(cancelled.is_err());

        endpoint.write_all(rest.as_bytes()).await.unwrap();
        let message = events.next().await.unwrap();
        assert_eq!(message, Some(ServerMessage::Lagged { dropped: 7 }));
    }

    #[tokio::test]
    async fn an_overlong_line_is_an_error_and_ends_reading() {
        let line = lagged_line(1);
        let (mut events, mut endpoint) = events(line.len() - 1);
        endpoint.write_all(line.as_bytes()).await.unwrap();
        endpoint.write_all(line.as_bytes()).await.unwrap();

        let err = events.next().await.unwrap_err();
        assert!(matches!(err, ClientError::Protocol(m) if m.contains("longer than")));
        // the stream cannot be framed any more
        assert!(matches!(events.next().await, Err(ClientError::Protocol(_))));
    }

    #[tokio::test]
    async fn a_line_of_exactly_the_limit_is_read() {
        let line = lagged_line(2);
        let (mut events, mut endpoint) = events(line.len());
        endpoint.write_all(line.as_bytes()).await.unwrap();
        drop(endpoint);
        let message = events.next().await.unwrap();
        assert_eq!(message, Some(ServerMessage::Lagged { dropped: 2 }));
        assert_eq!(events.next().await.unwrap(), None);
    }
}
