// SPDX-License-Identifier: LGPL-3.0-or-later

//! Node events of a scripted session.

mod common;

use std::sync::Arc;

use common::{Client, Sim, node};
use must_server::NodeEvent;
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
            NodeEvent::Updated { module, parameter } if module == "t1" && parameter == "value" => {
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
