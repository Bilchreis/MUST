// SPDX-License-Identifier: LGPL-3.0-or-later

//! The module start hook: it runs before any request reaches the handler, and a failure or
//! hang is logged without taking the module out of the node.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::{Client, kelvin, parse, status, status_info};
use must_server::logfiles::LogFiles;
use must_server::{
    Ctx, Discovery, HandlerResult, LogLevel, Module, ModuleHandler, NodeBuilder, NodeEvent,
    Parameter, Reading, RunningNode,
};
use must_types::{ErrorClass, SecopError, Value};
use tokio::runtime::Handle;
use tokio::sync::broadcast;

#[derive(Default)]
struct Starter {
    delay: Duration,
    fail: bool,
    hang: bool,
    started: AtomicBool,
    reads_before_start: AtomicUsize,
}

#[async_trait]
impl ModuleHandler for Starter {
    async fn start(&self, _ctx: &Ctx) -> Result<(), SecopError> {
        if self.hang {
            std::future::pending::<()>().await;
        }
        tokio::time::sleep(self.delay).await;
        self.started.store(true, Ordering::SeqCst);
        if self.fail {
            return Err(SecopError::new(
                ErrorClass::HardwareError,
                "controller not found",
            ));
        }
        Ok(())
    }

    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        if !self.started.load(Ordering::SeqCst) {
            self.reads_before_start.fetch_add(1, Ordering::SeqCst);
        }
        Ok(match parameter {
            "value" => Reading::new(Value::Double(4.2)),
            _ => Reading::new(status(100, "ok")),
        })
    }
}

fn start(handler: Arc<Starter>, timeout: f64) -> RunningNode {
    let module = Module::new("T", "a module with a start hook")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "value"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .no_polling()
        .handler_arc(handler);
    NodeBuilder::new("must-test", "start hook test node")
        .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .timeout(timeout)
        .log_files(LogFiles::off())
        .discovery(Discovery::off())
        .module(module)
        .unwrap()
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

/// The next log message of module `T`, waiting at most `within`.
async fn log_of_t(events: &mut broadcast::Receiver<NodeEvent>, within: Duration) -> String {
    tokio::time::timeout(within, async {
        loop {
            if let NodeEvent::Log {
                level,
                module,
                message,
            } = events.recv().await.unwrap()
                && module.as_deref() == Some("T")
            {
                assert_eq!(level, LogLevel::Error);
                return message;
            }
        }
    })
    .await
    .expect("no log message of T")
}

#[tokio::test]
async fn read_waits_for_start() {
    let handler = Arc::new(Starter {
        delay: Duration::from_millis(500),
        ..Starter::default()
    });
    let begin = Instant::now();
    let n = start(Arc::clone(&handler), 10.0);
    let mut c = Client::connect(n.local_addr()).await;
    let reply = c.request("read T:value").await;
    let (action, specifier, data) = parse(&reply);
    assert_eq!((action.as_str(), specifier.as_str()), ("reply", "T:value"));
    assert_eq!(data[0], 4.2);
    assert!(begin.elapsed() >= Duration::from_millis(500));
    // neither the client's read nor the startup reads ran before the hook finished
    assert_eq!(handler.reads_before_start.load(Ordering::SeqCst), 0);
    n.stop().await;
}

#[tokio::test]
async fn start_hook_fails() {
    let handler = Arc::new(Starter {
        fail: true,
        delay: Duration::from_millis(200),
        ..Starter::default()
    });
    let n = start(handler, 10.0);
    let mut events = n.subscribe();
    let message = log_of_t(&mut events, Duration::from_secs(5)).await;
    assert!(message.contains("controller not found"), "{message}");
    let mut c = Client::connect(n.local_addr()).await;
    assert!(c.request("read T:value").await.starts_with("reply T:value"));
    n.stop().await;
}

#[tokio::test]
async fn hanging_start_hook() {
    let handler = Arc::new(Starter {
        hang: true,
        ..Starter::default()
    });
    let begin = Instant::now();
    // a node timeout of 0.1 s gives the start hook 1 s
    let n = start(handler, 0.1);
    let mut events = n.subscribe();
    let message = log_of_t(&mut events, Duration::from_secs(5)).await;
    assert!(
        message.contains("TimeoutError") || message.contains("no answer"),
        "{message}"
    );
    let elapsed = begin.elapsed();
    assert!(elapsed >= Duration::from_millis(900), "{elapsed:?}");
    let mut c = Client::connect(n.local_addr()).await;
    assert!(c.request("read T:value").await.starts_with("reply T:value"));
    n.stop().await;
}
