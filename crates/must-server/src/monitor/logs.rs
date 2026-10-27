// SPDX-License-Identifier: LGPL-3.0-or-later

//! Captures `tracing` records of the process for the monitoring endpoint.
//!
//! The layer never blocks and never takes the endpoint's locks: it hands each record to
//! a broadcast channel, from which the endpoint's own task adds it to the log buffer and
//! the monitors' queues. Records of nodes about themselves (target
//! [`NODE_LOG_TARGET`](crate::runtime::NODE_LOG_TARGET)) are skipped, since they reach the
//! endpoint with their node as [`NodeEvent::Log`](crate::NodeEvent::Log).

use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};

use must_monitor::protocol::LogRecord;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};

use super::Shared;
use crate::protocol::now;
use crate::runtime::NODE_LOG_TARGET;

/// Records waiting for the endpoint's task; old ones are dropped if it falls behind.
static RECORDS: LazyLock<broadcast::Sender<LogRecord>> =
    LazyLock::new(|| broadcast::channel(1024).0);

/// Least severe level captured.
const CAPTURED: Level = Level::INFO;

/// A `tracing_subscriber` layer that sends the process's log records (info and more
/// severe) to the monitoring endpoint. Add it to your subscriber, e.g.
/// `tracing_subscriber::registry().with(must_server::monitor::log_layer())`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogLayer;

/// The layer capturing log records for monitoring; see [`LogLayer`].
pub fn log_layer() -> LogLayer {
    LogLayer
}

/// Installs a global `tracing` subscriber with only [`LogLayer`], for hosts that have
/// none (such as C and LabVIEW programs using the SHALL C ABI). Does nothing if the
/// process already has a global subscriber.
pub fn install_log_capture() {
    let subscriber = tracing_subscriber::registry().with(LogLayer);
    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn captured(metadata: &Metadata<'_>) -> bool {
    *metadata.level() <= CAPTURED && metadata.target() != NODE_LOG_TARGET
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if captured(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        captured(metadata)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::from_level(CAPTURED))
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if RECORDS.receiver_count() == 0 || !captured(event.metadata()) {
            return;
        }
        let mut text = Text::default();
        event.record(&mut text);
        let mut message = text.message;
        if !text.fields.is_empty() {
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(&text.fields);
        }
        let _ = RECORDS.send(LogRecord {
            time: now(),
            level: event.metadata().level().as_str().to_owned(),
            node: None,
            module: None,
            message,
        });
    }
}

/// The message and the other fields of a record as `key=value`.
#[derive(Default)]
struct Text {
    message: String,
    fields: String,
}

impl Visit for Text {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.add(field, &value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            self.add(field, value);
        }
    }
}

impl Text {
    fn add(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(self.fields, "{}={value:?}", field.name());
    }
}

/// The endpoint's task that moves captured records into the log buffer and to monitors.
pub(super) async fn forward(shared: Arc<Shared>, shutdown: CancellationToken) {
    let mut records = RECORDS.subscribe();
    loop {
        let record = tokio::select! {
            () = shutdown.cancelled() => break,
            record = records.recv() => record,
        };
        match record {
            Ok(record) => shared.lock().log(record),
            Err(RecvError::Lagged(n)) => {
                let message = format!("{n} log records were dropped");
                shared.lock().log(LogRecord {
                    time: now(),
                    level: "WARN".to_owned(),
                    node: None,
                    module: None,
                    message,
                });
            }
            Err(RecvError::Closed) => break,
        }
    }
}
