// SPDX-License-Identifier: LGPL-3.0-or-later

//! The scenarios of "Log events", "Log events never disconnect" and "Application log
//! records" in specs/sec-node-remote-logging, over loopback TCP. The log file part of
//! "Handler logs" and "Debug record" is in `tests/log_files.rs`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::{Client, Sim, kelvin, node, status_info};
use must_server::{
    AppLogLevel, CallResult, Command, Ctx, HandlerResult, Module, ModuleHandler, NodeBuilder,
    NodeEvent, Parameter, RunningNode,
};
use must_types::{DataInfo, ErrorClass, SecopError, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpSocket;
use tokio::runtime::Handle;

const MIB: usize = 1 << 20;

/// Logs through its [`Ctx`]: `do chatty:say` logs one debug and one info message; reads
/// fail as if the controller did not answer.
struct Chatty;

#[async_trait]
impl ModuleHandler for Chatty {
    async fn read(&self, _ctx: &Ctx, _parameter: &str) -> HandlerResult {
        Err(SecopError::new(
            ErrorClass::CommunicationFailed,
            "no answer from controller",
        ))
    }

    async fn call(&self, ctx: &Ctx, _command: &str, _argument: Option<Value>) -> CallResult {
        ctx.log(AppLogLevel::Debug, "relay coil at 12 V");
        ctx.log(AppLogLevel::Info, "heater relay switched");
        Ok(None)
    }
}

/// A Readable module `chatty` with the [`Chatty`] handler, polled every `poll` if given.
fn chatty(poll: Option<Duration>) -> Module {
    let module = Module::new("chatty", "logs a lot")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .command(Command::new(
            "say",
            DataInfo::command().build().unwrap(),
            "logs",
        ))
        .unwrap()
        .handler(Chatty);
    match poll {
        Some(interval) => module.poll_interval(interval),
        None => module.no_polling(),
    }
}

fn start(builder: NodeBuilder) -> RunningNode {
    builder.build().unwrap().spawn(&Handle::current()).unwrap()
}

/// Lines received until none arrives for a moment.
async fn drain(c: &mut Client) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(line) = c.try_line(Duration::from_millis(300)).await {
        lines.push(line);
    }
    lines
}

#[tokio::test]
async fn poll_failure() {
    let n = start(
        node(Arc::new(Sim::default()))
            .module(chatty(Some(Duration::from_millis(100))))
            .unwrap(),
    );
    let mut c = Client::connect(n.local_addr()).await;
    c.request(r#"logging chatty "error""#).await;
    let line = c.line().await;
    assert!(line.starts_with("log chatty:error \""), "{line}");
    assert!(line.contains("no answer from controller"), "{line}");
}

#[tokio::test]
async fn level_filters() {
    let n = start(node(Arc::new(Sim::default())).module(chatty(None)).unwrap());
    let mut c = Client::connect(n.local_addr()).await;
    c.request(r#"logging chatty "info""#).await;
    c.send("do chatty:say").await;
    assert_eq!(c.line().await, r#"log chatty:info "heater relay switched""#);
    assert!(c.line().await.starts_with("done chatty:say "));
}

#[tokio::test]
async fn debug_record_reaches_clients_only() {
    let n = start(node(Arc::new(Sim::default())).module(chatty(None)).unwrap());
    let mut events = n.subscribe();
    let mut c = Client::connect(n.local_addr()).await;
    c.request(r#"logging chatty "debug""#).await;
    c.send("do chatty:say").await;
    assert_eq!(c.line().await, r#"log chatty:debug "relay coil at 12 V""#);
    assert_eq!(c.line().await, r#"log chatty:info "heater relay switched""#);
    assert!(c.line().await.starts_with("done chatty:say "));
    // the debug record is no node event: the first log event is the info record
    let message = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let NodeEvent::Log { message, .. } = events.recv().await.unwrap() {
                return message;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(message, "heater relay switched");
}

#[tokio::test]
async fn only_requesting_connections() {
    let n = start(node(Arc::new(Sim::default())));
    let mut a = Client::connect(n.local_addr()).await;
    let mut b = Client::connect(n.local_addr()).await;
    a.request(r#"logging  "debug""#).await;
    b.send("activate").await;
    while b.line().await != "active" {}
    n.log("t1", AppLogLevel::Info, "cooling started").unwrap();
    assert_eq!(a.line().await, r#"log t1:info "cooling started""#);
    assert_eq!(drain(&mut b).await, Vec::<String>::new());
}

#[tokio::test]
async fn records_without_module() {
    let n = start(node(Arc::new(Sim::default())));
    let mut events = n.subscribe();
    let mut c = Client::connect(n.local_addr()).await;
    c.request(r#"logging  "debug""#).await;
    // a panicking handler is logged without module (and its failed read for `t1`)
    let reply = c.request("read t1:panic").await;
    let mut lines = vec![reply];
    lines.extend(drain(&mut c).await);
    let panicked = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let NodeEvent::Log {
                module, message, ..
            } = events.recv().await.unwrap()
                && message.contains("panicked")
                && module.is_none()
            {
                return true;
            }
        }
    })
    .await
    .unwrap();
    assert!(panicked);
    // only the failed read of `t1` is sent, not the record without module
    let logs: Vec<_> = lines.iter().filter(|l| l.starts_with("log ")).collect();
    assert_eq!(logs.len(), 1, "{lines:#?}");
    assert!(
        logs[0].starts_with("log t1:error \"reading `panic` failed: "),
        "{lines:#?}"
    );
}

#[tokio::test]
async fn running_node_log_unknown_module() {
    let n = start(node(Arc::new(Sim::default())));
    assert!(n.log("nope", AppLogLevel::Info, "x").is_err());
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_client_with_logging() {
    let n = start(node(Arc::new(Sim::default())).max_outgoing_bytes(MIB));
    let mut events = n.subscribe();
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let mut stuck = socket.connect(n.local_addr()).await.unwrap();
    stuck.write_all(b"logging  \"debug\"\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // far more than the limit and what the kernel buffers absorb
    let chunk = "x".repeat(64 * 1024);
    let count = 1024;
    for i in 0..count {
        n.log("t1", AppLogLevel::Debug, format!("{i} {chunk}"))
            .unwrap();
        if i % 16 == 0 {
            tokio::task::yield_now().await;
        }
    }
    // a reply still gets through
    stuck.write_all(b"ping after\n").await.unwrap();

    let mut reader = BufReader::new(stuck);
    let mut line = String::new();
    let mut logs = 0;
    loop {
        line.clear();
        let read = tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line))
            .await
            .expect("the connection stays open")
            .unwrap();
        assert_ne!(read, 0, "the connection was closed");
        if line.starts_with("log t1:debug ") {
            logs += 1;
        } else if line.starts_with("pong after ") {
            break;
        }
    }
    assert!(logs > 0, "log events are sent");
    assert!(logs < count, "log events over the limit are dropped");
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, NodeEvent::ClientDisconnected { .. }),
            "{event:?}"
        );
    }
}
