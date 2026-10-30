// SPDX-License-Identifier: LGPL-3.0-or-later

//! The log writer thread runs only while a node logs. A test binary of its own, so that
//! no other test's node keeps the thread running.

#![cfg(target_os = "linux")]

mod common;

use std::path::Path;

use common::sensor;
use must_server::logfiles::LogFiles;
use must_server::{NodeBuilder, RunningNode};
use tokio::runtime::Handle;

/// The names of this process's threads.
fn threads() -> Vec<String> {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .map(|name| name.trim_end().to_owned())
        .collect()
}

fn writers() -> usize {
    threads().iter().filter(|t| *t == "must-log-writer").count()
}

fn start(dir: &Path, name: &str) -> RunningNode {
    NodeBuilder::new(name, "a node")
        .bind(([127, 0, 0, 1], 0).into())
        .module(sensor("t1"))
        .unwrap()
        .monitored(false)
        .log_files(LogFiles::default().dir(dir))
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

fn log_of(dir: &Path, name: &str) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d");
    std::fs::read_to_string(dir.join(name).join(format!("{today}.log"))).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writer_thread_ends_with_the_last_logging_node() {
    let dir = std::env::temp_dir().join(format!("must-logs-thread-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(writers(), 0, "{:?}", threads());

    let a = start(&dir, "a");
    let b = start(&dir, "b");
    assert_eq!(writers(), 1, "one writer for all nodes");
    a.stop().await;
    assert_eq!(writers(), 1, "node b still logs");
    b.stop().await;
    assert_eq!(writers(), 0, "{:?}", threads());
    assert!(log_of(&dir, "b").trim_end().ends_with("* node stopped"));

    // logging starts the thread again
    let c = start(&dir, "c");
    assert_eq!(writers(), 1);
    c.stop().await;
    assert_eq!(writers(), 0, "{:?}", threads());
    assert!(log_of(&dir, "c").trim_end().ends_with("* node stopped"));
    let _ = std::fs::remove_dir_all(&dir);
}
