// SPDX-License-Identifier: LGPL-3.0-or-later

//! The TUI's app and discovery against live endpoints: a real node's traffic, an endpoint
//! that refuses the protocol version, and a recording endpoint that proves the TUI only
//! reads.

// tests are serialized with a std mutex
#![allow(clippy::await_holding_lock)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use must_monitor::client::Options;
use must_monitor::protocol::{
    ClientMessage, NodeInfo, NodeState, ProcessInfo, ServerMessage, Snapshot, protocol_version,
    to_line,
};
use must_monitor::registry::{self, Registration};
use must_monitor::{platform, transport};
use must_server::logfiles::LogFiles;
use must_server::{Module, NodeBuilder, Parameter};
use must_tui::app::{App, Effect, Link, Msg};
use must_tui::discovery;
use must_types::{DataInfo, Value};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("must-tui-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    registry::prepare_dir(&dir).unwrap();
    dir
}

/// The app with discovery running on `dir`: feed it with [`Tui::pump`].
struct Tui {
    app: App,
    rx: mpsc::UnboundedReceiver<Msg>,
    effects: mpsc::UnboundedSender<Effect>,
}

impl Tui {
    fn start(dir: &Path) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (effects, effects_rx) = mpsc::unbounded_channel();
        tokio::spawn(discovery::run(
            dir.to_owned(),
            Options::new("must-tui test"),
            tx,
            effects_rx,
        ));
        Self {
            app: App::new(now(), None),
            rx,
            effects,
        }
    }

    fn apply(&mut self, msg: Msg) {
        for effect in self.app.update(msg) {
            self.effects.send(effect).unwrap();
        }
    }

    fn key(&mut self, code: KeyCode) {
        self.apply(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    /// Applies messages until `done` holds; panics after `within`.
    async fn pump(&mut self, within: Duration, what: &str, done: impl Fn(&App) -> bool) {
        let started = Instant::now();
        while !done(&self.app) {
            let left = within.saturating_sub(started.elapsed());
            match tokio::time::timeout(left, self.rx.recv()).await {
                Ok(Some(msg)) => self.apply(msg),
                _ => panic!("{what} not within {within:?}: {:#?}", self.app),
            }
        }
    }

    /// Applies what arrives within `duration`.
    async fn settle(&mut self, duration: Duration) {
        let end = tokio::time::Instant::now() + duration;
        while let Ok(Some(msg)) = tokio::time::timeout_at(end, self.rx.recv()).await {
            self.apply(msg);
        }
    }
}

/// One endpoint per process at a time: the registry directory of must-server is global,
/// and on Windows an endpoint's pipe name is made from the process id.
static SERIAL: Mutex<()> = Mutex::new(());

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_traffic_within_a_second() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = temp("traffic");
    must_server::monitor::set_registry_dir(Some(dir.clone()));
    let kelvin = DataInfo::double().unit("K").build().unwrap();
    let status = DataInfo::tuple(&[
        DataInfo::enumeration([("IDLE", 100)]).build().unwrap(),
        DataInfo::string().build().unwrap(),
    ])
    .build()
    .unwrap();
    let node = NodeBuilder::new("cryo", "a test node")
        .bind(([127, 0, 0, 1], 0).into())
        .log_files(LogFiles::off())
        .module(
            Module::new("t1", "a sensor")
                .interface_classes(["Readable"])
                .parameter(
                    Parameter::new("value", kelvin, "temperature")
                        .cached()
                        .initial_value(Value::Double(4.2)),
                )
                .unwrap()
                .parameter(
                    Parameter::new("status", status, "status")
                        .cached()
                        .initial_value(Value::Tuple(vec![
                            Value::Enum(100),
                            Value::String("ok".into()),
                        ])),
                )
                .unwrap(),
        )
        .unwrap()
        .build()
        .unwrap()
        .spawn(&tokio::runtime::Handle::current())
        .unwrap();

    let mut tui = Tui::start(&dir);
    tui.pump(Duration::from_secs(5), "node", |a| {
        a.selected_node().is_some()
    })
    .await;
    tui.key(KeyCode::Char('3')); // the traffic view subscribes
    tui.settle(Duration::from_millis(300)).await;

    let stream = tokio::net::TcpStream::connect(node.local_addr())
        .await
        .unwrap();
    let (reader, mut writer) = stream.into_split();
    writer.write_all(b"read t1:value\n").await.unwrap();
    let mut reply = String::new();
    BufReader::new(reader).read_line(&mut reply).await.unwrap();
    assert!(reply.starts_with("reply t1:value"), "{reply}");
    let sent = Instant::now();
    tui.pump(Duration::from_secs(1), "request and reply", |a| {
        let texts: Vec<_> = a.visible_traffic().iter().map(|l| l.text.clone()).collect();
        texts.iter().any(|t| t == "read t1:value")
            && texts.iter().any(|t| t.starts_with("reply t1:value"))
    })
    .await;
    println!("traffic shown after {:?}", sent.elapsed());
    node.stop().await;
    must_server::monitor::set_registry_dir(None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A fake endpoint registered in `dir` under this process; records every line it gets,
/// and answers the hello with `answer`.
fn fake_endpoint(
    dir: &Path,
    answer: Vec<ServerMessage>,
) -> (Arc<Mutex<Vec<String>>>, registry::RegistrationFile) {
    // on Windows the pipe name is made from the pid: the previous test's endpoint may
    // still be closing (its runtime is dropped after the test returns)
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut listener, endpoint) = loop {
        match transport::Listener::bind(dir) {
            Ok(bound) => break bound,
            Err(e) if Instant::now() < deadline => {
                println!("endpoint not free yet: {e}");
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("cannot open the fake endpoint: {e}"),
        }
    };
    let registration = Registration::new(&platform::current_process().unwrap(), endpoint);
    let file = registry::register(dir, &registration).unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&received);
    tokio::spawn(async move {
        while let Ok(stream) = listener.accept().await {
            let (reader, mut writer) = tokio::io::split(stream);
            let mut lines = BufReader::new(reader).lines();
            let record = Arc::clone(&record);
            let answer = answer.clone();
            tokio::spawn(async move {
                let mut answered = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    record.lock().unwrap().push(line);
                    if !answered {
                        answered = true;
                        for message in &answer {
                            let _ = writer.write_all(to_line(message).as_bytes()).await;
                        }
                    }
                }
            });
        }
    });
    (received, file)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incompatible_endpoint_shows_a_version_error() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = temp("version");
    let refusal = ServerMessage::Error {
        message: "unsupported protocol version 1.1; this endpoint speaks 2.0".into(),
    };
    let (_received, _file) = fake_endpoint(&dir, vec![refusal]);
    let mut tui = Tui::start(&dir);
    tui.pump(Duration::from_secs(5), "refusal", |a| {
        a.processes
            .values()
            .any(|p| matches!(p.link, Link::Refused(_)))
    })
    .await;
    let process = tui.app.processes.values().next().unwrap();
    assert!(process.nodes.is_empty(), "a version error instead of nodes");
    // not retried
    tui.settle(Duration::from_millis(2500)).await;
    assert!(matches!(
        tui.app.processes.values().next().unwrap().link,
        Link::Refused(_)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_key_only_reads() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = temp("readonly");
    let process = ProcessInfo {
        pid: std::process::id(),
        name: "fake".into(),
        library_version: "0.1.0".into(),
        start_time: now(),
    };
    let node = NodeInfo {
        id: 1,
        equipment_id: "cryo".into(),
        address: Some("127.0.0.1:2055".into()),
        state: NodeState::Running,
        context: None,
        messages: vec![],
        structure: Some(json!({"modules": {}})),
        parameters: vec![],
        clients: vec![],
        log_file: None,
    };
    let answer = vec![
        ServerMessage::Welcome {
            version: protocol_version(),
            process: process.clone(),
        },
        ServerMessage::Snapshot(Snapshot {
            process,
            nodes: vec![node],
            logs: vec![],
        }),
    ];
    let (received, _file) = fake_endpoint(&dir, answer);
    let mut tui = Tui::start(&dir);
    tui.pump(Duration::from_secs(5), "node", |a| {
        a.selected_node().is_some()
    })
    .await;

    // every key the help lists, in every view, and some others
    let mut keys = vec![
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Enter,
        KeyCode::Esc,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Delete,
        KeyCode::Insert,
        KeyCode::F(5),
    ];
    keys.extend("jkplnoP?xcsdwr".chars().map(KeyCode::Char));
    for view in ['1', '2', '3', '4'] {
        for key in &keys {
            tui.key(KeyCode::Char(view));
            tui.key(*key);
            if tui.app.help {
                tui.key(KeyCode::Esc);
            }
        }
        // a filter, typed and kept
        tui.key(KeyCode::Char(view));
        tui.key(KeyCode::Char('/'));
        for c in "change t1:target 0".chars() {
            tui.key(KeyCode::Char(c));
        }
        tui.key(KeyCode::Enter);
    }
    tui.key(KeyCode::Char('3'));
    tui.settle(Duration::from_millis(500)).await;

    let lines = received.lock().unwrap().clone();
    assert_ne!(lines, Vec::<String>::new());
    let mut subscriptions = 0;
    for line in &lines {
        match serde_json::from_str::<ClientMessage>(line) {
            Ok(ClientMessage::Hello { .. }) => {}
            Ok(ClientMessage::SubscribeTraffic { .. } | ClientMessage::SubscribeUpdates { .. }) => {
                subscriptions += 1;
            }
            other => panic!("the TUI sent {line:?}: {other:?}"),
        }
    }
    assert!(subscriptions > 0, "the traffic view subscribed: {lines:?}");
    assert!(!tui.app.quit);
    let _ = std::fs::remove_dir_all(&dir);
}
