// SPDX-License-Identifier: LGPL-3.0-or-later

//! A separate host process using the built `SHALLserver` (`examples/c_host.rs`, possibly
//! 32-bit) is discovered and read through the monitoring client library of this
//! (possibly 64-bit) process. Run by `cargo xtask check`.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

use must_monitor::client::{self, Options};
use must_monitor::protocol::NodeState;
use must_monitor::registry::EndpointStatus;

#[tokio::test]
#[ignore = "needs the dist libraries and the c_host example; run by cargo xtask check"]
async fn host_process_is_discovered_and_read() {
    let exe = std::env::var("MUST_HOST_EXE").expect("MUST_HOST_EXE is set");
    let dist = std::env::var("MUST_DIST_DIR").expect("MUST_DIST_DIR is set");
    let registry = std::env::temp_dir().join(format!("must-cross-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&registry);
    let mut host = Command::new(&exe)
        .arg(&dist)
        .env("MUST_MONITOR_DIR", &registry)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{exe}: {e}"));
    let mut ready = String::new();
    BufReader::new(host.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert!(ready.starts_with("ready"), "host said {ready:?}");
    println!(
        "host is {}-bit, this monitor {}-bit",
        ready.trim().trim_start_matches("ready "),
        usize::BITS
    );

    let listed = client::list_endpoints_in(&registry).unwrap();
    let entry = listed
        .iter()
        .find(|l| l.registration.pid == host.id())
        .expect("the host registered its endpoint");
    assert_eq!(entry.status, EndpointStatus::Live);
    let connection = tokio::time::timeout(
        Duration::from_secs(10),
        client::connect(&entry.registration, &Options::new("cross-process test")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(connection.process().pid, host.id());
    let mut nodes: Vec<_> = connection
        .snapshot()
        .nodes
        .iter()
        .map(|n| (n.equipment_id.clone(), n.context.clone(), n.state))
        .collect();
    nodes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        nodes,
        [
            (
                "hostnode_a".to_owned(),
                Some("lv_a".to_owned()),
                NodeState::Running
            ),
            (
                "hostnode_b".to_owned(),
                Some("lv_b".to_owned()),
                NodeState::Running
            ),
        ]
    );
    drop(connection);

    // closing its input ends the host; its registration goes with it
    drop(host.stdin.take());
    assert!(host.wait().unwrap().success());
    assert!(
        !client::list_endpoints_in(&registry)
            .unwrap()
            .iter()
            .any(|l| l.registration.pid == host.id())
    );
    let _ = std::fs::remove_dir_all(&registry);
}
