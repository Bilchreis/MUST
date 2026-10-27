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

use crate::protocol::{
    ClientMessage, NodeId, ProcessInfo, ServerMessage, Snapshot, UpdateMode, protocol_version,
    to_line,
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
}

impl Options {
    /// Options with the default update interval.
    pub fn new(client: &str) -> Self {
        Self {
            client: client.to_owned(),
            update_interval: None,
        }
    }
}

/// A connection to an endpoint, after the welcome and the snapshot.
pub struct Connection {
    process: ProcessInfo,
    snapshot: Snapshot,
    reader: BufReader<ReadHalf<Stream>>,
    writer: WriteHalf<Stream>,
    line: String,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("process", &self.process)
            .finish_non_exhaustive()
    }
}

/// Connects to the endpoint of a registration and reads its welcome and snapshot.
pub async fn connect(
    registration: &Registration,
    options: &Options,
) -> Result<Connection, ClientError> {
    let stream = transport::connect(&registration.endpoint, registration.pid).await?;
    connect_stream(stream, options).await
}

/// Like [`connect`], on an already open stream.
pub async fn connect_stream(stream: Stream, options: &Options) -> Result<Connection, ClientError> {
    let (reader, mut writer) = tokio::io::split(stream);
    let hello = ClientMessage::Hello {
        version: protocol_version(),
        client: options.client.clone(),
        update_interval: options.update_interval,
    };
    writer.write_all(to_line(&hello).as_bytes()).await?;
    let mut connection = Connection {
        process: ProcessInfo::default(),
        snapshot: Snapshot::default(),
        reader: BufReader::new(reader),
        writer,
        line: String::new(),
    };
    match connection.read().await? {
        Some(ServerMessage::Welcome { process, .. }) => connection.process = process,
        Some(ServerMessage::Error { message }) => return Err(ClientError::Refused(message)),
        other => {
            return Err(ClientError::Protocol(format!(
                "{other:?} instead of welcome"
            )));
        }
    }
    match connection.read().await? {
        Some(ServerMessage::Snapshot(snapshot)) => connection.snapshot = snapshot,
        other => {
            return Err(ClientError::Protocol(format!(
                "{other:?} instead of snapshot"
            )));
        }
    }
    Ok(connection)
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
    pub async fn next(&mut self) -> Result<Option<ServerMessage>, ClientError> {
        self.read().await
    }

    /// Turns message events of a node on or off.
    pub async fn subscribe_traffic(
        &mut self,
        node: NodeId,
        enabled: bool,
    ) -> Result<(), ClientError> {
        self.send(&ClientMessage::SubscribeTraffic { node, enabled })
            .await
    }

    /// Chooses coalesced or every update for a node.
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

    async fn read(&mut self) -> Result<Option<ServerMessage>, ClientError> {
        self.line.clear();
        if self.reader.read_line(&mut self.line).await? == 0 {
            return Ok(None);
        }
        serde_json::from_str(&self.line)
            .map(Some)
            .map_err(|e| ClientError::Protocol(format!("{e}: {}", self.line.trim_end())))
    }
}
