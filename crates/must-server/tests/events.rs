// SPDX-License-Identifier: LGPL-3.0-or-later

//! Node events of a scripted session.

mod common;

use std::sync::Arc;

use common::{Client, Sim, node};
use must_server::{LogLevel, NodeEvent};
use tokio::runtime::Handle;

#[tokio::test]
async fn event_stream_of_a_session() {
    let n = node(Arc::new(Sim::default()))
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap();
    let mut events = n.subscribe();
    let mut c = Client::connect(n.local_addr()).await;
    c.request("read t1:value").await;
    drop(c);

    let mut seen = Vec::new();
    loop {
        let event = events.recv().await.unwrap();
        let done = matches!(event, NodeEvent::ClientDisconnected { .. });
        seen.push(event);
        if done {
            break;
        }
    }
    let kinds: Vec<&str> = seen
        .iter()
        .map(|e| match e {
            NodeEvent::ClientConnected { .. } => "connected",
            NodeEvent::MessageReceived { line, .. } if &**line == "read t1:value" => "received",
            NodeEvent::Updated {
                module,
                parameter,
                line,
            } if module == "t1"
                && parameter == "value"
                && line.starts_with("update t1:value [") =>
            {
                "updated"
            }
            NodeEvent::MessageSent { line, .. } if line.starts_with("reply t1:value") => "sent",
            NodeEvent::ClientDisconnected { reason, .. } if reason == "closed by client" => {
                "disconnected"
            }
            _ => "other",
        })
        .filter(|k| *k != "other")
        .collect();
    assert_eq!(
        kinds,
        ["connected", "received", "updated", "sent", "disconnected"]
    );
}

/// Collects events until `stop` says so, with a timeout.
async fn collect(
    events: &mut tokio::sync::broadcast::Receiver<NodeEvent>,
    mut stop: impl FnMut(&NodeEvent) -> bool,
) -> Vec<NodeEvent> {
    let mut seen = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            let done = stop(&event);
            seen.push(event);
            if done {
                break;
            }
        }
    })
    .await
    .expect("expected event");
    seen
}

#[tokio::test]
async fn activation_events() {
    let n = node(Arc::new(Sim::default()))
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap();
    let mut events = n.subscribe();
    let mut c = Client::connect(n.local_addr()).await;
    for request in ["activate t1", "deactivate t1", "activate"] {
        c.send(request).await;
        while !c.line().await.starts_with("active") && !request.starts_with("deactivate") {}
    }
    let seen = collect(&mut events, |e| {
        matches!(e, NodeEvent::ClientActivated { module: None, .. })
    })
    .await;
    let activation: Vec<_> = seen
        .iter()
        .filter_map(|e| match e {
            NodeEvent::ClientActivated { module, .. } => Some(("activated", module.clone())),
            NodeEvent::ClientDeactivated { module, .. } => Some(("deactivated", module.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        activation,
        [
            ("activated", Some("t1".to_owned())),
            ("deactivated", Some("t1".to_owned())),
            ("activated", None),
        ]
    );
}

#[tokio::test]
async fn one_updated_event_for_several_clients() {
    let n = node(Arc::new(Sim::default()))
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap();
    let mut a = Client::connect(n.local_addr()).await;
    let mut b = Client::connect(n.local_addr()).await;
    for c in [&mut a, &mut b] {
        c.send("activate t1").await;
        while c.line().await != "active t1" {}
    }
    let mut events = n.subscribe();
    n.update("t1", "value", must_types::Value::Double(7.5))
        .unwrap();
    let seen = collect(&mut events, |e| matches!(e, NodeEvent::Updated { .. })).await;
    let NodeEvent::Updated { line, .. } = seen.last().unwrap() else {
        unreachable!()
    };
    let a_line = loop {
        let l = a.line().await;
        if l.starts_with("update t1:value") {
            break l;
        }
    };
    let b_line = loop {
        let l = b.line().await;
        if l.starts_with("update t1:value") {
            break l;
        }
    };
    assert_eq!(&**line, a_line);
    assert_eq!(&**line, b_line);
    // exactly one Updated event for the one update
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut more = 0;
    while let Ok(e) = events.try_recv() {
        if matches!(e, NodeEvent::Updated { ref parameter, .. } if parameter == "value") {
            more += 1;
        }
    }
    assert_eq!(more, 0);
}

#[tokio::test]
async fn failing_read_is_logged_for_its_module() {
    let n = node(Arc::new(Sim::default()))
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap();
    let mut events = n.subscribe();
    let mut c = Client::connect(n.local_addr()).await;
    c.request("read t1:broken").await;
    let seen = collect(&mut events, |e| matches!(e, NodeEvent::Log { .. })).await;
    let NodeEvent::Log {
        level,
        module,
        message,
    } = seen.last().unwrap()
    else {
        unreachable!()
    };
    assert_eq!(*level, LogLevel::Warn);
    assert_eq!(module.as_deref(), Some("t1"));
    assert!(message.contains("broken"), "{message}");
}
