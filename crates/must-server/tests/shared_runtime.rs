// SPDX-License-Identifier: LGPL-3.0-or-later

//! The library's shared runtime is only created for nodes started with `Node::start`. A
//! test binary of its own, so that no other test's node creates it.

#![cfg(target_os = "linux")]

mod common;

use std::sync::Arc;

use common::{Sim, node};

/// Threads of the shared runtime in this process.
fn shared_runtime_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.trim_end() == "must-node")
        .count()
}

#[test]
fn stop_blocking_a_spawned_node_needs_no_shared_runtime() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let n = node(Arc::new(Sim::default()))
        .monitored(false)
        .build()
        .unwrap()
        .spawn(runtime.handle())
        .unwrap();
    let addr = n.local_addr();
    n.stop_blocking();
    assert!(std::net::TcpStream::connect(addr).is_err(), "stopped");
    assert_eq!(shared_runtime_threads(), 0);
}
