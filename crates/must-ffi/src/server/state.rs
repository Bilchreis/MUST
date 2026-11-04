// SPDX-License-Identifier: LGPL-3.0-or-later

//! The library's global state: contexts, nodes, queued requests and the runtime.
//!
//! One mutex guards everything and is only held briefly: never while calling the
//! application or waiting for a node to stop.

use std::collections::{HashMap, VecDeque};
use std::ffi::c_int;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use must_server::RunningNode;
use must_server::monitor::ExternalNode;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::oneshot;

use super::build::DraftNode;
use super::codes::err;
use super::filter::Filter;
use super::handler::Answer;
use crate::variant;

/// `SECOP_POLLING_TIMEOUT` of SHALL, in milliseconds.
const DEFAULT_POLL_TIMEOUT_MS: u64 = 60_000;

/// The node `timeout` of nodes built through the C ABI, in seconds: SHALL's 60 s, for
/// polled requests and callbacks alike, instead of MUST's default of 10 s.
pub(crate) const NODE_TIMEOUT: f64 = 60.0;

static POLL_TIMEOUT_MS: AtomicU64 = AtomicU64::new(DEFAULT_POLL_TIMEOUT_MS);

/// How long a polled request waits for `SECoP_S_putCommandAnswer`.
pub(crate) fn poll_timeout() -> Duration {
    Duration::from_millis(POLL_TIMEOUT_MS.load(Ordering::Relaxed))
}

/// Changes [`poll_timeout`]; for tests.
pub(crate) fn set_poll_timeout(timeout: Duration) {
    let ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    POLL_TIMEOUT_MS.store(ms, Ordering::Relaxed);
}

/// A request waiting for the application in polling mode.
pub(crate) struct Pending {
    pub node: String,
    pub module: String,
    pub accessible: String,
    pub action: c_int,
    /// The value of a `change`, the argument of a `do`.
    pub value: Option<variant::Node>,
    pub reply: oneshot::Sender<Answer>,
}

/// A context ID of `SECoP_S_initLibrary`: an application (or LabVIEW VI) using the
/// library independently of others in the same process.
pub(crate) struct Context {
    /// Function pointers (true) or polling (false).
    pub callbacks: bool,
    /// The node that `SECoP_S_add*` calls of this context apply to.
    pub current: Option<String>,
    /// Requests not yet handed out by `SECoP_S_getStoredCommand`.
    pub queue: VecDeque<(u64, Pending)>,
}

pub(crate) struct NodeEntry {
    pub context: String,
    pub callbacks: bool,
    pub draft: DraftNode,
    pub running: Option<RunningNode>,
    /// The node's entry at the monitoring endpoint, from creation on; dropping it
    /// removes the node there.
    pub monitor: Option<ExternalNode>,
}

#[derive(Default)]
pub(crate) struct State {
    pub runtime: Option<Runtime>,
    /// The function pointer mode of the last `SECoP_S_initLibrary`, for contexts that
    /// create nodes without initializing themselves.
    pub default_callbacks: bool,
    pub contexts: HashMap<String, Context>,
    /// All nodes, in creation order.
    pub nodes: Vec<NodeEntry>,
    /// Requests handed out by `SECoP_S_getStoredCommand`, waiting for their answer.
    pub handed_out: HashMap<u64, Pending>,
    pub next_id: u64,
    /// Errors and warnings while building nodes, for `SECoP_S_showErrors`.
    pub errors: Messages,
    pub warnings: Messages,
}

/// How many errors (and as many warnings) are kept for `SECoP_S_showErrors`.
const MAX_MESSAGES: usize = 1000;

/// The latest errors or warnings, for a host that may never call `SECoP_S_showErrors`:
/// beyond [`MAX_MESSAGES`], the oldest are dropped and counted.
#[derive(Default)]
pub(crate) struct Messages {
    list: VecDeque<String>,
    dropped: usize,
}

