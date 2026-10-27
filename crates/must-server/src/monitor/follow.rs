// SPDX-License-Identifier: LGPL-3.0-or-later

//! Turns a node's [`NodeEvent`]s into monitoring events, and sends message counters.

use std::sync::Arc;
use std::time::Duration;

use must_monitor::protocol::{
    ClientInfo, Direction, Event, LogRecord, NodeId, NodeState, ParameterState, ServerMessage,
    truncate_text,
};
use serde_json::Value as Json;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;

use super::{Inner, Shared};
use crate::events::NodeEvent;
use crate::protocol::now;

/// Follows one node's events until its channel closes or the task is aborted.
pub(super) async fn follow(
    shared: Arc<Shared>,
    node: NodeId,
    mut events: broadcast::Receiver<NodeEvent>,
) {
    loop {
        match events.recv().await {
            Ok(event) => handle(&mut shared.lock(), node, event),
            Err(RecvError::Lagged(n)) => {
                // the monitor itself fell behind the node: tell every monitor
                shared
                    .lock()
                    .broadcast(&ServerMessage::Lagged { dropped: n });
            }
            Err(RecvError::Closed) => break,
        }
    }
}

fn handle(inner: &mut Inner, node: NodeId, event: NodeEvent) {
    let Some(entry) = inner.nodes.get_mut(&node) else {
        return;
    };
    match event {
        NodeEvent::Started { addr } => entry.address = Some(addr.to_string()),
        NodeEvent::Stopped => {
            entry.state = NodeState::Stopped;
            entry.clients.clear();
            inner.event(Event::NodeStopped { node });
        }
        NodeEvent::ClientConnected { id, peer } => {
            let client = ClientInfo {
                id,
                peer: peer.to_string(),
                connected_at: now(),
                active_all: false,
                active_modules: Vec::new(),
            };
            entry.clients.insert(id, client.clone());
            inner.event(Event::ClientConnected { node, client });
        }
        NodeEvent::ClientDisconnected { id, reason } => {
            entry.clients.shift_remove(&id);
            inner.event(Event::ClientDisconnected {
                node,
                client: id,
                reason,
            });
        }
        NodeEvent::ClientActivated { id, module } => {
            if let Some(client) = entry.clients.get_mut(&id) {
                match &module {
                    None => client.active_all = true,
                    Some(m) if !client.active_modules.contains(m) => {
                        client.active_modules.push(m.clone());
                    }
                    Some(_) => {}
                }
            }
            inner.event(Event::ClientActivated {
                node,
                client: id,
                module,
            });
        }
        NodeEvent::ClientDeactivated { id, module } => {
            if let Some(client) = entry.clients.get_mut(&id) {
                match &module {
                    None => {
                        client.active_all = false;
                        client.active_modules.clear();
                    }
                    Some(m) => client.active_modules.retain(|x| x != m),
                }
            }
            inner.event(Event::ClientDeactivated {
                node,
                client: id,
                module,
            });
        }
        NodeEvent::MessageReceived { id, line } => {
            entry.rx += 1;
            message(inner, node, id, Direction::Received, &line);
        }
        NodeEvent::MessageSent { id, line } => {
            entry.tx += 1;
            message(inner, node, id, Direction::Sent, &line);
        }
        NodeEvent::Updated {
            module,
            parameter,
            line,
        } => {
            let state = parse_update(&module, &parameter, &line);
            for peer in &inner.peers {
                peer.update(node, state.clone());
            }
        }
        NodeEvent::Log {
            level,
            module,
            message,
        } => inner.log(LogRecord {
            time: now(),
            level: level.as_str().to_owned(),
            node: Some(node),
            module,
            message,
        }),
    }
}

/// Sends a message event to the monitors that subscribed to the node's traffic; the text
/// is copied only if there is one.
fn message(inner: &Inner, node: NodeId, client: u64, direction: Direction, line: &str) {
    let mut event = None;
    for peer in &inner.peers {
        if peer.wants_traffic(node) {
            let event = event.get_or_insert_with(|| {
                let (text, truncated) = truncate_text(line);
                ServerMessage::Event(Event::Message {
                    node,
                    client,
                    direction,
                    time: now(),
                    text,
                    truncated,
                })
            });
            peer.push(event.clone());
        }
    }
}

/// The parameter state of an `update`/`error_update` line.
fn parse_update(module: &str, parameter: &str, line: &str) -> ParameterState {
    let error = line.starts_with("error_update");
    let report = line
        .splitn(3, ' ')
        .nth(2)
        .and_then(|data| serde_json::from_str(data).ok())
        .unwrap_or(Json::Null);
    ParameterState {
        module: module.to_owned(),
        parameter: parameter.to_owned(),
        report,
        error,
    }
}

/// Sends every node's message counters to all monitors once per second.
pub(super) async fn counters(shared: Arc<Shared>, shutdown: CancellationToken) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = ticker.tick() => {}
        }
        let mut inner = shared.lock();
        let mut messages = Vec::new();
        for (id, entry) in &mut inner.nodes {
            if entry.state != NodeState::Running {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "counts per second")]
            messages.push(ServerMessage::Counters {
                node: *id,
                rx: entry.rx,
                tx: entry.tx,
                rx_per_second: (entry.rx - entry.last_rx) as f64,
                tx_per_second: (entry.tx - entry.last_tx) as f64,
            });
            entry.last_rx = entry.rx;
            entry.last_tx = entry.tx;
        }
        for message in &messages {
            inner.broadcast(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_lines() {
        let state = parse_update("t1", "value", r#"update t1:value [4.2,{"t":1.5}]"#);
        assert_eq!(state.report, serde_json::json!([4.2, {"t": 1.5}]));
        assert!(!state.error);
        let state = parse_update(
            "t1",
            "value",
            r#"error_update t1:value ["HardwareError","broken",{"t":1.5}]"#,
        );
        assert!(state.error);
        assert_eq!(state.report[0], "HardwareError");
    }
}
