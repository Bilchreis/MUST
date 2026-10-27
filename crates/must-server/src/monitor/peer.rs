// SPDX-License-Identifier: LGPL-3.0-or-later

//! One monitor connection: hello, snapshot, then queued events.
//!
//! Every monitor has a bounded queue. Pushing never waits: when the queue is full, the
//! oldest message is dropped and counted, and the monitor gets a `lagged` message with
//! the count before the next one. Parameter updates are coalesced per monitor: the latest
//! state of each parameter is kept and flushed every interval, unless the monitor asked
//! for every update of a node.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use indexmap::IndexMap;
use must_monitor::protocol::{
    ClientMessage, Event, NodeId, PROTOCOL_MAJOR, ParameterState, ServerMessage, UpdateMode,
    major_version, protocol_version, to_line,
};
use must_monitor::transport::{Listener, Stream};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::Shared;

/// Messages a monitor may have queued before the oldest are dropped.
const QUEUE_CAPACITY: usize = 4096;
/// Longest line accepted from a monitor.
const MAX_LINE: u64 = 64 * 1024;
/// How long a new connection may take to send its hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct PeerState {
    queue: VecDeque<ServerMessage>,
    dropped: u64,
    traffic: HashSet<NodeId>,
    all_updates: HashSet<NodeId>,
    /// Latest state per (node, module, parameter), waiting for the next flush.
    dirty: IndexMap<(NodeId, String, String), ParameterState>,
}

/// A connected monitor, as seen by the event producers.
pub(super) struct Peer {
    state: Mutex<PeerState>,
    ready: Notify,
}

