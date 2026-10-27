// SPDX-License-Identifier: LGPL-3.0-or-later

//! Log files: a rolling on-disk record of every node's traffic and log records.
//!
//! Each running node writes `<log dir>/<node folder>/<YYYY-MM-DD>.log` (local date), where
//! the node folder is the `equipment_id` with characters other than ASCII letters,
//! digits, `-`, `_` and `.` replaced by `_`. The log directory defaults to
//! `$XDG_STATE_HOME/must/logs` or `~/.local/state/must/logs` on Linux (mode 0700) and
//! `%LOCALAPPDATA%\must\logs` on Windows. If another process already writes a node's file
//! of the day, this process writes `<YYYY-MM-DD>-<pid>.log`.
//!
//! A line looks like
//!
//! ```text
//! 2026-10-27T16:42:10.123+01:00 > [3 192.168.1.20:51234] change t1:target 300
//! ```
//!
//! with the kinds `>` (received from a client), `<` (sent to a client), `U` (a published
//! `update` or `error_update`, logged once however many clients receive it), `*`
//! (connections, start and stop, dropped lines) and `INFO`/`WARN`/`ERROR` (what the node
//! logs about itself, with the module when known).
//!
//! Files whose day ended more than [`LogFiles::days`] days ago (default 2) are deleted
//! when a node starts and every hour, in the folders of the nodes this process runs.
//!
//! File logging is on by default. The environment variables `MUST_LOG_FILES=0`,
//! `MUST_LOG_DIR` and `MUST_LOG_DAYS` configure it for the process, and
//! [`NodeBuilder::log_files`](crate::NodeBuilder::log_files) for one node.
//!
//! Logging never slows a node: lines go through a bounded queue to one writer thread per
//! process; when the queue is full, lines are dropped and a `* dropped <n> lines` line
//! marks the gap. Files are flushed every second and when the node stops. If a file
//! cannot be written, the node logs one warning and file logging stops for it until the
//! next day.

mod config;
mod format;
mod writer;

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use chrono::Local;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

pub use config::{
    DAYS_VARIABLE, DEFAULT_DAYS, DIR_VARIABLE, ENABLE_VARIABLE, LogFiles, default_dir,
};
#[doc(hidden)]
pub use writer::pause_writer;

use self::format::{LineFormatter, dropped, node_folder, timestamp};
use self::writer::{Command, LogId};
use crate::events::{LogLevel, NodeEvent};
use crate::runtime::Hub;

/// How long [`LogHandle::finished`] waits for the writer to close a stopped node's file.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// A node's file logging, for waiting until its file is closed.
#[derive(Clone, Debug)]
pub(crate) struct LogHandle {
    finished: CancellationToken,
}

impl LogHandle {
    /// Resolves once the node's file is flushed and closed after `NodeEvent::Stopped`
    /// (or after a few seconds if the writer is stuck).
    pub(crate) async fn finished(&self) {
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, self.finished.cancelled()).await;
    }
}

/// Starts file logging for a node whose hub was just created, before anything is emitted;
/// `None` if it is off. Must be called within a tokio runtime.
pub(crate) fn start(hub: &Arc<Hub>, config: Option<&LogFiles>) -> Option<LogHandle> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    static WARNED: Once = Once::new();
    let config = config.cloned().unwrap_or_else(LogFiles::from_env);
    if !config.enabled {
        return None;
    }
    let events = hub.events.subscribe();
    let Some(dir) = config.resolved_dir() else {
        hub.log(
            LogLevel::Warn,
            None,
            format!(
                "no log files: no home directory to put them in; set {DIR_VARIABLE} or {ENABLE_VARIABLE}=0"
            ),
        );
        return None;
    };
    let sender = writer::sender()?;
    let node = NEXT.fetch_add(1, Ordering::Relaxed);
    let finished = CancellationToken::new();
    let open = Command::Open {
        node,
        folder: dir.join(node_folder(&hub.model.equipment_id)),
        days: config.days,
        hub: Arc::downgrade(hub),
        today: Local::now().date_naive(),
    };
    tokio::spawn(run(events, sender, node, open, finished.clone()));

    WARNED.call_once(|| {
        for warning in config::env_warnings() {
            hub.log(LogLevel::Warn, None, warning.clone());
        }
    });
    Some(LogHandle { finished })
}

/// Hands `command` to the writer, waiting up to a few seconds while the queue is full.
async fn send(sender: &std::sync::mpsc::SyncSender<Command>, mut command: Command) -> bool {
    let deadline = tokio::time::Instant::now() + CLOSE_TIMEOUT;
    loop {
        match writer::try_send(sender, command) {
            Ok(()) => return true,
            Err(c) if tokio::time::Instant::now() < deadline => {
                command = c;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(_) => return false,
        }
    }
}

/// The node's logger: turns its events into lines for the writer, never waiting for it.
async fn run(
    mut events: tokio::sync::broadcast::Receiver<NodeEvent>,
    sender: &'static std::sync::mpsc::SyncSender<Command>,
    node: LogId,
    open: Command,
    finished: CancellationToken,
) {
    if send(sender, open).await {
        let mut formatter = LineFormatter::default();
        let mut lost: u64 = 0;
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(n)) => {
                    lost += n;
                    continue;
                }
                Err(RecvError::Closed) => break,
            };
            if let Some(text) = formatter.format(&event) {
                let now = Local::now();
                let time = timestamp(&now);
                let mut lines = String::with_capacity(text.len() + 40);
                if lost > 0 {
                    let _ = writeln!(lines, "{time} {}", dropped(lost));
                }
                lines.push_str(&time);
                lines.push(' ');
                lines.push_str(&text);
                lines.push('\n');
                let command = Command::Lines {
                    node,
                    date: now.date_naive(),
                    text: lines,
                };
                if matches!(event, NodeEvent::Stopped) {
                    // the last line, and the count of lines lost before it, must not be
                    // dropped; the node is stopping anyway
                    send(sender, command).await;
                    break;
                }
                match writer::try_send(sender, command) {
                    Ok(()) => lost = 0,
                    Err(_) => lost += 1,
                }
            }
        }
        let (ack, closed) = tokio::sync::oneshot::channel();
        if send(sender, Command::Close { node, ack }).await {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, closed).await;
        }
    }
    finished.cancel();
}
