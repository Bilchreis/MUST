// SPDX-License-Identifier: LGPL-3.0-or-later

//! Nodes built through the C ABI at the monitoring endpoint: listed from
//! `SECoP_S_createNode` on, failed with their messages, running once complete, with
//! their context IDs.

// the guard serializes whole tests, each on its own runtime
#![allow(clippy::await_holding_lock)]

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{free_port, serial};
use must_ffi_ctests::server as c;
use must_monitor::client::{self, Connection, Options};
use must_monitor::protocol::{Event, NodeInfo, NodeState, ServerMessage};
use must_monitor::registry::Registration;

fn check(test: &str, line: std::ffi::c_int) {
    assert_eq!(line, 0, "C test {test} failed at c/server.c line {line}");
}

fn done(context: &std::ffi::CStr) {
    // SAFETY: a C test taking a NUL-terminated string
    check("done", unsafe { c::ctest_server_done(context.as_ptr()) });
}

fn registry() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("must-ctests-monitor-{}", std::process::id()));
    must_server::monitor::set_registry_dir(Some(dir.clone()));
    dir
}

fn own_registration(dir: &std::path::Path) -> Option<Registration> {
    client::list_endpoints_in(dir)
        .ok()?
        .into_iter()
        .find(|l| l.registration.pid == std::process::id())
        .map(|l| l.registration)
}

async fn snapshot(dir: &std::path::Path) -> (Connection, Vec<NodeInfo>) {
    let registration = own_registration(dir).expect("endpoint registered");
    let connection = client::connect(&registration, &Options::new("ctests"))
        .await
        .unwrap();
    let nodes = connection.snapshot().nodes.clone();
    (connection, nodes)
}

fn node<'a>(nodes: &'a [NodeInfo], name: &str) -> &'a NodeInfo {
    nodes
        .iter()
        .find(|n| n.equipment_id == name)
        .unwrap_or_else(|| panic!("node {name} in {nodes:?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c_abi_nodes_at_the_monitor() {
    let _serial = serial();
    let dir = registry();

    // being built: the endpoint exists and lists the node
    // SAFETY: a C test without arguments
    check("monitor_building", unsafe { c::ctest_monitor_building() });
    let (monitor, nodes) = snapshot(&dir).await;
    let building = node(&nodes, "mnode");
    assert_eq!(building.state, NodeState::Building);
    assert_eq!(building.context.as_deref(), Some("mon"));
    assert_eq!(building.structure, None);
    drop(monitor);
    done(c"mon");
    assert!(
        own_registration(&dir).is_none(),
        "no node left, endpoint closed"
    );

    // nodeComplete fails: listed as failed with the message naming the parameter
    let port = free_port();
    // SAFETY: a C test with a port argument
    check("validation_fails", unsafe {
        c::ctest_server_validation_fails(port)
    });
    let (mut m, nodes) = snapshot(&dir).await;
    let failed = node(&nodes, "vnode");
    assert_eq!(failed.state, NodeState::Failed);
    assert!(
        failed.messages.iter().any(|msg| msg.contains("nodatainfo")),
        "{:?}",
        failed.messages
    );
    let id = failed.id;

    // fixed and completed: running
    // SAFETY: a C test without arguments
    check("validation_fixed", unsafe {
        c::ctest_server_validation_fixed()
    });
    let started = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match m.next().await.unwrap() {
                Some(ServerMessage::Event(Event::NodeStarted { node, address, .. }))
                    if node == id =>
                {
                    break address;
                }
                Some(_) => {}
                None => panic!("endpoint closed"),
            }
        }
    })
    .await
    .unwrap();
    assert!(started.ends_with(&format!(":{port}")));
    let (second, nodes) = snapshot(&dir).await;
    assert_eq!(node(&nodes, "vnode").state, NodeState::Running);
    drop((m, second));
    done(c"val");

    // two contexts in one process
    let (port1, port2) = (free_port(), free_port());
    // SAFETY: a C test with port arguments
    check("two_contexts", unsafe {
        c::ctest_server_two_contexts(port1, port2)
    });
    let (monitor, nodes) = snapshot(&dir).await;
    assert_eq!(node(&nodes, "node1").context.as_deref(), Some("id1"));
    assert_eq!(node(&nodes, "node2").context.as_deref(), Some("id2"));
    assert!(nodes.iter().all(|n| n.state == NodeState::Running));
    drop(monitor);
    done(c"id1");
    done(c"id2");
    assert!(own_registration(&dir).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
