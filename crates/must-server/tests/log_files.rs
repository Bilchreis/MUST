// SPDX-License-Identifier: LGPL-3.0-or-later

//! File logging with real nodes and clients.

#![allow(clippy::await_holding_lock)] // tests are serialized with a std mutex

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use common::{Client, Sim, kelvin, node, sensor, status_info};
use must_server::logfiles::{LogFiles, pause_writer};
use must_server::{Module, NodeBuilder, Parameter, RunningNode};
use must_types::Value;
use tokio::runtime::Handle;

/// The writer thread is shared by the process; tests that stall it or count lines run
/// one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    LazyLock::force(&WARNINGS);
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Warnings logged through `tracing` by any thread of this process.
static WARNINGS: LazyLock<Arc<Mutex<Vec<u8>>>> = LazyLock::new(|| {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buffer);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || Sink(Arc::clone(&sink)))
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    buffer
});

struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn warnings() -> String {
    String::from_utf8_lossy(&WARNINGS.lock().unwrap()).into_owned()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("must-logs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn start(builder: NodeBuilder) -> RunningNode {
    builder
        .monitored(false)
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

fn logged(dir: &Path) -> NodeBuilder {
    node(Arc::new(Sim::default())).log_files(LogFiles::default().dir(dir))
}

/// The lines of a file without their time stamps.
fn read_log(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .map(|line| {
            let (time, rest) = line.split_once(' ').unwrap();
            assert!(
                time.len() >= 29 && time.contains('T'),
                "time stamp {time:?}"
            );
            rest.to_owned()
        })
        .collect()
}

fn node_log(dir: &Path) -> Vec<String> {
    read_log(&dir.join("must-test").join(format!("{}.log", today())))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_nodes() {
    let _serial = serial();
    let dir = temp("two");
    let nodes: Vec<_> = ["cryo", "magnet"]
        .into_iter()
        .map(|name| {
            start(
                NodeBuilder::new(name, "a node")
                    .bind(([127, 0, 0, 1], 0).into())
                    .module(sensor("t1"))
                    .unwrap()
                    .log_files(LogFiles::default().dir(&dir)),
            )
        })
        .collect();
    for n in &nodes {
        n.stop().await;
    }
    for name in ["cryo", "magnet"] {
        let lines = read_log(&dir.join(name).join(format!("{}.log", today())));
        assert!(
            lines[0].starts_with("* node started on 127.0.0.1:"),
            "{lines:?}"
        );
        assert_eq!(lines.last().unwrap(), "* node stopped");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_reply_and_handler_error() {
    let _serial = serial();
    let dir = temp("request");
    let n = start(logged(&dir));
    let mut c = Client::connect(n.local_addr()).await;
    let reply = c.request("read t1:value").await;
    assert!(c.request("read t1:broken").await.starts_with("error_read"));
    n.stop().await;

    let lines = node_log(&dir);
    let read = lines
        .iter()
        .position(|l| l.starts_with("> [") && l.ends_with("] read t1:value"))
        .unwrap_or_else(|| panic!("{lines:#?}"));
    let client = &lines[read][2..=lines[read].find(']').unwrap()];
    assert!(
        client.contains("127.0.0.1:"),
        "client with its address: {client}"
    );
    // the next line sent to that client is the reply (the value read is also
    // published, as a `U` line, in either order with the reply)
    let next_sent = lines[read + 1..]
        .iter()
        .find(|l| l.starts_with(&format!("< {client}")))
        .unwrap();
    assert_eq!(*next_sent, format!("< {client} {reply}"));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("U update t1:value [295"))
    );
    assert!(lines.contains(&format!("* {client} connected")));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("WARN t1: ") && l.contains("sensor disconnected")),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(&format!("* {client} disconnected: ")))
    );
    // complete up to the stop
    assert_eq!(lines.last().unwrap(), "* node stopped");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn updates_logged_once() {
    let _serial = serial();
    let dir = temp("once");
    let n = start(logged(&dir));
    let mut clients = Vec::new();
    for _ in 0..3 {
        let mut c = Client::connect(n.local_addr()).await;
        c.send("activate").await;
        while !c.line().await.starts_with("active") {}
        clients.push(c);
    }
    n.update("t1", "value", Value::Double(123.5)).unwrap();
    for c in &mut clients {
        assert!(c.line().await.starts_with("update t1:value [123.5"));
    }
    n.stop().await;

    let lines = node_log(&dir);
    let updates = lines
        .iter()
        .filter(|l| l.starts_with("U update t1:value [123.5,"))
        .count();
    assert_eq!(updates, 1, "{lines:#?}");
    // activation: request and reply, but not the initial updates sent to the client
    assert_eq!(
        lines.iter().filter(|l| l.ends_with("] activate")).count(),
        3
    );
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("< [") && l.contains("] active"))
            .count(),
        3
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("< [") && l.contains("] update ")),
        "{lines:#?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_activated_client() {
    let _serial = serial();
    let dir = temp("noclient");
    let sim = Arc::new(Sim::default());
    let polled = Module::new("p1", "a polled sensor")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .poll_interval(Duration::from_millis(50))
        .handler_arc(Arc::clone(&sim) as Arc<dyn must_server::ModuleHandler>);
    let n = start(
        node(sim)
            .module(polled)
            .unwrap()
            .log_files(LogFiles::default().dir(&dir)),
    );
    n.update("t1", "value", Value::Double(100.0)).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    n.stop().await;

    let lines = node_log(&dir);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("U update t1:value [100"))
    );
    let polls = lines
        .iter()
        .filter(|l| l.starts_with("U update p1:value [295"))
        .count();
    assert!(polls >= 2, "each polled value is logged: {lines:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turned_off_by_the_rust_api() {
    let _serial = serial();
    let dir = temp("off");
    let n = start(node(Arc::new(Sim::default())).log_files(LogFiles::off().dir(&dir)));
    let mut c = Client::connect(n.local_addr()).await;
    assert!(c.request("ping x").await.starts_with("pong x"));
    n.stop().await;
    assert!(!dir.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unwritable_directory() {
    let _serial = serial();
    let base = temp("unwritable");
    std::fs::create_dir_all(&base).unwrap();
    let blocker = base.join("a-file");
    std::fs::write(&blocker, "").unwrap();
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut cases = vec![blocker.join("logs")];
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let read_only = base.join("read-only");
        std::fs::create_dir(&read_only).unwrap();
        std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o500)).unwrap();
        // root may write anyway
        if std::fs::write(read_only.join("probe"), "").is_err() {
            cases.push(read_only);
        } else {
            println!("running as root: mode 0500 not tested");
        }
    }
    for dir in cases {
        let n = start(logged(&dir));
        let mut c = Client::connect(n.local_addr()).await;
        for _ in 0..3 {
            assert!(c.request("ping x").await.starts_with("pong x"));
        }
        n.stop().await;
        let named = dir.join("must-test").display().to_string();
        let count = warnings().matches(&named).count();
        assert_eq!(count, 1, "one warning naming {named}:\n{}", warnings());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(
            base.join("read-only"),
            std::fs::Permissions::from_mode(0o700),
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// Runs `pings` pings one after the other; the slowest round trip.
async fn slowest_ping(n: &RunningNode, pings: usize) -> Duration {
    let mut c = Client::connect(n.local_addr()).await;
    let mut slowest = Duration::ZERO;
    for i in 0..pings {
        let start = Instant::now();
        assert!(c.request(&format!("ping {i}")).await.starts_with("pong"));
        slowest = slowest.max(start.elapsed());
    }
    slowest
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_writer_adds_no_latency() {
    const PINGS: usize = 6000; // 12000 lines, more than the writer's queue holds
    let _serial = serial();
    let n = start(node(Arc::new(Sim::default())));
    let started = Instant::now();
    let slowest_off = slowest_ping(&n, PINGS).await;
    let total_off = started.elapsed();
    n.stop().await;

    let dir = temp("stalled");
    let n = start(logged(&dir));
    let pause = pause_writer();
    let started = Instant::now();
    let slowest_on = slowest_ping(&n, PINGS).await;
    let total_on = started.elapsed();
    drop(pause);
    n.stop().await;
    println!(
        "without logging: {total_off:?} (slowest {slowest_off:?}); \
         stalled writer: {total_on:?} (slowest {slowest_on:?})"
    );
    assert!(
        total_on < total_off * 2 + Duration::from_millis(500),
        "{total_on:?} vs {total_off:?}"
    );
    assert!(
        slowest_on < slowest_off * 2 + Duration::from_millis(500),
        "{slowest_on:?} vs {slowest_off:?}"
    );

    let lines = node_log(&dir);
    let dropped: u64 = lines
        .iter()
        .filter_map(|l| {
            l.strip_prefix("* dropped ")?
                .strip_suffix(" lines")?
                .parse::<u64>()
                .ok()
        })
        .sum();
    assert!(dropped > 0, "a dropped-lines line");
    assert_eq!(lines.last().unwrap(), "* node stopped");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- child processes

/// Run in a child process: a node named `$MUST_TEST_NODE`, logging into `$MUST_TEST_LOG_DIR`
/// if set (else as the environment says), answers a ping and stops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "run in a child process"]
async fn child_node() {
    let name = std::env::var("MUST_TEST_NODE").unwrap();
    let mut builder = NodeBuilder::new(&name, "a node")
        .bind(([127, 0, 0, 1], 0).into())
        .module(sensor("t1"))
        .unwrap();
    if let Some(dir) = std::env::var_os("MUST_TEST_LOG_DIR") {
        builder = builder.log_files(LogFiles::default().dir(dir));
    }
    let n = start(builder);
    let mut c = Client::connect(n.local_addr()).await;
    assert!(c.request("ping x").await.starts_with("pong x"));
    n.stop().await;
}

fn child(name: &str, env: &[(&str, &std::ffi::OsStr)]) -> u32 {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", "child_node", "--test-threads=1"])
        .env("MUST_TEST_NODE", name)
        .env_remove("MUST_LOG_FILES")
        .env_remove("MUST_LOG_DIR")
        .env_remove("MUST_LOG_DAYS")
        .env("MUST_MONITOR", "0");
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    let pid = child.id();
    assert!(child.wait().unwrap().success());
    pid
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn node_name_used_twice() {
    let _serial = serial();
    let dir = temp("twice");
    let n = start(
        NodeBuilder::new("cryo", "a node")
            .bind(([127, 0, 0, 1], 0).into())
            .module(sensor("t1"))
            .unwrap()
            .log_files(LogFiles::default().dir(&dir)),
    );
    let first = dir.join("cryo").join(format!("{}.log", today()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !first.exists() {
        assert!(Instant::now() < deadline, "{} not created", first.display());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pid = child("cryo", &[("MUST_TEST_LOG_DIR", dir.as_os_str())]);
    n.stop().await;

    let own = read_log(&first);
    let other = read_log(&dir.join("cryo").join(format!("{}-{pid}.log", today())));
    assert_eq!(
        own.iter()
            .filter(|l| l.starts_with("* node started"))
            .count(),
        1
    );
    assert_eq!(
        other
            .iter()
            .filter(|l| l.starts_with("* node started"))
            .count(),
        1
    );
    assert!(other.iter().any(|l| l.ends_with("] ping x")));
    assert!(!own.iter().any(|l| l.ends_with("] ping x")));
    assert_eq!(other.last().unwrap(), "* node stopped");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn turned_off_by_the_environment() {
    let dir = temp("env-off");
    child(
        "envnode",
        &[
            ("MUST_LOG_FILES", "0".as_ref()),
            ("MUST_LOG_DIR", dir.as_os_str()),
        ],
    );
    assert!(!dir.exists(), "no directory created");
}

#[test]
fn directory_and_days_from_the_environment() {
    let dir = temp("env-dir");
    let folder = dir.join("envnode");
    std::fs::create_dir_all(&folder).unwrap();
    let day = |days_ago: u64| {
        (chrono::Local::now().date_naive() - chrono::Days::new(days_ago))
            .format("%Y-%m-%d.log")
            .to_string()
    };
    for days_ago in [7, 8] {
        std::fs::write(folder.join(day(days_ago)), "old\n").unwrap();
    }
    child(
        "envnode",
        &[
            ("MUST_LOG_DIR", dir.as_os_str()),
            ("MUST_LOG_DAYS", "7".as_ref()),
        ],
    );
    let lines = read_log(&folder.join(format!("{}.log", today())));
    assert!(lines.iter().any(|l| l.ends_with("] ping x")), "{lines:#?}");
    assert!(folder.join(day(7)).exists(), "kept for 7 days");
    assert!(!folder.join(day(8)).exists(), "deleted after 7 days");
    let _ = std::fs::remove_dir_all(&dir);
}
