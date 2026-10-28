// SPDX-License-Identifier: LGPL-3.0-or-later

//! Finds the user's monitoring endpoints and keeps one connection to each.
//!
//! The registry directory is listed every [`RESCAN`]; a new live registration gets a
//! connection task, which reports to the app as [`Msg::Endpoint`]. A connection that
//! failed is tried again at the next rescan; one the endpoint refused (e.g. another
//! protocol major version) is not.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use must_monitor::client::{self, ClientError, Options};
use must_monitor::protocol::NodeId;
use must_monitor::registry::{EndpointStatus, Registration};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::{Effect, EndpointMsg, Msg, ProcessKey};

/// How often the registry directory is listed.
pub const RESCAN: Duration = Duration::from_secs(1);

/// A request for one connection.
#[derive(Debug, Clone, Copy)]
struct Traffic {
    node: NodeId,
    enabled: bool,
}

/// How a connection task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Connecting failed; retry.
    Failed,
    /// Refused by the endpoint; do not retry.
    Refused,
    /// The connection ended; the process may connect again later.
    Closed,
}

struct Link {
    requests: mpsc::UnboundedSender<Traffic>,
    task: JoinHandle<Outcome>,
}

/// Discovers and follows endpoints in `dir` until `effects` closes, reporting to `tx`.
pub async fn run(
    dir: PathBuf,
    options: Options,
    tx: mpsc::UnboundedSender<Msg>,
    mut effects: mpsc::UnboundedReceiver<Effect>,
) {
    let mut links: HashMap<ProcessKey, Link> = HashMap::new();
    let mut refused: HashSet<ProcessKey> = HashSet::new();
    let mut rescan = tokio::time::interval(RESCAN);
    rescan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = rescan.tick() => {
                scan(&dir, &options, &tx, &mut links, &mut refused).await;
            }
            effect = effects.recv() => match effect {
                Some(Effect::Traffic { key, node, enabled }) => {
                    if let Some(link) = links.get(&key) {
                        let _ = link.requests.send(Traffic { node, enabled });
                    }
                }
                None => break,
            },
        }
    }
    for link in links.into_values() {
        link.task.abort();
    }
}

async fn scan(
    dir: &std::path::Path,
    options: &Options,
    tx: &mpsc::UnboundedSender<Msg>,
    links: &mut HashMap<ProcessKey, Link>,
    refused: &mut HashSet<ProcessKey>,
) {
    // a missing or unreadable directory lists nothing
    let listed: Vec<Registration> = client::list_endpoints_in(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.status == EndpointStatus::Live)
        .map(|l| l.registration)
        .collect();
    let live: HashSet<ProcessKey> = listed.iter().map(|r| (r.pid, r.start)).collect();

    // finished connection tasks
    let finished: Vec<ProcessKey> = links
        .iter()
        .filter(|(_, link)| link.task.is_finished())
        .map(|(key, _)| *key)
        .collect();
    for key in finished {
        if let Some(link) = links.remove(&key)
            && let Ok(Outcome::Refused) = link.task.await
        {
            refused.insert(key);
        }
    }
    // processes whose registration disappeared while connected
    let vanished: Vec<ProcessKey> = links
        .keys()
        .filter(|key| !live.contains(key))
        .copied()
        .collect();
    for key in vanished {
        if let Some(link) = links.remove(&key) {
            link.task.abort();
            let _ = tx.send(Msg::Endpoint {
                key,
                msg: EndpointMsg::Closed,
            });
        }
    }
    // new ones
    for registration in listed {
        let key = (registration.pid, registration.start);
        if links.contains_key(&key) || refused.contains(&key) {
            continue;
        }
        let _ = tx.send(Msg::Endpoint {
            key,
            msg: EndpointMsg::Connecting {
                name: registration.name.clone(),
            },
        });
        let (requests, request_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(follow(
            registration,
            options.clone(),
            key,
            tx.clone(),
            request_rx,
        ));
        links.insert(key, Link { requests, task });
    }
}

/// One connection: connects, forwards everything to the app, sends requests.
async fn follow(
    registration: Registration,
    options: Options,
    key: ProcessKey,
    tx: mpsc::UnboundedSender<Msg>,
    mut requests: mpsc::UnboundedReceiver<Traffic>,
) -> Outcome {
    let send = |msg| {
        let _ = tx.send(Msg::Endpoint { key, msg });
    };
    let connection = match tokio::time::timeout(
        Duration::from_secs(5),
        client::connect(&registration, &options),
    )
    .await
    {
        Ok(Ok(connection)) => connection,
        Ok(Err(ClientError::Refused(reason))) => {
            send(EndpointMsg::Refused(reason));
            return Outcome::Refused;
        }
        Ok(Err(e)) => {
            send(EndpointMsg::Failed(e.to_string()));
            return Outcome::Failed;
        }
        Err(_) => {
            send(EndpointMsg::Failed("no answer within 5 s".into()));
            return Outcome::Failed;
        }
    };
    let (process, snapshot, mut events, mut writer) = connection.into_parts();
    send(EndpointMsg::Connected { process, snapshot });

    // reading in its own task: a read must not be cancelled halfway through a line
    let reader_tx = tx.clone();
    let mut reader = tokio::spawn(async move {
        while let Ok(Some(message)) = events.next().await {
            if reader_tx
                .send(Msg::Endpoint {
                    key,
                    msg: EndpointMsg::Message(message),
                })
                .is_err()
            {
                break;
            }
        }
    });
    loop {
        tokio::select! {
            _ = &mut reader => break,
            request = requests.recv() => {
                let Some(Traffic { node, enabled }) = request else {
                    reader.abort();
                    break;
                };
                let _ = writer.subscribe_traffic(node, enabled).await;
            }
        }
    }
    send(EndpointMsg::Closed);
    Outcome::Closed
}
