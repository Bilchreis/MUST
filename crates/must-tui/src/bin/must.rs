// SPDX-License-Identifier: LGPL-3.0-or-later

//! `must`: the command line entry point. `must monitor` runs the status TUI
//! (`must-tui`), found next to this executable or else on `PATH`. `must update` installs
//! the latest release (the highest `vX.Y.Z` tag), a given tag or `main` in the checkout
//! `must` was installed from, without changing what that checkout has checked out. It
//! fetches them from the public repository over HTTPS, which needs no credentials.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Where `must update` fetches releases and `main` from, whatever the checkout's remotes
/// are: anonymous HTTPS works without SSH keys and through firewalls closing port 22.
const UPSTREAM: &str = concat!(env!("CARGO_PKG_REPOSITORY"), ".git");

/// The checkout's ref that `must update` fetches upstream's `main` into, apart from the
/// checkout's own remote-tracking branches.
const UPSTREAM_MAIN: &str = "refs/must-update/main";

const USAGE: &str = "\
usage: must <command> [arguments]

commands:
  monitor [--focus-pid <pid>]  show all MUST nodes of this user (the status TUI)
  update [--tag <tag>]         install the latest release, if newer than this one;
                               with --tag: that release (e.g. v0.2.0), `latest` or
                               `main` (the latest commit on main)

  --version                    print the version
  --help                       print this text";

#[cfg(windows)]
const TUI: &str = "must-tui.exe";
#[cfg(not(windows))]
const TUI: &str = "must-tui";

/// `must-tui` next to this executable (as `cargo install` and `dist/` put them), else
/// the bare name, looked up on `PATH`.
fn tui() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(TUI)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(TUI))
}

/// The checkout `must` was installed from: `MUST_CHECKOUT` when `must update` built it
/// in a temporary worktree, else where it was built (`cargo install --path` builds in
/// place). `None` for a `must` from `dist/` or a release archive (`cargo xtask dist`
/// sets `MUST_CHECKOUT` empty).
fn checkout() -> Option<PathBuf> {
    match option_env!("MUST_CHECKOUT") {
        Some("") => None,
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")),
    }
}

/// `"v1.2.3"` or `"1.2.3"` (a pre-release suffix is ignored) as numbers.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn main() -> ExitCode {
    #[cfg(windows)]
    windows::remove_old_exe();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("monitor") => run(Command::new(tui()).args(args)),
        Some("update") => match (args.next().as_deref(), args.next(), args.next()) {
            (None, ..) => update(None),
            (Some("--tag"), Some(tag), None) => update(Some(&tag)),
            (Some("--tag"), None, _) => usage_error("--tag needs a tag, e.g. v0.2.0 or main"),
            (Some(arg), ..) => usage_error(&format!("unknown argument {arg:?} to update")),
        },
        Some("--version") => {
            println!("must {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => usage_error(&format!("unknown command {other:?}")),
        None => usage_error("no command given"),
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("must: {message}\n\n{USAGE}");
    ExitCode::from(2)
}

/// Replaces this process with `command` (Unix), so the TUI owns the terminal alone.
#[cfg(unix)]
fn run(command: &mut Command) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let e = command.exec();
    eprintln!("must: cannot run {}: {e}", command.get_program().display());
    ExitCode::FAILURE
}

/// Runs `command` and passes on its exit code.
#[cfg(not(unix))]
fn run(command: &mut Command) -> ExitCode {
    match command.status() {
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map_or(ExitCode::FAILURE, ExitCode::from),
        Err(e) => {
            eprintln!("must: cannot run {}: {e}", command.get_program().display());
            ExitCode::FAILURE
        }
    }
}

