// SPDX-License-Identifier: LGPL-3.0-or-later

//! Opening the status TUI (`must-tui`) in a window of its own, as SHALL's status window
//! flags ask for (`bGUI`, `SECoP_S_showStatusWindow`).
//!
//! [`show_status_tui`] returns at once; a thread of its own does the work and keeps the
//! TUI open: it looks every [`CHECK_INTERVAL`] and opens the TUI again when it was closed,
//! until [`hide_status_tui`]. It starts nothing while a TUI runs for the user (the TUI
//! holds a lock in the monitoring registry directory) or another process is starting one
//! (it holds a second lock there until its TUI is up). A TUI that does not come up within
//! [`START_TIMEOUT`] (blocked, or failed) is tried again at growing intervals, up to
//! [`MAX_RETRY`]. The executable is looked for next to the library (or program)
//! containing must, then at `MUST_TUI`, then on `PATH`. `MUST_TUI=0` turns launching off.
//!
//! - **Windows:** the TUI gets a console window of its own; closing it ends only the TUI.
//! - **Linux:** with a graphical session (`DISPLAY` or `WAYLAND_DISPLAY`), the TUI runs in
//!   a terminal emulator: `MUST_TUI_TERMINAL` (a command, the TUI's command line is
//!   appended), else the first found of `$TERMINAL -e`, `x-terminal-emulator -e`,
//!   `gnome-terminal --`, `konsole -e`, `xfce4-terminal -x`, `xterm -e`. Without one, an
//!   info record says to run `must-tui` by hand.
//!
//! The TUI is passed `--focus-pid <pid>` to select this process's first node, and keeps
//! running when this process ends. Problems are logged, never returned to the caller.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use must_monitor::{platform, registry};

/// Environment variable naming the TUI executable (or the directory holding it); `0`
/// turns launching off.
pub const TUI_VARIABLE: &str = "MUST_TUI";
/// Environment variable with the terminal command used on Linux.
pub const TERMINAL_VARIABLE: &str = "MUST_TUI_TERMINAL";

/// File name of the TUI executable.
pub const EXECUTABLE: &str = if cfg!(windows) {
    "must-tui.exe"
} else {
    "must-tui"
};

/// How often a shown TUI is looked for, to open it again when it was closed.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// How long a started TUI may take to come up (to take its lock).
pub const START_TIMEOUT: Duration = Duration::from_secs(20);
/// The longest wait between attempts while the TUI cannot be opened.
pub const MAX_RETRY: Duration = Duration::from_secs(300);

/// How often a TUI being started is looked for.
const START_POLL: Duration = Duration::from_millis(200);

/// What [`launch`] (or a look of the thread keeping the TUI open) did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Started: the pid of the TUI (Windows) or of its terminal emulator (Linux).
    Started {
        /// The process started.
        pid: u32,
        /// The TUI executable.
        program: PathBuf,
    },
    /// A TUI already runs for this user.
    AlreadyRunning,
    /// Another process is starting a TUI.
    Starting,
    /// A TUI was started but did not come up within [`START_TIMEOUT`]: blocked (by
    /// endpoint protection, say) or failed.
    NotUp {
        /// The TUI executable.
        program: PathBuf,
    },
    /// Launching is turned off (`MUST_TUI=0`).
    TurnedOff,
    /// No TUI executable was found in these places.
    NotFound {
        /// Where it was looked for.
        searched: Vec<PathBuf>,
    },
    /// Linux without a graphical session: nothing to open a window in.
    NoGraphicalSession,
    /// No terminal emulator could be started.
    NoTerminal {
        /// The commands tried.
        tried: Vec<String>,
    },
    /// Something else failed.
    Failed(String),
}

/// The thread keeping the TUI open, while there is one.
static KEEPER: Mutex<Option<Keeper>> = Mutex::new(None);

struct Keeper {
    /// Wakes the thread to look now; dropping it stops the thread.
    wake: Sender<()>,
    thread: JoinHandle<()>,
}