impl Messages {
    pub(crate) fn push(&mut self, message: String) {
        if self.list.len() >= MAX_MESSAGES {
            self.list.pop_front();
            self.dropped += 1;
        }
        self.list.push_back(message);
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Takes the messages: how many were dropped, and the rest, oldest first.
    pub(crate) fn take(&mut self) -> (usize, VecDeque<String>) {
        let taken = std::mem::take(self);
        (taken.dropped, taken.list)
    }
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

/// Locks the global state.
pub(crate) fn lock() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

impl State {
    pub(crate) fn initialized(&self) -> bool {
        self.runtime.is_some()
    }

    pub(crate) fn node_index(&self, name: &str) -> Option<usize> {
        self.nodes
            .iter()
            .position(|n| n.draft.name.eq_ignore_ascii_case(name))
    }

    /// The node `SECoP_S_add*` calls of `context` apply to.
    pub(crate) fn current_node(&mut self, context: &str) -> Option<&mut NodeEntry> {
        let name = self.contexts.get(context)?.current.clone()?;
        let index = self.node_index(&name)?;
        Some(&mut self.nodes[index])
    }

    /// Records a non-zero result for `SECoP_S_showErrors`.
    pub(crate) fn note(&mut self, code: c_int, message: String) {
        if code < 0 {
            self.errors.push(message);
        } else if code > 0 {
            self.warnings.push(message);
        }
    }

    /// Removes a node and its queued requests; returns it to be stopped.
    pub(crate) fn remove_node(&mut self, index: usize) -> NodeEntry {
        let entry = self.nodes.remove(index);
        let name = &entry.draft.name;
        if let Some(context) = self.contexts.get_mut(&entry.context) {
            context.queue.retain(|(_, p)| &p.node != name);
            if context.current.as_ref() == Some(name) {
                context.current = None;
            }
        }
        self.handed_out.retain(|_, p| &p.node != name);
        entry
    }

    /// Removes all nodes of a context; returns those that run, to be stopped.
    pub(crate) fn remove_context_nodes(&mut self, context: &str) -> Vec<RunningNode> {
        let mut running = Vec::new();
        while let Some(i) = self.nodes.iter().position(|n| n.context == context) {
            running.extend(self.remove_node(i).running);
        }
        running
    }

    pub(crate) fn handle(&self) -> Option<Handle> {
        self.runtime.as_ref().map(|r| r.handle().clone())
    }

    /// Hands out the oldest queued request of `context` that `filter` selects (the
    /// oldest of all without filter): moves it to `handed_out` and returns its id.
    /// `SECoP_S_ERROR_INVALID_CONTEXT_ID` or `SECoP_S_ERROR_NO_DATA` otherwise.
    pub(crate) fn take_matching(
        &mut self,
        context: &str,
        filter: Option<&Filter>,
    ) -> Result<u64, c_int> {
        let queue = &mut self
            .contexts
            .get_mut(context)
            .ok_or(err::INVALID_CONTEXT_ID)?
            .queue;
        // ids grow in queue order: the first match is the oldest
        let index = match filter {
            None => (!queue.is_empty()).then_some(0),
            Some(filter) => queue.iter().position(|(_, p)| filter.matches(p)),
        };
        let (id, pending) = index.and_then(|i| queue.remove(i)).ok_or(err::NO_DATA)?;
        self.handed_out.insert(id, pending);
        Ok(id)
    }
}

/// Queues a polled request of `context`; `None` if the context is gone.
pub(crate) fn enqueue(context: &str, pending: Pending) -> Option<u64> {
    let mut state = lock();
    state.next_id += 1;
    let id = state.next_id;
    state
        .contexts
        .get_mut(context)?
        .queue
        .push_back((id, pending));
    Some(id)
}

/// A queued request of a module. Dropping it (answered, timed out, or abandoned because
/// the node gave up on it) removes what is left of it, so the host is never handed a
/// request nobody waits for, and a late answer is refused.
pub(crate) struct Queued(pub u64);

impl Drop for Queued {
    fn drop(&mut self) {
        forget(self.0);
    }
}

/// Drops a request that timed out, so that a late answer is rejected.
pub(crate) fn forget(id: u64) {
    let mut state = lock();
    if state.handed_out.remove(&id).is_none() {
        for context in state.contexts.values_mut() {
            context.queue.retain(|(i, _)| *i != id);
        }
    }
}

/// Stops nodes and waits (at most a few seconds each) until they have closed their ports.
/// Must be called without holding the lock.
pub(crate) fn stop(nodes: Vec<RunningNode>, handle: Option<Handle>) {
    let Some(handle) = handle else { return };
    if nodes.is_empty() {
        return;
    }
    let work = async move {
        for node in &nodes {
            let _ = tokio::time::timeout(Duration::from_secs(5), node.stop()).await;
        }
    };
    if Handle::try_current().is_ok() {
        // called from within the runtime (e.g. from a callback): wait on another thread
        let _ = std::thread::spawn(move || handle.block_on(work)).join();
    } else {
        handle.block_on(work);
    }
}

/// Shuts the runtime down, so no thread of the library is left running (e.g. before a
/// LabVIEW unloads the library). Must be called without holding the lock.
pub(crate) fn shut_down(runtime: Runtime) {
    if Handle::try_current().is_ok() {
        runtime.shutdown_background();
    } else {
        runtime.shutdown_timeout(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::codes::action;

    /// A context of its own: the state is global and shared with other tests.
    const CONTEXT: &str = "state-tests-queued";

    fn request() -> (u64, oneshot::Receiver<Answer>) {
        let (reply, answer) = oneshot::channel();
        let pending = Pending {
            node: "n".into(),
            module: "m".into(),
            accessible: "p".into(),
            action: 1,
            value: None,
            reply,
        };
        (enqueue(CONTEXT, pending).expect("context exists"), answer)
    }

    fn queued() -> usize {
        lock().contexts[CONTEXT].queue.len()
    }

    #[test]
    fn dropped_request_leaves_the_queue_and_the_handed_out() {
        lock().contexts.insert(
            CONTEXT.into(),
            Context {
                callbacks: false,
                current: None,
                queue: VecDeque::new(),
            },
        );

        // abandoned before the host fetched it: never handed out
        let (id, _answer) = request();
        let guard = Queued(id);
        assert_eq!(queued(), 1);
        drop(guard);
        assert_eq!(queued(), 0);

        // abandoned after the host fetched it: its answer is refused
        let (id, _answer) = request();
        let guard = Queued(id);
        {
            let mut state = lock();
            let (id, pending) = state
                .contexts
                .get_mut(CONTEXT)
                .unwrap()
                .queue
                .pop_front()
                .unwrap();
            state.handed_out.insert(id, pending);
        }
        drop(guard);
        assert!(!lock().handed_out.contains_key(&id));

        lock().contexts.remove(CONTEXT);
    }

    #[test]
    fn take_matching_takes_the_oldest_match_of_one_context() {
        const A: &str = "state-tests-filter-a";
        const B: &str = "state-tests-filter-b";
        let queue = |context: &str, accessible: &str, action: c_int| {
            let pending = Pending {
                node: "n".into(),
                module: "mod1".into(),
                accessible: accessible.into(),
                action,
                value: None,
                reply: oneshot::channel().0,
            };
            enqueue(context, pending).expect("context exists")
        };
        let ids = |context: &str| -> Vec<u64> {
            lock().contexts[context]
                .queue
                .iter()
                .map(|(i, _)| *i)
                .collect()
        };
        for context in [A, B] {
            lock().contexts.insert(
                context.into(),
                Context {
                    callbacks: false,
                    current: None,
                    queue: VecDeque::new(),
                },
            );
        }
        let read_a = queue(A, "a", 1);
        let change_b = queue(A, "b", 2);
        let check_b = queue(A, "b", action::CHECK);
        let read_b = queue(A, "b", 1);
        let other = queue(B, "b", 1);
        let filter = |accessibles: &str, action: c_int| {
            Filter::parse(Some(accessibles), action).expect("valid filter")
        };

        let mut state = lock();
        assert_eq!(
            state.take_matching(A, Some(&filter("", action::CHECK))),
            Ok(check_b)
        );
        assert_eq!(
            state.take_matching(A, Some(&filter("n:mod1:b", 0))),
            Ok(change_b)
        );
        assert!(state.handed_out.contains_key(&change_b));
        assert_eq!(
            state.take_matching(A, Some(&filter("n:mod1:x", 0))),
            Err(err::NO_DATA)
        );
        assert_eq!(state.take_matching(A, Some(&filter("", 1))), Ok(read_a));
        assert_eq!(
            state.take_matching("state-tests-filter-none", None),
            Err(err::INVALID_CONTEXT_ID)
        );
        drop(state);
        // the others stay queued, in order; the other context is untouched
        assert_eq!(ids(A), [read_b]);
        assert_eq!(ids(B), [other]);
        assert_eq!(lock().take_matching(A, None), Ok(read_b));
        assert_eq!(lock().take_matching(A, None), Err(err::NO_DATA));

        let mut state = lock();
        for id in [check_b, change_b, read_a, read_b] {
            state.handed_out.remove(&id);
        }
        state.contexts.remove(A);
        state.contexts.remove(B);
    }

    #[test]
    fn messages_keep_the_latest() {
        let mut messages = Messages::default();
        for i in 0..MAX_MESSAGES + 5 {
            messages.push(i.to_string());
        }
        let (dropped, list) = messages.take();
        assert_eq!(dropped, 5);
        assert_eq!(list.len(), MAX_MESSAGES);
        assert_eq!(list.front().map(String::as_str), Some("5"));
        assert_eq!(messages.take(), (0, VecDeque::new()), "taken");
    }
}
