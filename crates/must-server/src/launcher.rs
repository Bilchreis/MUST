// SPDX-License-Identifier: LGPL-3.0-or-later

//! Opening the status TUI (`must-tui`) in a window of its own, as SHALL's status window
//! flags ask for (`bGUI`, `SECoP_S_showStatusWindow`).
//!
//! [`show_status_tui`] returns at once; a short-lived thread does the work. It starts
//! nothing if a TUI already runs for the user (the TUI holds a lock in the monitoring
//! registry directory). The executable is looked for next to the library (or program)
//! containing must, then at `MUST_TUI`, then on `PATH`.
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

use must_monitor::{platform, registry};

/// Environment variable naming the TUI executable (or the directory holding it).
pub const TUI_VARIABLE: &str = "MUST_TUI";
/// Environment variable with the terminal command used on Linux.
pub const TERMINAL_VARIABLE: &str = "MUST_TUI_TERMINAL";

/// File name of the TUI executable.
pub const EXECUTABLE: &str = if cfg!(windows) {
    "must-tui.exe"
} else {
    "must-tui"
};

/// What [`launch`] did.
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

/// Opens the status TUI in a window of its own unless one runs already; returns at once
/// and logs what happened.
pub fn show_status_tui() {
    let spawned = std::thread::Builder::new()
        .name("must-tui-launch".into())
        .spawn(|| report(&launch()));
    if let Err(e) = spawned {
        tracing::warn!("cannot open the status TUI: {e}");
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

fn report(outcome: &Launch) {
    match outcome {
        Launch::Started { program, .. } => {
            tracing::info!("status TUI started: {}", program.display());
        }
        Launch::AlreadyRunning => tracing::info!("the status TUI is already running"),
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
        use std::os::unix::fs::PermissionsExt;

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
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = format!("{} --title must", script.display());
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
}
