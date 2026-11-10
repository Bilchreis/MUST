// SPDX-License-Identifier: LGPL-3.0-or-later

//! The table of live `CSECoPbaseType` handles: C sees opaque pointers, Rust keeps the nodes.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use super::Node;

/// The opaque C type behind `CSECoPbaseType*`.
#[repr(C)]
pub struct CSECoPbaseType {
    _private: [u8; 0],
}

/// All live handles. Every C function checks a handle against this table, as SHALL does,
/// so that stale or foreign pointers are rejected.
///
/// Handles are numbers from a counter, not the nodes' addresses: an address is reused
/// once its node is freed, so a destroyed handle would silently refer to a new value.
/// The counter only repeats after it wraps around (2^29 handles on 32-bit targets), and
/// then skips handles still in use.
static HANDLES: LazyLock<Mutex<HandleTable>> = LazyLock::new(Mutex::default);

/// First handle value; handles step by [`HANDLE_STEP`] so they look like aligned pointers.
const FIRST_HANDLE: usize = 0x1_0000;
const HANDLE_STEP: usize = 8;

struct HandleTable {
    nodes: HashMap<usize, Node>,
    next: usize,
}

impl Default for HandleTable {
    fn default() -> Self {
        Self {
            nodes: HashMap::new(),
            next: FIRST_HANDLE,
        }
    }
}

fn handles() -> MutexGuard<'static, HandleTable> {
    HANDLES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl HandleTable {
    /// Stores `node` under the next free handle.
    fn insert(&mut self, node: Node) -> usize {
        let mut id = self.next;
        while self.nodes.contains_key(&id) {
            id = next_handle(id);
        }
        self.next = next_handle(id);
        self.nodes.insert(id, node);
        id
    }
}

/// Registers a node and returns its handle.
pub fn new_handle(node: Node) -> *mut CSECoPbaseType {
    std::ptr::without_provenance_mut(handles().insert(node))
}

fn next_handle(id: usize) -> usize {
    match id.checked_add(HANDLE_STEP) {
        Some(next) => next,
        None => FIRST_HANDLE,
    }
}

/// Runs `f` on the node of a handle; `None` for NULL or unknown handles.
pub fn with_handle<R>(handle: *const CSECoPbaseType, f: impl FnOnce(&mut Node) -> R) -> Option<R> {
    let mut table = handles();
    table.nodes.get_mut(&handle.addr()).map(f)
}

/// A copy of the node of a handle.
pub fn clone_handle(handle: *const CSECoPbaseType) -> Option<Node> {
    with_handle(handle, |n| n.clone())
}

/// Removes a handle and returns its node; `None` for NULL or unknown handles.
pub fn take_handle(handle: *const CSECoPbaseType) -> Option<Node> {
    handles().nodes.remove(&handle.addr())
}

/// Removes and frees a handle; returns whether it existed.
pub fn destroy_handle(handle: *const CSECoPbaseType) -> bool {
    handles().nodes.remove(&handle.addr()).is_some()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn handle_counter_wraps_and_skips_live_handles() {
        let node = || Node::from_json(&json!(1)).unwrap();
        let mut table = HandleTable::default();
        let first = table.insert(node());
        assert_eq!(first, FIRST_HANDLE);
        // the counter wraps around at the end of the address range ...
        table.next = usize::MAX - 3;
        assert_eq!(table.insert(node()), usize::MAX - 3);
        // ... and skips handles still in use
        assert_eq!(table.insert(node()), FIRST_HANDLE + HANDLE_STEP);
        assert_eq!(table.nodes.len(), 3);
    }
}