/// Timing of the thread keeping the TUI open.
#[derive(Debug, Clone, Copy)]
struct Timing {
    interval: Duration,
    start_timeout: Duration,
    max_retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            interval: CHECK_INTERVAL,
            start_timeout: START_TIMEOUT,
            max_retry: MAX_RETRY,
        }
    }
}

/// Opens the status TUI in a window of its own unless one runs already, and keeps it open
/// (opens it again when it is closed) until [`hide_status_tui`]; returns at once and logs
/// what happened.
pub fn show_status_tui() {
    let mut keeper = KEEPER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(k) = keeper.as_ref()
        && !k.thread.is_finished()
    {
        // look now, not at the next interval
        let _ = k.wake.send(());
        return;
    }
    let (wake, woken) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("must-tui-keeper".into())
        .spawn(move || {
            let dir = match registry::registry_dir() {
                Ok(dir) => dir,
                Err(e) => {
                    let reason = format!("no monitoring registry directory: {e}");
                    return report(&Launch::Failed(reason), false);
                }
            };
            let var = |name: &str| std::env::var_os(name);
            let library = library_dir();
            let pid = std::process::id();
            keep_open(
                Timing::default(),
                &dir,
                library.as_deref(),
                &var,
                pid,
                &woken,
            );
        });
    match spawned {
        Ok(thread) => *keeper = Some(Keeper { wake, thread }),
        Err(e) => tracing::warn!("cannot open the status TUI: {e}"),
    }
}

/// Stops keeping the status TUI open; a TUI that runs stays open (it may show other
/// processes' nodes). Waits for a launch in progress, at most a process-spawn time.
pub fn hide_status_tui() {
    let keeper = KEEPER.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(Keeper { wake, thread }) = keeper {
        drop(wake);
        if thread.thread().id() != std::thread::current().id() {
            let _ = thread.join();
        }
    }
}

