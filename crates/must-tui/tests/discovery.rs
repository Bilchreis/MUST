// SPDX-License-Identifier: LGPL-3.0-or-later

//! Discovery with real host processes: a node appears, its process exits, it is removed.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use must_monitor::client::Options;
use must_monitor::protocol::NodeState;
use must_server::logfiles::LogFiles;
use must_server::{Module, NodeBuilder, Parameter};
use must_tui::app::{App, GONE_REMOVAL, Msg};
use must_tui::discovery;
use must_types::{DataInfo, Value};
use tokio::sync::mpsc;

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Run in a child process: serves node `$MUST_TUI_TEST_NODE`, registered in
/// `$MUST_TUI_TEST_DIR`, until its standard input closes.
#[test]
#[ignore = "run in a child process"]
fn host() {
    let dir = PathBuf::from(std::env::var_os("MUST_TUI_TEST_DIR").unwrap());
    let name = std::env::var("MUST_TUI_TEST_NODE").unwrap();
    must_server::monitor::set_registry_dir(Some(dir));
    let kelvin = DataInfo::double().unit("K").build().unwrap();
    let status = DataInfo::tuple(&[
        DataInfo::enumeration([("IDLE", 100)]).build().unwrap(),
        DataInfo::string().build().unwrap(),
    ])
    .build()
    .unwrap();
    let node = NodeBuilder::new(&name, "a test node")
        .bind(([127, 0, 0, 1], 0).into())
        .log_files(LogFiles::off())
        .module(
            Module::new("t1", "a sensor")
                .interface_classes(["Readable"])
                .parameter(
                    Parameter::new("status", status, "status")
                        .cached()
                        .initial_value(Value::Tuple(vec![
                            Value::Enum(100),
                            Value::String("idle".into()),
                        ])),
                )
                .unwrap()
                .parameter(
                    Parameter::new("value", kelvin, "temperature")
                        .cached()
                        .initial_value(Value::Double(4.2)),
                )
                .unwrap(),
        )
        .unwrap()
        .build()
        .unwrap()
        .start()
        .unwrap();
    println!("ready");
    std::io::stdout().flush().unwrap();
    for _ in std::io::stdin().lock().lines() {}
    node.stop_blocking();
}

fn start_host(dir: &Path, name: &str) -> Child {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "host",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("MUST_TUI_TEST_DIR", dir)
        .env("MUST_TUI_TEST_NODE", name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.trim_end().ends_with("ready") {
        line.clear();
        assert_ne!(out.read_line(&mut line).unwrap(), 0, "host ended early");
    }
    // the rest of its output, so the child never writes into a closed pipe
    std::thread::spawn(move || std::io::copy(&mut out, &mut std::io::sink()));
    child
}

/// Feeds the app until `done` holds; panics after `within`.
async fn pump(
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<Msg>,
    within: Duration,
    what: &str,
    done: impl Fn(&App) -> bool,
) -> Duration {
    let started = Instant::now();
    while !done(app) {
        let left = within.saturating_sub(started.elapsed());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(msg)) => {
                app.update(msg);
            }
            _ => panic!("{what} not within {within:?}: {app:#?}"),
        }
    }
    started.elapsed()
}

fn node_state(app: &App, name: &str) -> Option<NodeState> {
    app.processes
        .values()
        .flat_map(|p| p.nodes.values())
        .find(|n| n.info.equipment_id == name)
        .map(|n| n.info.state)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nodes_appear_and_hosts_go() {
    let dir = std::env::temp_dir().join(format!("must-tui-discovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    must_monitor::registry::prepare_dir(&dir).unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (_effects, effects_rx) = mpsc::unbounded_channel();
    tokio::spawn(discovery::run(
        dir.clone(),
        Options::new("test"),
        tx,
        effects_rx,
    ));
    let mut app = App::new(now(), None);

    // a node started while the TUI runs appears within 3 s
    let mut first = start_host(&dir, "tui-first");
    let took = pump(
        &mut app,
        &mut rx,
        Duration::from_secs(3),
        "first node",
        |a| node_state(a, "tui-first") == Some(NodeState::Running),
    )
    .await;
    println!("first node seen after {took:?}");
    let mut second = start_host(&dir, "tui-second");
    pump(
        &mut app,
        &mut rx,
        Duration::from_secs(3),
        "second node",
        |a| node_state(a, "tui-second") == Some(NodeState::Running),
    )
    .await;
    assert_eq!(app.processes.len(), 2, "grouped by process");

    // the first host exits: its process is shown as gone
    drop(first.stdin.take());
    assert!(first.wait().unwrap().success());
    let first_pid = first.id();
    pump(&mut app, &mut rx, Duration::from_secs(5), "gone", |a| {
        a.processes
            .values()
            .any(|p| p.pid == first_pid && p.gone_since.is_some())
    })
    .await;
    // ... and is removed after 60 s, unless pinned
    let second_key = *app
        .processes
        .iter()
        .find(|(_, p)| p.pid == second.id())
        .unwrap()
        .0;
    drop(second.stdin.take());
    assert!(second.wait().unwrap().success());
    pump(
        &mut app,
        &mut rx,
        Duration::from_secs(5),
        "second gone",
        |a| a.processes[&second_key].gone_since.is_some(),
    )
    .await;
    app.processes[&second_key].pinned = true;
    let later = app.now + GONE_REMOVAL + 1.0;
    app.update(Msg::Tick { now: later });
    assert!(!app.processes.values().any(|p| p.pid == first_pid));
    assert!(app.processes.contains_key(&second_key), "pinned stays");
    let _ = std::fs::remove_dir_all(&dir);
}
