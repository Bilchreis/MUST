// SPDX-License-Identifier: LGPL-3.0-or-later

//! Starting and stopping a node.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use must_types::SecopError;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::info;

use super::Hub;
use super::actor::{ActorHandle, Job, spawn_actor};
use super::connection;
use crate::events::{LogLevel, NodeEvent};
use crate::handler::Reading;
use crate::model::Node;

/// A node could not be started.
#[derive(Debug, Error)]
pub enum StartError {
    /// The listening address could not be bound, e.g. because the port is in use.
    #[error("cannot listen on {addr}: {source}")]
    Bind {
        /// The address.
        addr: SocketAddr,
        /// The operating system's error.
        source: io::Error,
    },
}

/// A node serving clients. Dropping the handle does not stop the node; call
/// [`stop`](Self::stop).
#[derive(Clone)]
pub struct RunningNode {
    hub: Arc<Hub>,
    addr: SocketAddr,
    stop: CancellationToken,
    tracker: TaskTracker,
}

/// The runtime used by [`Node::start`], created on first use.
fn shared_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("must-node")
            .build()
            .expect("cannot create the must runtime")
    })
}

impl Node {
    /// Starts serving on a runtime owned by the library, shared by all nodes started
    /// this way. Callable from any thread, also outside of any async runtime.
    pub fn start(self) -> Result<RunningNode, StartError> {
        self.spawn(shared_runtime().handle())
    }

    /// Starts serving on the given tokio runtime.
    pub fn spawn(self, runtime: &Handle) -> Result<RunningNode, StartError> {
        let addr = self.model.options.bind;
        // bind synchronously, so a port in use is reported before anything runs
        let listener = std::net::TcpListener::bind(addr)
            .and_then(|l| l.set_nonblocking(true).map(|()| l))
            .map_err(|source| StartError::Bind { addr, source })?;
        let _guard = runtime.enter();
        let listener =
            TcpListener::from_std(listener).map_err(|source| StartError::Bind { addr, source })?;
        let addr = listener
            .local_addr()
            .map_err(|source| StartError::Bind { addr, source })?;

        let report: Arc<str> = self.structure_report().to_string().into();
        let hub = Arc::new(Hub::new(Arc::clone(&self.model), report));
        let stop = CancellationToken::new();
        let tracker = TaskTracker::new();

        let actors: HashMap<String, ActorHandle> = self
            .model
            .modules
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    spawn_actor(&hub, name, &tracker, stop.clone()),
                )
            })
            .collect();
        let actors = Arc::new(actors);

        start_polling(&hub, &actors, &tracker, &stop);
        tracker.spawn(accept_loop(
            Arc::clone(&hub),
            Arc::clone(&actors),
            listener,
            stop.clone(),
            tracker.clone(),
        ));

        info!(%addr, equipment_id = %self.model.equipment_id, "node started");
        hub.emit(NodeEvent::Started { addr });
        Ok(RunningNode {
            hub,
            addr,
            stop,
            tracker,
        })
    }
}

async fn accept_loop(
    hub: Arc<Hub>,
    actors: Arc<HashMap<String, ActorHandle>>,
    listener: TcpListener,
    stop: CancellationToken,
    tracker: TaskTracker,
) {
    loop {
        let accepted = tokio::select! {
            () = stop.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, peer)) => {
                tracker.spawn(connection::serve(
                    Arc::clone(&hub),
                    Arc::clone(&actors),
                    stream,
                    peer,
                    stop.clone(),
                ));
            }
            Err(e) => hub.log(
                LogLevel::Warn,
                None,
                format!("accepting a connection failed: {e}"),
            ),
        }
    }
    // the listener is dropped here, closing the port
}

/// Starts the startup reads and one poll task per polled parameter.
fn start_polling(
    hub: &Arc<Hub>,
    actors: &Arc<HashMap<String, ActorHandle>>,
    tracker: &TaskTracker,
    stop: &CancellationToken,
) {
    for (mname, module) in &hub.model.modules {
        let actor = &actors[mname];
        for (pname, param) in module.parameters() {
            if param.constant.is_some() || param.cached {
                continue;
            }
            if hub.model.options.initial_read && param.initial.is_none() {
                let _ = actor.try_send(Job::Poll {
                    parameter: pname.clone(),
                });
            }
            let Some(interval) = param.poll_interval else {
                continue;
            };
            let (actor, parameter, stop) = (actor.clone(), pname.clone(), stop.clone());
            tracker.spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                ticker.tick().await; // the first tick is immediate; the startup read covers it
                loop {
                    tokio::select! {
                        () = stop.cancelled() => break,
                        _ = ticker.tick() => {
                            // skip this poll if the module is busy with a backlog
                            let _ = actor.try_send(Job::Poll { parameter: parameter.clone() });
                        }
                    }
                }
            });
        }
    }
}

impl std::fmt::Debug for RunningNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningNode")
            .field("equipment_id", &self.hub.model.equipment_id)
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl RunningNode {
    /// The address the node listens on (useful after binding to port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Pushes a new value of a parameter: stores it and sends it to activated clients.
    /// Fails if the parameter does not exist or the value does not fit its datainfo.
    pub fn update(
        &self,
        module: &str,
        parameter: &str,
        reading: impl Into<Reading>,
    ) -> Result<(), SecopError> {
        self.hub
            .publish_value(module, parameter, reading.into())
            .map(|_| ())
    }

    /// Pushes that a parameter could not be obtained; activated clients receive an
    /// `error_update`.
    pub fn update_error(&self, module: &str, parameter: &str, error: SecopError) {
        self.hub.publish_error(module, parameter, error);
    }

    /// Subscribes to the node's events.
    pub fn subscribe(&self) -> broadcast::Receiver<NodeEvent> {
        self.hub.events.subscribe()
    }

    /// Stops the node: closes the port and all connections and ends polling. Resolves
    /// once everything has finished.
    pub async fn stop(&self) {
        self.stop.cancel();
        self.tracker.close();
        self.tracker.wait().await;
        info!(addr = %self.addr, "node stopped");
        self.hub.emit(NodeEvent::Stopped);
    }

    /// Stops the node from synchronous code; see [`stop`](Self::stop).
    ///
    /// Must not be called from within an async runtime.
    pub fn stop_blocking(&self) {
        shared_runtime().block_on(self.stop());
    }
}
