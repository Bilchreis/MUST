// SPDX-License-Identifier: LGPL-3.0-or-later

//! Tracing output of a scripted session.
//!
//! This is the only test in its binary: tracing caches per call site whether anyone
//! listens, so a concurrent test without a subscriber could hide events from this one.

mod common;

use std::sync::{Arc, Mutex};

use common::{Client, Sim, node};
use must_server::NodeEvent;
use must_types::Value;
use tokio::runtime::Handle;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

/// Records `level message` of every tracing event.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for Recorder {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap()
            .push(format!("{} {}", event.metadata().level(), visitor.0));
    }
}

// a current-thread runtime keeps all node tasks on this thread, where the recorder is
// the default subscriber
#[tokio::test(flavor = "current_thread")]
async fn scripted_session() {
    let recorder = Recorder::default();
    let subscriber = tracing_subscriber::registry().with(recorder.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let n = node(Arc::new(Sim::default()))
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap();
    let mut events = n.subscribe();
    let mut c = Client::connect(n.local_addr()).await;
    assert!(
        c.request("read t1:value")
            .await
            .starts_with("reply t1:value")
    );
    n.update("t2", "value", Value::Double(5.0)).unwrap();
    drop(c);
    // wait until the node noticed the disconnect
    loop {
        if let NodeEvent::ClientDisconnected { .. } = events.recv().await.unwrap() {
            break;
        }
    }
    n.stop().await;

    let mut stopped = false;
    while let Ok(event) = events.try_recv() {
        stopped |= event == NodeEvent::Stopped;
    }
    assert!(stopped, "no Stopped event");

    let logs = recorder.0.lock().unwrap().clone();
    let has = |level: Level, text: &str| {
        logs.iter()
            .any(|l| l.starts_with(&level.to_string()) && l.contains(text))
    };
    assert!(has(Level::INFO, "node started"), "{logs:#?}");
    assert!(has(Level::INFO, "client connected"), "{logs:#?}");
    assert!(has(Level::DEBUG, "< read t1:value"), "{logs:#?}");
    assert!(has(Level::DEBUG, "> reply t1:value"), "{logs:#?}");
    assert!(has(Level::DEBUG, "updated"), "{logs:#?}");
    assert!(has(Level::INFO, "client disconnected"), "{logs:#?}");
    assert!(has(Level::INFO, "node stopped"), "{logs:#?}");
}