/// Installs `tag` from [`UPSTREAM`]: `None` or `latest` the highest `vX.Y.Z` tag if it
/// is newer than this `must`, `main` the latest commit of upstream's `main`,
/// anything else that tag. It is built in a temporary worktree, so the checkout keeps
/// its branch and local changes.
fn update(tag: Option<&str>) -> ExitCode {
    let Some(root) = checkout() else {
        eprintln!(
            "must: this must comes from a release archive; download new releases from \
             {}/releases, or clone the repository and run install.sh \
             (install.ps1 on Windows) to get `must update`",
            env!("CARGO_PKG_REPOSITORY")
        );
        return ExitCode::FAILURE;
    };
    if !root.join(".git").exists() {
        eprintln!(
            "must: {} is no longer a git checkout of MUST; clone it again and run \
             install.sh (install.ps1 on Windows) there",
            root.display()
        );
        return ExitCode::FAILURE;
    }
    let root = root.canonicalize().unwrap_or(root);
    let git = || {
        let mut git = Command::new("git");
        git.arg("-C").arg(&root);
        git
    };
    let fetched = succeeds(
        git()
            .args([
                "fetch",
                "--quiet",
                "--no-tags",
                UPSTREAM,
                "+refs/tags/*:refs/tags/*",
            ])
            .arg(format!("+refs/heads/main:{UPSTREAM_MAIN}"))
            // fail instead of asking for credentials the public repository does not need
            .env("GIT_TERMINAL_PROMPT", "0"),
    );
    if !fetched {
        eprintln!("must: cannot fetch the releases; nothing was installed");
        return ExitCode::FAILURE;
    }
    let installed = env!("CARGO_PKG_VERSION");
    let (rev, name) = match tag {
        None | Some("latest") => {
            let Some((version, tag)) = latest_release(&mut git()) else {
                eprintln!("must: no release found in {}", root.display());
                return ExitCode::FAILURE;
            };
            if parse_version(installed).is_some_and(|i| i >= version) {
                println!("MUST {installed} is up to date (latest release: {tag}).");
                return ExitCode::SUCCESS;
            }
            (tag.clone(), tag)
        }
        Some("main") => (UPSTREAM_MAIN.to_owned(), "main".to_owned()),
        // `0.2.0` means `v0.2.0`
        Some(tag) if !tag.starts_with('v') && parse_version(tag).is_some() => {
            (format!("refs/tags/v{tag}"), format!("v{tag}"))
        }
        Some(tag) => (format!("refs/tags/{tag}"), tag.to_owned()),
    };
    if !holds(
        git()
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("{rev}^{{commit}}")),
    ) {
        eprintln!(
            "must: {name} not found in {}; nothing was installed",
            root.display()
        );
        return ExitCode::FAILURE;
    }
    // installing a version without `must` would make cargo remove this one
    if !holds(
        git()
            .args(["cat-file", "-e"])
            .arg(format!("{rev}:crates/must-tui/src/bin/must.rs")),
    ) {
        eprintln!(
            "must: {name} has no `must` command yet, so `must update` cannot install it; \
             check it out and run install.sh there instead"
        );
        return ExitCode::FAILURE;
    }

    println!("Installing {name} (installed: {installed})");
    install(&root, &rev, &name)
}

/// Builds `rev` of the checkout `root` in a temporary worktree and installs it.
fn install(root: &Path, rev: &str, name: &str) -> ExitCode {
    let git = || {
        let mut git = Command::new("git");
        git.arg("-C").arg(root);
        git
    };
    let worktree = std::env::temp_dir().join(format!("must-update-{}", std::process::id()));
    if !succeeds(
        git()
            .args(["worktree", "add", "--quiet", "--detach"])
            .arg(&worktree)
            .arg(rev),
    ) {
        eprintln!("must: cannot check out {name}; nothing was installed");
        return ExitCode::FAILURE;
    }
    // Windows cannot overwrite a running executable, but can rename it
    #[cfg(windows)]
    let moved = windows::move_exe_aside();
    let installed = succeeds(
        Command::new("cargo")
            .args(["install", "--locked", "--path", "crates/must-tui"])
            // the tag's rust-toolchain.toml applies; the checkout's build cache is reused
            .current_dir(&worktree)
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env("MUST_CHECKOUT", root),
    );
    #[cfg(windows)]
    if !installed {
        windows::restore_exe(moved);
    }
    let _ = git()
        .args(["worktree", "remove", "--force"])
        .arg(&worktree)
        .status();
    if installed {
        println!("Installed {name}.");
        ExitCode::SUCCESS
    } else {
        eprintln!("must: cargo install failed (on Windows, close must-tui first)");
        ExitCode::FAILURE
    }
}

/// The highest `vX.Y.Z` tag (releases only: no pre-release tags like `v1.0.0-rc1`).
fn latest_release(git: &mut Command) -> Option<((u64, u64, u64), String)> {
    let out = git.args(["tag", "--list", "v*"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|tag| !tag.contains(['-', '+']))
        .filter_map(|tag| Some((parse_version(tag)?, tag.to_owned())))
        .max()
}

/// Whether `command` succeeds, without showing its output (for yes/no questions to git).
fn holds(command: &mut Command) -> bool {
    command.output().is_ok_and(|out| out.status.success())
}

fn succeeds(command: &mut Command) -> bool {
    match command.status() {
        Ok(status) => status.success(),
        Err(e) => {
            eprintln!("must: cannot run {}: {e}", command.get_program().display());
            false
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::path::PathBuf;

    fn paths() -> Option<(PathBuf, PathBuf)> {
        let exe = std::env::current_exe().ok()?;
        let old = exe.with_extension("exe.old");
        Some((exe, old))
    }

    /// Removes what the last `must update` moved aside.
    pub fn remove_old_exe() {
        if let Some((_, old)) = paths() {
            let _ = std::fs::remove_file(old);
        }
    }

    /// Renames the running `must.exe` so `cargo install` can write a new one.
    pub fn move_exe_aside() -> bool {
        paths().is_some_and(|(exe, old)| std::fs::rename(exe, old).is_ok())
    }

    /// Puts `must.exe` back after a failed install, unless a new one was written.
    pub fn restore_exe(moved: bool) {
        if let Some((exe, old)) = paths().filter(|_| moved)
            && !exe.exists()
        {
            let _ = std::fs::rename(old, exe);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.2.10"), Some((0, 2, 10)));
        assert_eq!(parse_version("1.0.0-rc1"), Some((1, 0, 0)));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        assert_eq!(parse_version("vnext"), None);
        // numeric, not alphabetic order
        assert!(parse_version("v0.10.0") > parse_version("v0.9.0"));
    }
}
