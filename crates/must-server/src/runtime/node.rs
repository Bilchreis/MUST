// SPDX-License-Identifier: LGPL-3.0-or-later

//! Starting and stopping a node.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use must_types::SecopError;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::info;

use super::Hub;
use super::actor::{ActorHandle, Job, spawn_actor};
use super::connection;
use crate::events::{LogLevel, NodeEvent};
use crate::handler::Reading;
use crate::logfiles::LogHandle;
use crate::model::{Node, PollSource};

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
    /// The node's id at the monitoring endpoint, if it registered itself.
    monitor_id: Option<u64>,
    /// The node's file logging, if on.
    log: Option<LogHandle>,
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
        // before anything happens that the log should contain
        let log = crate::logfiles::start(&hub, self.model.options.log_files.as_ref());
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
        let mut running = RunningNode {
            hub,
            addr,
            stop,
            tracker,
            monitor_id: None,
            log,
        };
        if self.model.options.monitored {
            running.monitor_id = crate::monitor::register_running(&running);
        }
        Ok(running)
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

/// Starts the startup reads and one poll task per polled module.
fn start_polling(
    hub: &Arc<Hub>,
    actors: &Arc<HashMap<String, ActorHandle>>,
    tracker: &TaskTracker,
    stop: &CancellationToken,
) {
    for (mname, module) in &hub.model.modules {
        let actor = &actors[mname];
        let mut polled = Vec::new();
        for (pname, param) in module.parameters() {
            if param.constant.is_some() || param.cached {
                continue;
            }
            if hub.model.options.initial_read && param.initial.is_none() {
                let _ = actor.try_send(Job::Refresh {
                    parameter: pname.clone(),
                });
            }
            if param.poll != PollSource::None {
                polled.push((pname.clone(), param.poll));
            }
        }
        if polled.is_empty() {
            continue;
        }
        let fast = hub.watch_poll_interval(mname);
        let initial_fast = fast.as_ref().map_or(module.poll.fast, |rx| *rx.borrow());
        let schedule = Schedule::new(polled, initial_fast, module.poll.slow, Instant::now());
        tracker.spawn(poll_module(actor.clone(), schedule, fast, stop.clone()));
    }
}

/// When each polled parameter of a module is due next.
struct Schedule {
    params: Vec<(String, PollSource)>,
    /// `None` while not polled.
    next: Vec<Option<Instant>>,
    fast: Option<Duration>,
    slow: Duration,
}

impl Schedule {
    /// The first poll of each parameter is one interval after `now`; the startup reads
    /// cover the start.
    fn new(
        params: Vec<(String, PollSource)>,
        fast: Option<Duration>,
        slow: Duration,
        now: Instant,
    ) -> Self {
        let mut schedule = Self {
            next: vec![None; params.len()],
            params,
            fast,
            slow,
        };
        for i in 0..schedule.params.len() {
            schedule.next[i] = schedule.interval(i).map(|d| now + d);
        }
        schedule
    }

    fn interval(&self, i: usize) -> Option<Duration> {
        match self.params[i].1 {
            PollSource::Own(d) => Some(d),
            PollSource::Fast => self.fast,
            PollSource::Slow => Some(self.slow),
            PollSource::None => None,
        }
    }

    fn earliest(&self) -> Option<Instant> {
        self.next.iter().flatten().min().copied()
    }

    /// The parameters due at `now`, in declaration order, each rescheduled one interval
    /// after its due time, or after `now` if it is late by a whole interval.
    fn take_due(&mut self, now: Instant) -> Vec<String> {
        let mut due = Vec::new();
        for i in 0..self.params.len() {
            let Some(at) = self.next[i].filter(|at| *at <= now) else {
                continue;
            };
            due.push(self.params[i].0.clone());
            self.next[i] = self.interval(i).map(|d| {
                let next = at + d;
                if next <= now { now + d } else { next }
            });
        }
        due
    }

    /// Changes the fast interval; the fast parameters are next due one new interval
    /// after `now`, or never for `None`.
    fn set_fast(&mut self, fast: Option<Duration>, now: Instant) {
        self.fast = fast;
        for i in 0..self.params.len() {
            if self.params[i].1 == PollSource::Fast {
                self.next[i] = fast.map(|d| now + d);
            }
        }
    }
}

async fn poll_module(
    actor: ActorHandle,
    mut schedule: Schedule,
    mut fast: Option<watch::Receiver<Option<Duration>>>,
    stop: CancellationToken,
) {
    loop {
        let earliest = schedule.earliest();
        let tick = async {
            match earliest {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        let retimed = async {
            match fast.as_mut() {
                Some(rx) => rx.changed().await.is_ok(),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = stop.cancelled() => break,
            () = tick => {
                let due = schedule.take_due(Instant::now());
                if !due.is_empty() {
                    // skip this tick if the module is busy with a backlog
                    let _ = actor.try_send(Job::Poll { parameters: due });
                }
            }
            ok = retimed => {
                match fast.as_mut() {
                    Some(rx) if ok => {
                        let interval = *rx.borrow_and_update();
                        schedule.set_fast(interval, Instant::now());
                    }
                    // the node is going away
                    _ => fast = None,
                }
            }
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
        if let Some(log) = &self.log {
            log.finished().await;
        }
        if let Some(id) = self.monitor_id {
            crate::monitor::unregister_running(id);
        }
    }

    /// The node's shared state, for the monitoring endpoint.
    pub(crate) fn hub(&self) -> &Arc<Hub> {
        &self.hub
    }

    /// Stops the node from synchronous code; see [`stop`](Self::stop).
    ///
    /// Must not be called from within an async runtime.
    pub fn stop_blocking(&self) {
        shared_runtime().block_on(self.stop());
    }
}
