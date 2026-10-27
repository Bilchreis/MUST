// SPDX-License-Identifier: LGPL-3.0-or-later

//! Configuration of the log files: on/off, directory, retention.

use std::path::PathBuf;
use std::sync::LazyLock;

/// `MUST_LOG_FILES=0` turns file logging off.
pub const ENABLE_VARIABLE: &str = "MUST_LOG_FILES";
/// `MUST_LOG_DIR` sets the log directory.
pub const DIR_VARIABLE: &str = "MUST_LOG_DIR";
/// `MUST_LOG_DAYS` sets the retention in days (at least 1).
pub const DAYS_VARIABLE: &str = "MUST_LOG_DAYS";

/// Default retention in days.
pub const DEFAULT_DAYS: u32 = 2;

/// Where and how long a node logs to disk; see the [module documentation](super).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFiles {
    /// Whether the node writes log files.
    pub enabled: bool,
    /// The log directory; each node writes into a folder named after its
    /// `equipment_id`. `None` for the default directory.
    pub dir: Option<PathBuf>,
    /// Files are deleted once their day ended more than this many days ago.
    pub days: u32,
}

impl Default for LogFiles {
    /// On, default directory, 2 days.
    fn default() -> Self {
        Self {
            enabled: true,
            dir: None,
            days: DEFAULT_DAYS,
        }
    }
}

impl LogFiles {
    /// Logging turned off.
    pub fn off() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Logging into `dir`.
    #[must_use]
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Keeping `days` days (at least 1).
    #[must_use]
    pub fn days(mut self, days: u32) -> Self {
        self.days = days.max(1);
        self
    }

    /// The settings of the environment variables, read once per process; invalid
    /// values are reported (as a returned warning) and replaced by the default.
    pub fn from_env() -> Self {
        ENV.0.clone()
    }

    /// The directory to use: the configured one or the per-user default.
    pub fn resolved_dir(&self) -> Option<PathBuf> {
        self.dir.clone().or_else(default_dir)
    }
}

/// The environment settings and the warnings about invalid values, read once.
static ENV: LazyLock<(LogFiles, Vec<String>)> = LazyLock::new(|| {
    parse(
        std::env::var(ENABLE_VARIABLE).ok().as_deref(),
        std::env::var_os(DIR_VARIABLE),
        std::env::var(DAYS_VARIABLE).ok().as_deref(),
    )
});

/// Warnings about invalid environment values (reported once, when the first node
/// starts logging).
pub(super) fn env_warnings() -> &'static [String] {
    &ENV.1
}

/// The settings from the values of the environment variables.
pub(super) fn parse(
    enabled: Option<&str>,
    dir: Option<std::ffi::OsString>,
    days: Option<&str>,
) -> (LogFiles, Vec<String>) {
    let mut warnings = Vec::new();
    let mut config = LogFiles::default();
    match enabled.map(str::trim) {
        None | Some("" | "1") => {}
        Some("0") => config.enabled = false,
        Some(other) => warnings.push(format!(
            "{ENABLE_VARIABLE}={other:?} is not 0 or 1; file logging stays on"
        )),
    }
    if let Some(dir) = dir.filter(|d| !d.is_empty()) {
        config.dir = Some(PathBuf::from(dir));
    }
    match days.map(str::trim) {
        None | Some("") => {}
        Some(text) => match text.parse::<u32>() {
            Ok(n) if n >= 1 => config.days = n,
            _ => warnings.push(format!(
                "{DAYS_VARIABLE}={text:?} is not a number of days of at least 1; keeping {DEFAULT_DAYS}"
            )),
        },
    }
    (config, warnings)
}

/// The per-user default log directory: `$XDG_STATE_HOME/must/logs` or
/// `~/.local/state/must/logs` on Linux, `%LOCALAPPDATA%\must\logs` on Windows.
pub fn default_dir() -> Option<PathBuf> {
    default_dir_from(&|name| std::env::var_os(name))
}

fn default_dir_from(var: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        set("LOCALAPPDATA").map(|d| d.join("must").join("logs"))
    } else {
        set("XDG_STATE_HOME")
            .or_else(|| set("HOME").map(|h| h.join(".local").join("state")))
            .map(|d| d.join("must").join("logs"))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn environment_values() {
        let (c, w) = parse(None, None, None);
        assert_eq!(c, LogFiles::default());
        assert_eq!(w, Vec::<String>::new());

        let (c, w) = parse(Some("0"), Some(OsString::from("/data/logs")), Some("7"));
        assert!(!c.enabled);
        assert_eq!(c.dir, Some(PathBuf::from("/data/logs")));
        assert_eq!(c.days, 7);
        assert_eq!(w, Vec::<String>::new());

        // invalid values: a warning each and the default
        let (c, w) = parse(Some("maybe"), Some(OsString::new()), Some("0"));
        assert!(c.enabled);
        assert_eq!(c.dir, None);
        assert_eq!(c.days, DEFAULT_DAYS);
        assert_eq!(w.len(), 2, "{w:?}");
        let (c, w) = parse(None, None, Some("two"));
        assert_eq!(c.days, DEFAULT_DAYS);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn rust_api() {
        let c = LogFiles::default().dir("/x").days(0);
        assert_eq!(c.days, 1, "at least one day");
        assert_eq!(c.resolved_dir(), Some(PathBuf::from("/x")));
        assert!(!LogFiles::off().enabled);
    }

    #[test]
    fn default_directories() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        if cfg!(windows) {
            assert_eq!(
                default_dir_from(&vars(&[("LOCALAPPDATA", r"C:\Users\me\AppData\Local")])),
                Some(PathBuf::from(r"C:\Users\me\AppData\Local\must\logs"))
            );
        } else {
            assert_eq!(
                default_dir_from(&vars(&[("XDG_STATE_HOME", "/s"), ("HOME", "/home/me")])),
                Some(PathBuf::from("/s/must/logs"))
            );
            assert_eq!(
                default_dir_from(&vars(&[("HOME", "/home/me")])),
                Some(PathBuf::from("/home/me/.local/state/must/logs"))
            );
        }
        assert_eq!(default_dir_from(&vars(&[])), None);
    }
}