impl Peer {
    fn new() -> Self {
        Self {
            state: Mutex::new(PeerState::default()),
            ready: Notify::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PeerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues a message, dropping the oldest when full.
    pub(super) fn push(&self, message: ServerMessage) {
        let mut state = self.lock();
        enqueue(&mut state, message);
        drop(state);
        self.ready.notify_one();
    }

    /// A parameter's new state: sent now for nodes with every update, else coalesced.
    pub(super) fn update(&self, node: NodeId, parameter: ParameterState) {
        let mut state = self.lock();
        if state.all_updates.contains(&node) {
            enqueue(
                &mut state,
                ServerMessage::Event(Event::Update {
                    node,
                    state: parameter,
                }),
            );
            drop(state);
            self.ready.notify_one();
        } else {
            let key = (node, parameter.module.clone(), parameter.parameter.clone());
            state.dirty.insert(key, parameter);
        }
    }

    pub(super) fn wants_traffic(&self, node: NodeId) -> bool {
        self.lock().traffic.contains(&node)
    }

    /// Moves the coalesced updates into the queue.
    fn flush(&self) {
        let mut state = self.lock();
        if state.dirty.is_empty() {
            return;
        }
        let dirty = std::mem::take(&mut state.dirty);
        for ((node, _, _), parameter) in dirty {
            enqueue(
                &mut state,
                ServerMessage::Event(Event::Update {
                    node,
                    state: parameter,
                }),
            );
        }
        drop(state);
        self.ready.notify_one();
    }

    /// The next message to write: a `lagged` notice first if messages were dropped.
    fn next(&self) -> Option<ServerMessage> {
        let mut state = self.lock();
        if state.dropped > 0 {
            let dropped = std::mem::take(&mut state.dropped);
            return Some(ServerMessage::Lagged { dropped });
        }
        state.queue.pop_front()
    }

    fn apply(&self, message: &ClientMessage) {
        let mut state = self.lock();
        match *message {
            ClientMessage::SubscribeTraffic { node, enabled } => {
                if enabled {
                    state.traffic.insert(node);
                } else {
                    state.traffic.remove(&node);
                }
            }
            ClientMessage::SubscribeUpdates { node, mode } => match mode {
                UpdateMode::All => {
                    state.all_updates.insert(node);
                    // pending coalesced values of the node go out as they are
                    let pending: Vec<_> = state
                        .dirty
                        .keys()
                        .filter(|(n, _, _)| *n == node)
                        .cloned()
                        .collect();
                    for key in pending {
                        if let Some(parameter) = state.dirty.shift_remove(&key) {
                            enqueue(
                                &mut state,
                                ServerMessage::Event(Event::Update {
                                    node,
                                    state: parameter,
                                }),
                            );
                        }
                    }
                }
                UpdateMode::Coalesced => {
                    state.all_updates.remove(&node);
                }
            },
            // a second hello changes nothing
            ClientMessage::Hello { .. } => {}
        }
    }
}

fn enqueue(state: &mut PeerState, message: ServerMessage) {
    if state.queue.len() >= QUEUE_CAPACITY {
        state.queue.pop_front();
        state.dropped += 1;
    }
    state.queue.push_back(message);
}

/// Accepts monitors until shutdown.
pub(super) async fn accept_loop(
    shared: Arc<Shared>,
    mut listener: Listener,
    shutdown: CancellationToken,
) {
    loop {
        let accepted = tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok(stream) => {
                tokio::spawn(serve(Arc::clone(&shared), stream, shutdown.clone()));
            }
            // e.g. too many open files; try again shortly
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Reads one line of at most [`MAX_LINE`] bytes; `None` at the end or for an overlong
/// or invalid line.
async fn read_line(reader: &mut BufReader<impl tokio::io::AsyncRead + Unpin>) -> Option<String> {
    let mut line = String::new();
    let n = reader.take(MAX_LINE).read_line(&mut line).await.ok()?;
    (n > 0 && line.ends_with('\n')).then_some(line)
}

/// Serves one monitor until it disconnects or the endpoint closes.
async fn serve(shared: Arc<Shared>, stream: Stream, shutdown: CancellationToken) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);

    // the first message must be a hello; otherwise close without a word
    let Ok(Some(first)) = tokio::time::timeout(HELLO_TIMEOUT, read_line(&mut reader)).await else {
        return;
    };
    let Ok(ClientMessage::Hello {
        version,
        update_interval,
        ..
    }) = serde_json::from_str::<ClientMessage>(&first)
    else {
        return;
    };
    if major_version(&version) != Some(PROTOCOL_MAJOR) {
        let refusal = ServerMessage::Error {
            message: format!(
                "protocol version {version} is not supported; this endpoint speaks {}",
                protocol_version()
            ),
        };
        let _ = writer.write_all(to_line(&refusal).as_bytes()).await;
        let _ = writer.shutdown().await;
        return;
    }
    let interval = Duration::from_secs_f64(update_interval.unwrap_or(1.0).clamp(0.1, 60.0));

    // welcome and snapshot are queued, and the monitor registered, under the lock that
    // event producers take: nothing is missed or seen twice
    let peer = Arc::new(Peer::new());
    {
        let mut inner = shared.lock();
        peer.push(ServerMessage::Welcome {
            version: protocol_version(),
            process: shared.process.clone(),
        });
        peer.push(ServerMessage::Snapshot(inner.snapshot(&shared.process)));
        inner.peers.push(Arc::clone(&peer));
    }

    let done = shutdown.child_token();
    let writing = {
        let (peer, done) = (Arc::clone(&peer), done.clone());
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                while let Some(message) = peer.next() {
                    if writer
                        .write_all(to_line(&message).as_bytes())
                        .await
                        .is_err()
                    {
                        done.cancel();
                        return;
                    }
                }
                tokio::select! {
                    () = done.cancelled() => return,
                    () = peer.ready.notified() => {}
                    _ = ticker.tick() => peer.flush(),
                }
            }
        })
    };
    loop {
        let line = tokio::select! {
            () = done.cancelled() => break,
            line = read_line(&mut reader) => line,
        };
        let Some(line) = line else { break };
        // read-only: subscriptions only; anything else is ignored
        if let Ok(message) = serde_json::from_str::<ClientMessage>(&line) {
            peer.apply(&message);
        }
    }
    done.cancel();
    shared.lock().peers.retain(|p| !Arc::ptr_eq(p, &peer));
    let _ = writing.await;
}
