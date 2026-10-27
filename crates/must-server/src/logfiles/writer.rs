// SPDX-License-Identifier: LGPL-3.0-or-later

//! The process's writer thread: all file I/O of file logging happens here.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate};

use super::format::{day_file, parse_day_file};
use crate::events::LogLevel;
use crate::runtime::Hub;

/// Lines waiting for the writer, for all nodes together.
const QUEUE: usize = 8192;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const CLEAN_INTERVAL: Duration = Duration::from_secs(3600);

/// Identifies a logging node at the writer.
pub(crate) type LogId = u64;

/// A request to the writer.
pub(crate) enum Command {
    /// A node starts logging into `folder`; old files are deleted.
    Open {
        node: LogId,
        folder: PathBuf,
        days: u32,
        /// For warnings about the node's log.
        hub: Weak<Hub>,
        today: NaiveDate,
    },
    /// Text (one or more complete lines) for the file of `date`.
    Lines {
        node: LogId,
        date: NaiveDate,
        text: String,
    },
    /// The node stopped: flush and close its file, then acknowledge.
    Close {
        node: LogId,
        ack: tokio::sync::oneshot::Sender<()>,
    },
}

/// The sender to the writer thread, started on first use; `None` if it cannot start.
pub(crate) fn sender() -> Option<&'static SyncSender<Command>> {
    static SENDER: OnceLock<Option<SyncSender<Command>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (tx, rx) = sync_channel(QUEUE);
            match std::thread::Builder::new()
                .name("must-log-writer".into())
                .spawn(move || run(&rx))
            {
                Ok(_) => Some(tx),
                Err(e) => {
                    tracing::warn!("cannot start the log writer thread, no log files: {e}");
                    None
                }
            }
        })
        .as_ref()
}

/// Hands `command` to the writer without blocking; gives it back if the queue is full
/// or the writer is gone.
pub(crate) fn try_send(sender: &SyncSender<Command>, command: Command) -> Result<(), Command> {
    sender.try_send(command).map_err(|e| match e {
        TrySendError::Full(c) | TrySendError::Disconnected(c) => c,
    })
}

static PAUSE: Mutex<()> = Mutex::new(());