/// Keeps the TUI open until `woken` is disconnected (or launching is turned off): looks
/// now, then after each wait, and at once when woken.
fn keep_open(
    timing: Timing,
    dir: &Path,
    library_dir: Option<&Path>,
    var: &dyn Fn(&str) -> Option<OsString>,
    pid: u32,
    woken: &Receiver<()>,
) {
    let mut failures = 0;
    let mut last: Option<Launch> = None;
    loop {
        let Some(outcome) = look(timing, dir, library_dir, var, pid, woken) else {
            return;
        };
        let wait = match outcome {
            Launch::AlreadyRunning | Launch::Starting | Launch::Started { .. } => {
                failures = 0;
                timing.interval
            }
            Launch::TurnedOff => return report(&outcome, false),
            _ => {
                failures += 1;
                retry_wait(timing, failures)
            }
        };
        // each start is worth a record; anything else only when it changes
        let was_up = matches!(last, Some(Launch::AlreadyRunning | Launch::Started { .. }));
        let same = last
            .as_ref()
            .is_some_and(|l| std::mem::discriminant(l) == std::mem::discriminant(&outcome));
        if matches!(outcome, Launch::Started { .. }) || !same {
            report(&outcome, was_up);
        }
        last = Some(outcome);
        match woken.recv_timeout(wait) {
            Ok(()) => {
                // asked to show it: look now, as if for the first time
                while woken.try_recv().is_ok() {}
                failures = 0;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The wait after `failures` looks in a row that could not open the TUI: the interval,
/// doubled after each further failure, at most `max_retry`.
fn retry_wait(timing: Timing, failures: u32) -> Duration {
    let factor = 1u32 << failures.saturating_sub(1).min(16);
    timing
        .interval
        .saturating_mul(factor)
        .min(timing.max_retry)
        .max(timing.interval)
}

/// One look: opens the TUI unless it runs or another process is starting one, holding
/// the launch lock until a TUI started here is up. `None` if told to stop meanwhile.
fn look(
    timing: Timing,
    dir: &Path,
    library_dir: Option<&Path>,
    var: &dyn Fn(&str) -> Option<OsString>,
    pid: u32,
    woken: &Receiver<()>,
) -> Option<Launch> {
    let _launching = match registry::try_lock_launch(dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => return Some(Launch::Starting),
        Err(e) => return Some(Launch::Failed(format!("cannot take the launch lock: {e}"))),
    };
    let outcome = launch_in(dir, library_dir, var, pid);
    let Launch::Started { program, .. } = &outcome else {
        return Some(outcome);
    };
    let deadline = Instant::now() + timing.start_timeout;
    loop {
        match registry::tui_running(dir) {
            Ok(true) => return Some(outcome),
            Ok(false) => {}
            Err(e) => return Some(Launch::Failed(format!("cannot check for the TUI: {e}"))),
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            let program = program.clone();
            return Some(Launch::NotUp { program });
        }
        if let Err(RecvTimeoutError::Disconnected) = woken.recv_timeout(left.min(START_POLL)) {
            return None;
        }
    }
}

/// Opens the status TUI now (see the [module documentation](self)), from the calling
/// thread; [`show_status_tui`] does this in the background.
pub fn launch() -> Launch {
    let dir = match registry::registry_dir() {
        Ok(dir) => dir,
        Err(e) => return Launch::Failed(format!("no monitoring registry directory: {e}")),
    };
    let var = |name: &str| std::env::var_os(name);
    launch_in(&dir, library_dir().as_deref(), &var, std::process::id())
}

/// Logs what a launch did; `reopened` when the TUI ran before and was closed.
fn report(outcome: &Launch, reopened: bool) {
    match outcome {
        Launch::Started { program, .. } if reopened => {
            tracing::info!("status TUI closed, opened again: {}", program.display());
        }
        Launch::Started { program, .. } => {
            tracing::info!("status TUI started: {}", program.display());
        }
        Launch::AlreadyRunning => tracing::info!("the status TUI is already running"),
        Launch::Starting => tracing::debug!("another process is starting the status TUI"),
        Launch::NotUp { program } => tracing::warn!(
            "status TUI started ({}) but it did not come up within {}s; it may have been \
             blocked or failed to start; trying again at growing intervals",
            program.display(),
            START_TIMEOUT.as_secs()
        ),
        Launch::TurnedOff => {
            tracing::info!("opening the status TUI is turned off ({TUI_VARIABLE}=0)");
        }
        Launch::NotFound { searched } => {
            let places: Vec<String> = searched.iter().map(|p| p.display().to_string()).collect();
            tracing::warn!(
                "status TUI not found (looked for {}); put {EXECUTABLE} next to the must \
                 library, or set {TUI_VARIABLE} to its path",
                places.join(", ")
            );
        }
        Launch::NoGraphicalSession => tracing::info!(
            "no graphical session to open the status TUI in; run `must-tui` in a terminal"
        ),
        Launch::NoTerminal { tried } => tracing::info!(
            "no terminal emulator found to open the status TUI in (tried {}); run \
             `must-tui` in a terminal, or set {TERMINAL_VARIABLE}",
            tried.join(", ")
        ),
        Launch::Failed(e) => tracing::warn!("cannot open the status TUI: {e}"),
    }
}

/// The directory of the library or program that contains this code.
fn library_dir() -> Option<PathBuf> {
    let address = (library_dir as fn() -> Option<PathBuf> as *const ()).addr();
    let path = platform::module_path(address).ok()?;
    path.parent().map(Path::to_owned)
}

/// [`launch`] with its inputs: the registry directory, the library directory, the
/// environment and the pid to focus.
fn launch_in(
    registry_dir: &Path,
    library_dir: Option<&Path>,
    var: &dyn Fn(&str) -> Option<OsString>,
    pid: u32,
) -> Launch {
    if var(TUI_VARIABLE).is_some_and(|v| v.to_str().map(str::trim) == Some("0")) {
        return Launch::TurnedOff;
    }
    match registry::tui_running(registry_dir) {
        Ok(true) => return Launch::AlreadyRunning,
        Ok(false) => {}
        Err(e) => return Launch::Failed(format!("cannot check for a running TUI: {e}")),
    }
    let program = match find_executable(library_dir, var) {
        Ok(program) => program,
        Err(searched) => return Launch::NotFound { searched },
    };
    let args = vec!["--focus-pid".to_owned(), pid.to_string()];
    start(program, &args, var)
}

/// The TUI executable: next to the library, at `MUST_TUI` (a file, or a directory
/// holding it), on `PATH`; else the places searched.
fn find_executable(
    library_dir: Option<&Path>,
    var: &dyn Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, Vec<PathBuf>> {
    let mut candidates = Vec::new();
    if let Some(dir) = library_dir {
        candidates.push(dir.join(EXECUTABLE));
    }
    if let Some(path) = var(TUI_VARIABLE).filter(|v| !v.is_empty()) {
        let path = PathBuf::from(path);
        candidates.push(if path.is_dir() {
            path.join(EXECUTABLE)
        } else {
            path
        });
    }
    if let Some(paths) = var("PATH") {
        candidates.extend(std::env::split_paths(&paths).map(|d| d.join(EXECUTABLE)));
    }
    match candidates.iter().find(|c| c.is_file()) {
        Some(found) => Ok(found.clone()),
        None => Err(candidates),
    }
}

#[cfg(windows)]
fn start(program: PathBuf, args: &[String], _var: &dyn Fn(&str) -> Option<OsString>) -> Launch {
    match platform::spawn_in_new_console(&program, args) {
        Ok(pid) => Launch::Started { pid, program },
        Err(e) => Launch::Failed(format!("cannot start {}: {e}", program.display())),
    }
}

#[cfg(not(windows))]
fn start(program: PathBuf, args: &[String], var: &dyn Fn(&str) -> Option<OsString>) -> Launch {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    if !set("DISPLAY") && !set("WAYLAND_DISPLAY") {
        return Launch::NoGraphicalSession;
    }
    let mut tried = Vec::new();
    for terminal in terminals(var) {
        let Some((command, terminal_args)) = terminal.split_first() else {
            continue;
        };
        tried.push(terminal.join(" "));
        let Some(command) = find_program(command, var) else {
            continue;
        };
        let spawned = Command::new(&command)
            .args(terminal_args)
            .arg(&program)
            .args(args)
            // its own process group: not hit by signals to the host's group
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = spawned {
            let pid = child.id();
            // reap it when it ends, so no zombie is left behind
            let _ = std::thread::Builder::new()
                .name("must-tui-terminal".into())
                .spawn(move || child.wait());
            return Launch::Started { pid, program };
        }
    }
    Launch::NoTerminal { tried }
}

/// Terminal commands in the order they are tried; the TUI's command line is appended.
#[cfg(not(windows))]
fn terminals(var: &dyn Fn(&str) -> Option<OsString>) -> Vec<Vec<String>> {
    let text = |name: &str| {
        var(name)
            .map(|v| v.to_string_lossy().into_owned())
            .filter(|v| !v.trim().is_empty())
    };
    let mut terminals = Vec::new();
    if let Some(custom) = text(TERMINAL_VARIABLE) {
        terminals.push(custom.split_whitespace().map(str::to_owned).collect());
    }
    if let Some(terminal) = text("TERMINAL") {
        terminals.push(vec![terminal, "-e".to_owned()]);
    }
    for (command, flag) in [
        ("x-terminal-emulator", "-e"),
        ("gnome-terminal", "--"),
        ("konsole", "-e"),
        ("xfce4-terminal", "-x"),
        ("xterm", "-e"),
    ] {
        terminals.push(vec![command.to_owned(), flag.to_owned()]);
    }
    terminals
}

/// A program by path, or by name on `PATH`.
#[cfg(not(windows))]
fn find_program(command: &str, var: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if command.contains('/') {
        let path = PathBuf::from(command);
        return path.is_file().then_some(path);
    }
    let paths = var("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(command))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Held by the tests that take locks or start processes: a child forked by one test
    /// holds the other tests' lock files (and so their locks) until it executes.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("must-launcher-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn env(pairs: &[(&str, &Path)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.as_os_str().to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn executable_lookup_order() {
        let base = temp("lookup");
        let (lib, named, on_path) = (base.join("lib"), base.join("named"), base.join("bin"));
        let var = env(&[(TUI_VARIABLE, &named.join(EXECUTABLE)), ("PATH", &on_path)]);
        // nothing there: every place is reported
        let Err(searched) = find_executable(Some(&lib), &var) else {
            panic!("found something");
        };
        assert_eq!(
            searched,
            [
                lib.join(EXECUTABLE),
                named.join(EXECUTABLE),
                on_path.join(EXECUTABLE)
            ]
        );
        touch(&on_path.join(EXECUTABLE));
        assert_eq!(
            find_executable(Some(&lib), &var),
            Ok(on_path.join(EXECUTABLE))
        );
        touch(&named.join(EXECUTABLE));
        assert_eq!(
            find_executable(Some(&lib), &var),
            Ok(named.join(EXECUTABLE))
        );
        // shipped next to the library: found first
        touch(&lib.join(EXECUTABLE));
        assert_eq!(find_executable(Some(&lib), &var), Ok(lib.join(EXECUTABLE)));
        // MUST_TUI may name the directory
        let var = env(&[(TUI_VARIABLE, &named)]);
        assert_eq!(find_executable(None, &var), Ok(named.join(EXECUTABLE)));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn already_running_and_missing_executable() {
        let _serial = serial();
        let base = temp("running");
        let registry = base.join("registry");
        registry::prepare_dir(&registry).unwrap();
        let var = env(&[("PATH", &base.join("nowhere"))]);
        assert!(matches!(
            launch_in(&registry, None, &var, 1),
            Launch::NotFound { .. }
        ));
        let lock = registry::try_lock_tui(&registry).unwrap().unwrap();
        assert_eq!(launch_in(&registry, None, &var, 1), Launch::AlreadyRunning);
        drop(lock);
        // turned off
        let off = |name: &str| (name == TUI_VARIABLE).then(|| OsString::from("0"));
        assert_eq!(launch_in(&registry, None, &off, 1), Launch::TurnedOff);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn this_library_s_directory() {
        // in a test binary, this code lives in the executable
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            library_dir().unwrap().canonicalize().unwrap(),
            exe.parent().unwrap().canonicalize().unwrap()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn linux_without_graphical_session_or_terminal() {
        let _serial = serial();
        let base = temp("linux");
        let registry = base.join("registry");
        registry::prepare_dir(&registry).unwrap();
        let tui = base.join(EXECUTABLE);
        touch(&tui);
        let empty = base.join("empty");
        let var = env(&[(TUI_VARIABLE, &tui), ("PATH", &empty)]);
        assert_eq!(
            launch_in(&registry, None, &var, 1),
            Launch::NoGraphicalSession
        );
        let var = env(&[
            (TUI_VARIABLE, &tui),
            ("PATH", &empty),
            ("DISPLAY", Path::new(":0")),
        ]);
        let Launch::NoTerminal { tried } = launch_in(&registry, None, &var, 1) else {
            panic!("no terminal on an empty PATH");
        };
        assert_eq!(
            tried,
            [
                "x-terminal-emulator -e",
                "gnome-terminal --",
                "konsole -e",
                "xfce4-terminal -x",
                "xterm -e"
            ]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(not(windows))]
    #[test]
    fn linux_terminal_order_and_command_line() {
        let _serial = serial();
        let base = temp("terminal");
        let registry = base.join("registry");
        registry::prepare_dir(&registry).unwrap();
        let tui = base.join(EXECUTABLE);
        touch(&tui);
        // a "terminal" that records its arguments
        let record = base.join("args");
        let script = base.join("fake-terminal");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho \"$@\" > '{}'\n", record.display()),
        )
        .unwrap();
        // run by sh: executing a file just written fails while a forked test child still
        // holds it open for writing (ETXTBSY)
        let command = format!("/bin/sh {} --title must", script.display());
        let var = |name: &str| match name {
            TUI_VARIABLE => Some(tui.clone().into_os_string()),
            TERMINAL_VARIABLE => Some(OsString::from(&command)),
            "TERMINAL" => Some(OsString::from("never-tried")),
            "WAYLAND_DISPLAY" => Some(OsString::from("wayland-0")),
            "PATH" => Some(OsString::from("/usr/bin:/bin")),
            _ => None,
        };
        let Launch::Started { program, .. } = launch_in(&registry, None, &var, 4242) else {
            panic!("not started");
        };
        assert_eq!(program, tui);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !record.exists() {
            assert!(std::time::Instant::now() < deadline, "terminal not run");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        let args = std::fs::read_to_string(&record).unwrap();
        assert_eq!(
            args.trim(),
            format!("--title must {} --focus-pid 4242", tui.display())
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn retry_waits_grow_to_the_limit() {
        let timing = Timing {
            interval: Duration::from_secs(5),
            start_timeout: START_TIMEOUT,
            max_retry: Duration::from_secs(60),
        };
        let waits: Vec<u64> = (1..=6).map(|f| retry_wait(timing, f).as_secs()).collect();
        assert_eq!(waits, [5, 10, 20, 40, 60, 60]);
        assert_eq!(retry_wait(timing, u32::MAX), Duration::from_secs(60));
        // the defaults
        assert_eq!(retry_wait(Timing::default(), 1), CHECK_INTERVAL);
        assert_eq!(retry_wait(Timing::default(), 100), MAX_RETRY);
    }

    #[test]
    fn keeper_stops_without_an_executable() {
        let _serial = serial();
        let base = temp("keeper-missing");
        let registry = base.join("registry");
        registry::prepare_dir(&registry).unwrap();
        let var = env(&[("PATH", &base.join("nowhere"))]);
        let (wake, woken) = mpsc::channel();
        let fast = Timing {
            interval: Duration::from_millis(20),
            start_timeout: Duration::from_millis(100),
            max_retry: Duration::from_millis(50),
        };
        std::thread::scope(|scope| {
            let keeper = scope.spawn(|| keep_open(fast, &registry, None, &var, 1, &{ woken }));
            std::thread::sleep(Duration::from_millis(150));
            assert!(!keeper.is_finished(), "keeps looking");
            drop(wake);
        });
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A registry, a TUI executable and a "terminal" appending its arguments to a file.
    #[cfg(not(windows))]
    struct FakeTerminal {
        base: PathBuf,
        registry: PathBuf,
        tui: PathBuf,
        command: String,
        record: PathBuf,
    }

    #[cfg(not(windows))]
    impl FakeTerminal {
        fn new(name: &str) -> Self {
            let base = temp(name);
            let registry = base.join("registry");
            registry::prepare_dir(&registry).unwrap();
            let tui = base.join(EXECUTABLE);
            touch(&tui);
            let record = base.join("args");
            let script = base.join("fake-terminal");
            std::fs::write(
                &script,
                format!("#!/bin/sh\necho \"$@\" >> '{}'\n", record.display()),
            )
            .unwrap();
            // run by sh, not executed: see linux_terminal_order_and_command_line
            let command = format!("/bin/sh {}", script.display());
            FakeTerminal {
                base,
                registry,
                tui,
                command,
                record,
            }
        }

        fn var(&self, name: &str) -> Option<OsString> {
            match name {
                TUI_VARIABLE => Some(self.tui.clone().into_os_string()),
                TERMINAL_VARIABLE => Some(OsString::from(&self.command)),
                "WAYLAND_DISPLAY" => Some(OsString::from("wayland-0")),
                "PATH" => Some(OsString::from("/usr/bin:/bin")),
                _ => None,
            }
        }

        /// How often the terminal was started.
        fn starts(&self) -> usize {
            std::fs::read_to_string(&self.record).map_or(0, |r| r.lines().count())
        }

        fn wait_for_starts(&self, n: usize) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.starts() < n {
                assert!(Instant::now() < deadline, "not started {n} times");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[cfg(not(windows))]
    impl Drop for FakeTerminal {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    #[cfg(not(windows))]
    const FAST: Timing = Timing {
        interval: Duration::from_millis(30),
        start_timeout: Duration::from_secs(5),
        max_retry: Duration::from_millis(100),
    };

    #[cfg(not(windows))]
    #[test]
    fn keeper_opens_a_closed_tui_again() {
        let _serial = serial();
        let t = FakeTerminal::new("keeper-reopen");
        let var = |name: &str| t.var(name);
        let (wake, woken) = mpsc::channel();
        std::thread::scope(|scope| {
            let keeper = scope.spawn(|| keep_open(FAST, &t.registry, None, &var, 7, &{ woken }));
            t.wait_for_starts(1);
            // the TUI comes up and stays: nothing more is started
            let tui = registry::try_lock_tui(&t.registry).unwrap().unwrap();
            std::thread::sleep(FAST.interval * 8);
            assert_eq!(t.starts(), 1);
            // the user closes it: it is opened again
            drop(tui);
            t.wait_for_starts(2);
            let tui = registry::try_lock_tui(&t.registry).unwrap().unwrap();
            // hiding stops the keeper; the TUI stays
            drop(wake);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !keeper.is_finished() {
                assert!(Instant::now() < deadline, "keeper still running");
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(tui);
            std::thread::sleep(FAST.interval * 4);
            assert_eq!(t.starts(), 2, "not opened again once hidden");
        });
        let record = std::fs::read_to_string(&t.record).unwrap();
        let expected = format!("{} --focus-pid 7", t.tui.display());
        assert!(record.lines().all(|l| l == expected), "{record}");
    }

    #[cfg(not(windows))]
    #[test]
    fn keeper_waits_for_another_process_starting_one() {
        let _serial = serial();
        let t = FakeTerminal::new("keeper-starting");
        let var = |name: &str| t.var(name);
        let (wake, woken) = mpsc::channel();
        let launching = registry::try_lock_launch(&t.registry).unwrap().unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| keep_open(FAST, &t.registry, None, &var, 7, &{ woken }));
            std::thread::sleep(FAST.interval * 6);
            assert_eq!(t.starts(), 0, "another process is starting one");
            drop(launching);
            t.wait_for_starts(1);
            drop(wake);
        });
    }

    #[cfg(not(windows))]
    #[test]
    fn a_tui_that_does_not_come_up() {
        let _serial = serial();
        let t = FakeTerminal::new("keeper-not-up");
        let var = |name: &str| t.var(name);
        let (wake, woken) = mpsc::channel();
        let quick = Timing {
            start_timeout: Duration::from_millis(100),
            ..FAST
        };
        let outcome = look(quick, &t.registry, None, &var, 7, &woken);
        assert_eq!(
            outcome,
            Some(Launch::NotUp {
                program: t.tui.clone()
            })
        );
        // the launch lock is released after the look
        assert!(registry::try_lock_launch(&t.registry).unwrap().is_some());
        // hiding while waiting for it to come up ends the look
        drop(wake);
        assert_eq!(look(FAST, &t.registry, None, &var, 7, &woken), None);
        t.wait_for_starts(2);
    }
}
