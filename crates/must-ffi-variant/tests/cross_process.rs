// SPDX-License-Identifier: LGPL-3.0-or-later

//! A separate host process using the built `SHALLserver` (`examples/c_host.rs`, possibly
//! 32-bit) is discovered and read through the monitoring client library of this
//! (possibly 64-bit) process, and writes its nodes' log files where `MUST_LOG_DIR` says.
//! Run by `cargo xtask check`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use must_monitor::client::{self, Options};
use must_monitor::protocol::NodeState;
use must_monitor::registry::EndpointStatus;

#[tokio::test]
#[allow(clippy::too_many_lines)] // one scenario, step by step
#[ignore = "needs the dist libraries and the c_host example; run by cargo xtask check"]
async fn host_process_is_discovered_and_read() {
    let exe = std::env::var("MUST_HOST_EXE").expect("MUST_HOST_EXE is set");
    let dist = std::env::var("MUST_DIST_DIR").expect("MUST_DIST_DIR is set");
    let registry = std::env::temp_dir().join(format!("must-cross-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&registry);
    let logs = std::env::temp_dir().join(format!("must-cross-logs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&logs);
    let day = |days_ago: u64| {
        (chrono::Local::now().date_naive() - chrono::Days::new(days_ago))
            .format("%Y-%m-%d.log")
            .to_string()
    };
    let node_logs = logs.join("hostnode_a");
    std::fs::create_dir_all(&node_logs).unwrap();
    for days_ago in [7, 8] {
        std::fs::write(node_logs.join(day(days_ago)), "old\n").unwrap();
    }
    let mut host = Command::new(&exe)
        .arg(&dist)
        .env("MUST_MONITOR_DIR", &registry)
        .env("MUST_LOG_DIR", &logs)
        .env("MUST_LOG_DAYS", "7")
        .env_remove("MUST_LOG_FILES")
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
    let address = connection
        .snapshot()
        .nodes
        .iter()
        .find(|n| n.equipment_id == "hostnode_a")
        .and_then(|n| n.address.clone())
        .expect("hostnode_a has an address");
    drop(connection);

    // traffic for the log file
    let mut secop = TcpStream::connect(&address).unwrap();
    secop
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    secop.write_all(b"*IDN?\n").unwrap();
    let mut idn = String::new();
    BufReader::new(&secop).read_line(&mut idn).unwrap();
    assert!(idn.starts_with("ISSE"), "{idn:?}");
    drop(secop);

    // closing its input ends the host; its registration goes with it
    drop(host.stdin.take());
    assert!(host.wait().unwrap().success());
    assert!(
        !client::list_endpoints_in(&registry)
            .unwrap()
            .iter()
            .any(|l| l.registration.pid == host.id())
    );

    // the log file of today, complete up to the stop, and 7 days kept
    let log = std::fs::read_to_string(node_logs.join(day(0))).unwrap();
    println!("{log}");
    let lines: Vec<&str> = log.lines().map(|l| l.split_once(' ').unwrap().1).collect();
    assert!(lines[0].starts_with("* node started on 127.0.0.1:"));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("> [") && l.ends_with("] *IDN?"))
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("< [") && l.contains("] ISSE"))
    );
    assert_eq!(lines.last(), Some(&"* node stopped"));
    assert!(node_logs.join(day(7)).exists(), "kept for 7 days");
    assert!(!node_logs.join(day(8)).exists(), "deleted after 7 days");
    assert!(logs.join("hostnode_b").join(day(0)).exists());
    let _ = std::fs::remove_dir_all(&registry);
    let _ = std::fs::remove_dir_all(&logs);
}