/// Stops the writer thread before its next command while the guard lives (tests only).
#[doc(hidden)]
pub fn pause_writer() -> MutexGuard<'static, ()> {
    PAUSE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn run(rx: &Receiver<Command>) {
    let mut writer = Writer::new(std::process::id());
    let mut next_flush = Instant::now() + FLUSH_INTERVAL;
    let mut next_clean = Instant::now() + CLEAN_INTERVAL;
    loop {
        match rx.recv_timeout(next_flush.saturating_duration_since(Instant::now())) {
            Ok(command) => {
                drop(PAUSE.lock());
                writer.handle(command);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let now = Instant::now();
        if now >= next_flush {
            writer.flush_all();
            next_flush = now + FLUSH_INTERVAL;
        }
        if now >= next_clean {
            writer.clean_all(Local::now().date_naive());
            next_clean = now + CLEAN_INTERVAL;
        }
    }
}

/// The open file of a node.
struct OpenFile {
    date: NaiveDate,
    path: PathBuf,
    out: BufWriter<File>,
}

/// One logging node.
struct NodeLog {
    folder: PathBuf,
    days: u32,
    hub: Weak<Hub>,
    file: Option<OpenFile>,
    /// Logging failed for this day; retried with the next day's lines.
    failed: Option<NaiveDate>,
}

impl NodeLog {
    fn warn(&self, message: String) {
        if let Some(hub) = self.hub.upgrade() {
            hub.log(LogLevel::Warn, None, message);
        } else {
            tracing::warn!("{message}");
        }
    }

    fn fail(&mut self, date: NaiveDate, message: &str) {
        self.file = None;
        self.failed = Some(date);
        self.warn(format!("{message}; file logging stops until the next day"));
    }

    fn write(&mut self, pid: u32, date: NaiveDate, text: &str) {
        if self.failed == Some(date) {
            return;
        }
        if self.file.as_ref().is_none_or(|f| f.date != date) {
            if let Some(mut old) = self.file.take() {
                // closing the previous day's file
                let _ = old.out.flush();
            }
            match self.open(pid, date) {
                Ok(file) => {
                    self.file = Some(file);
                    self.failed = None;
                }
                Err(message) => return self.fail(date, &message),
            }
        }
        if let Some(file) = &mut self.file
            && let Err(e) = file.out.write_all(text.as_bytes())
        {
            let message = format!("cannot write log file {}: {e}", file.path.display());
            self.fail(date, &message);
        }
    }

    fn open(&self, pid: u32, date: NaiveDate) -> Result<OpenFile, String> {
        create_dir(&self.folder)
            .map_err(|e| format!("cannot create log directory {}: {e}", self.folder.display()))?;
        let mut path = self.folder.join(day_file(date, None));
        let file = match open_locked(&path) {
            Ok(Some(file)) => file,
            // another process writes this node's file today
            Ok(None) => {
                path = self.folder.join(day_file(date, Some(pid)));
                match open_locked(&path) {
                    Ok(Some(file)) => file,
                    Ok(None) => return Err(format!("log file {} is in use", path.display())),
                    Err(e) => return Err(format!("cannot open log file {}: {e}", path.display())),
                }
            }
            Err(e) => return Err(format!("cannot open log file {}: {e}", path.display())),
        };
        Ok(OpenFile {
            date,
            path,
            out: BufWriter::with_capacity(64 * 1024, file),
        })
    }
}

/// Opens `path` for appending with an exclusive lock; `None` if someone holds the lock.
fn open_locked(path: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Creates `dir` and its parents, readable only by the user on Unix.
fn create_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Deletes the log files in `folder` older than `days` days before `today`; leaves files
/// with other names alone. Returns the deleted names (for tests).
pub(crate) fn clean(folder: &Path, today: NaiveDate, days: u32) -> Vec<String> {
    let Some(cutoff) = today.checked_sub_days(chrono::Days::new(u64::from(days))) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut deleted = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if parse_day_file(name).is_some_and(|date| date < cutoff)
            && entry.file_type().is_ok_and(|t| t.is_file())
            && std::fs::remove_file(entry.path()).is_ok()
        {
            deleted.push(name.to_owned());
        }
    }
    deleted
}

/// The writer's state; driven by [`run`], or directly by tests.
pub(crate) struct Writer {
    pid: u32,
    nodes: HashMap<LogId, NodeLog>,
}

impl Writer {
    pub(crate) fn new(pid: u32) -> Self {
        Self {
            pid,
            nodes: HashMap::new(),
        }
    }

    pub(crate) fn handle(&mut self, command: Command) {
        match command {
            Command::Open {
                node,
                folder,
                days,
                hub,
                today,
            } => {
                clean(&folder, today, days);
                self.nodes.insert(
                    node,
                    NodeLog {
                        folder,
                        days,
                        hub,
                        file: None,
                        failed: None,
                    },
                );
            }
            Command::Lines { node, date, text } => {
                if let Some(log) = self.nodes.get_mut(&node) {
                    log.write(self.pid, date, &text);
                }
            }
            Command::Close { node, ack } => {
                if let Some(mut log) = self.nodes.remove(&node)
                    && let Some(mut file) = log.file.take()
                    && let Err(e) = file.out.flush()
                {
                    log.warn(format!(
                        "cannot write log file {}: {e}",
                        file.path.display()
                    ));
                }
                let _ = ack.send(());
            }
        }
    }

    pub(crate) fn flush_all(&mut self) {
        for log in self.nodes.values_mut() {
            if let Some(file) = &mut log.file
                && let Err(e) = file.out.flush()
            {
                let (date, message) = (
                    file.date,
                    format!("cannot write log file {}: {e}", file.path.display()),
                );
                log.fail(date, &message);
            }
        }
    }

    pub(crate) fn clean_all(&self, today: NaiveDate) {
        for log in self.nodes.values() {
            clean(&log.folder, today, log.days);
        }
    }

    /// The path of the node's open file (for tests).
    #[cfg(test)]
    fn open_path(&self, node: LogId) -> Option<&Path> {
        self.nodes
            .get(&node)?
            .file
            .as_ref()
            .map(|f| f.path.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("must-logwriter-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn open(w: &mut Writer, node: LogId, folder: &Path, days: u32, today: NaiveDate) {
        w.handle(Command::Open {
            node,
            folder: folder.to_owned(),
            days,
            hub: Weak::new(),
            today,
        });
    }

    fn lines(w: &mut Writer, node: LogId, d: NaiveDate, text: &str) {
        w.handle(Command::Lines {
            node,
            date: d,
            text: text.into(),
        });
    }

    fn close(w: &mut Writer, node: LogId) {
        let (ack, mut done) = tokio::sync::oneshot::channel();
        w.handle(Command::Close { node, ack });
        assert!(done.try_recv().is_ok());
    }

    #[test]
    fn midnight() {
        let dir = temp("midnight");
        let mut w = Writer::new(1);
        open(&mut w, 1, &dir, 2, date(27));
        lines(&mut w, 1, date(27), "2026-10-27T23:59:59.900+01:00 < a\n");
        let first = w.open_path(1).unwrap().to_owned();
        lines(&mut w, 1, date(28), "2026-10-28T00:00:00.100+01:00 < b\n");
        assert_eq!(w.open_path(1).unwrap(), dir.join("2026-10-28.log"));
        // the previous day's file is closed: complete, and its lock released
        assert_eq!(
            std::fs::read_to_string(&first).unwrap(),
            "2026-10-27T23:59:59.900+01:00 < a\n"
        );
        File::open(&first).unwrap().try_lock().unwrap();
        close(&mut w, 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("2026-10-28.log")).unwrap(),
            "2026-10-28T00:00:00.100+01:00 < b\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locked_file_falls_back_to_pid_file() {
        let dir = temp("locked");
        std::fs::create_dir_all(&dir).unwrap();
        // "another process" holds today's file
        let held = open_locked(&dir.join("2026-10-27.log")).unwrap().unwrap();
        let mut w = Writer::new(4242);
        open(&mut w, 1, &dir, 2, date(27));
        lines(&mut w, 1, date(27), "x\n");
        assert_eq!(w.open_path(1).unwrap(), dir.join("2026-10-27-4242.log"));
        close(&mut w, 1);
        drop(held);
        assert_eq!(
            std::fs::read_to_string(dir.join("2026-10-27-4242.log")).unwrap(),
            "x\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn touch(dir: &Path, names: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        for name in names {
            std::fs::write(dir.join(name), "x\n").unwrap();
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn default_retention() {
        let dir = temp("retention");
        touch(
            &dir,
            &[
                "2026-10-24.log",
                "2026-10-24-77.log",
                "2026-10-25.log",
                "2026-10-26.log",
                "notes.txt",
                "2026-10-01.log.bak",
            ],
        );
        // at node start
        let mut w = Writer::new(1);
        open(&mut w, 1, &dir, 2, date(27));
        assert_eq!(
            names(&dir),
            [
                "2026-10-01.log.bak",
                "2026-10-25.log",
                "2026-10-26.log",
                "notes.txt"
            ]
        );
        // hourly, the next day
        w.clean_all(date(28));
        assert_eq!(
            names(&dir),
            ["2026-10-01.log.bak", "2026-10-26.log", "notes.txt"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn longer_retention() {
        let dir = temp("retention30");
        touch(
            &dir,
            &["2026-09-26.log", "2026-09-27.log", "2026-10-26.log"],
        );
        assert_eq!(clean(&dir, date(27), 30), ["2026-09-26.log"]);
        assert_eq!(names(&dir), ["2026-09-27.log", "2026-10-26.log"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwritable_directory_retries_next_day() {
        let base = temp("unwritable");
        std::fs::create_dir_all(&base).unwrap();
        let blocker = base.join("file");
        std::fs::write(&blocker, "").unwrap();
        let folder = blocker.join("node");
        let mut w = Writer::new(1);
        open(&mut w, 1, &folder, 2, date(27));
        lines(&mut w, 1, date(27), "a\n");
        assert!(w.open_path(1).is_none());
        assert_eq!(w.nodes[&1].failed, Some(date(27)));
        // the problem goes away; logging resumes with the next day
        std::fs::remove_file(&blocker).unwrap();
        lines(&mut w, 1, date(27), "b\n");
        assert!(w.open_path(1).is_none(), "not retried the same day");
        lines(&mut w, 1, date(28), "c\n");
        assert_eq!(w.open_path(1).unwrap(), folder.join("2026-10-28.log"));
        close(&mut w, 1);
        assert_eq!(
            std::fs::read_to_string(folder.join("2026-10-28.log")).unwrap(),
            "c\n"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
