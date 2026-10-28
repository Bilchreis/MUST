// SPDX-License-Identifier: LGPL-3.0-or-later

//! The per-user directory in which every process with must nodes registers its
//! monitoring endpoint as `<pid>.json`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};
use thiserror::Error;

use crate::platform::{self, ProcessIdentity};

/// Environment variable that overrides the registry directory (for tests and special
/// setups); the directory must be private to the user like the default one.
pub const DIR_VARIABLE: &str = "MUST_MONITOR_DIR";

/// The registry directory cannot be used.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// No directory could be determined (e.g. no `LOCALAPPDATA` on Windows).
    #[error("no registry directory: {0}")]
    NoDirectory(String),
    /// The directory exists but another user owns it or others can access it.
    #[error("unsafe registry directory {}: {reason}", path.display())]
    Unsafe {
        /// The directory.
        path: PathBuf,
        /// What is wrong.
        reason: String,
    },
    /// A file system operation failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The path concerned.
        path: PathBuf,
        /// The error.
        source: io::Error,
    },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> RegistryError + '_ {
    move |source| RegistryError::Io {
        path: path.to_owned(),
        source,
    }
}

/// The registry directory, created if missing and checked to be private to the user:
/// `$XDG_RUNTIME_DIR/must` (or `/tmp/must-<uid>`) on Linux, `%LOCALAPPDATA%\must\run`
/// on Windows, or [`DIR_VARIABLE`] if set.
pub fn registry_dir() -> Result<PathBuf, RegistryError> {
    let dir = match std::env::var_os(DIR_VARIABLE) {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => default_dir()?,
    };
    prepare(&dir)?;
    Ok(dir)
}

/// Prepares `dir` as registry directory like [`registry_dir`] does with the default one
/// (created if missing, checked to be private to the user).
pub fn prepare_dir(dir: &Path) -> Result<(), RegistryError> {
    prepare(dir)
}

#[cfg(unix)]
fn default_dir() -> Result<PathBuf, RegistryError> {
    Ok(unix_dir(
        std::env::var_os("XDG_RUNTIME_DIR"),
        current_uid()?,
    ))
}

/// `$XDG_RUNTIME_DIR/must`, or `/tmp/must-<uid>` without a runtime directory.
#[cfg(unix)]
fn unix_dir(runtime_dir: Option<std::ffi::OsString>, uid: u32) -> PathBuf {
    match runtime_dir {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("must"),
        _ => PathBuf::from(format!("/tmp/must-{uid}")),
    }
}

#[cfg(windows)]
fn default_dir() -> Result<PathBuf, RegistryError> {
    match std::env::var_os("LOCALAPPDATA") {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d).join("must").join("run")),
        _ => Err(RegistryError::NoDirectory("LOCALAPPDATA is not set".into())),
    }
}

#[cfg(target_os = "linux")]
fn current_uid() -> Result<u32, RegistryError> {
    platform::uid().map_err(io_err(Path::new("/proc/self")))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn current_uid() -> Result<u32, RegistryError> {
    Err(RegistryError::NoDirectory(
        "monitoring supports Linux and Windows".into(),
    ))
}

/// Creates the directory with mode 0700 and refuses one that is not private.
#[cfg(unix)]
fn prepare(dir: &Path) -> Result<(), RegistryError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(io_err(dir)(e)),
    }
    let meta = std::fs::symlink_metadata(dir).map_err(io_err(dir))?;
    let unsafe_dir = |reason: String| RegistryError::Unsafe {
        path: dir.to_owned(),
        reason,
    };
    if !meta.is_dir() {
        return Err(unsafe_dir("not a directory".into()));
    }
    let uid = current_uid()?;
    if meta.uid() != uid {
        return Err(unsafe_dir(format!(
            "owned by uid {}, not {uid}",
            meta.uid()
        )));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(unsafe_dir(format!(
            "accessible by others (mode {:o})",
            meta.permissions().mode() & 0o777
        )));
    }
    Ok(())
}

/// Creates the directory; it inherits the per-user ACL of `%LOCALAPPDATA%`.
#[cfg(windows)]
fn prepare(dir: &Path) -> Result<(), RegistryError> {
    std::fs::create_dir_all(dir).map_err(io_err(dir))
}

/// How to reach an endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointDescription {
    /// A Unix domain socket (Linux).
    Unix {
        /// The socket file.
        path: PathBuf,
    },
    /// A named pipe (Windows).
    Pipe {
        /// The pipe name, e.g. `\\.\pipe\must-S-1-5-...-4242`.
        name: String,
    },
    /// A kind this version does not know, e.g. from a newer must.
    Unsupported {
        /// The kind.
        kind: String,
        /// All fields as written.
        fields: Map<String, Json>,
    },
}

impl Serialize for EndpointDescription {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let json = match self {
            EndpointDescription::Unix { path } => {
                serde_json::json!({"kind": "unix", "path": path})
            }
            EndpointDescription::Pipe { name } => serde_json::json!({"kind": "pipe", "name": name}),
            EndpointDescription::Unsupported { fields, .. } => Json::Object(fields.clone()),
        };
        json.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EndpointDescription {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let fields = Map::<String, Json>::deserialize(deserializer)?;
        let kind = fields
            .get("kind")
            .and_then(Json::as_str)
            .ok_or_else(|| D::Error::missing_field("kind"))?
            .to_owned();
        let text = |key: &'static str| {
            fields
                .get(key)
                .and_then(Json::as_str)
                .map(str::to_owned)
                .ok_or_else(|| D::Error::missing_field(key))
        };
        Ok(match kind.as_str() {
            "unix" => EndpointDescription::Unix {
                path: PathBuf::from(text("path")?),
            },
            "pipe" => EndpointDescription::Pipe {
                name: text("name")?,
            },
            _ => EndpointDescription::Unsupported { kind, fields },
        })
    }
}

/// A process's registration, the content of `<pid>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registration {
    /// Monitoring protocol version of the endpoint.
    pub version: String,
    /// Process id.
    pub pid: u32,
    /// Executable name.
    pub name: String,
    /// Process start time in the operating system's units (see
    /// [`ProcessIdentity::start`]); detects pid reuse.
    pub start: u64,
    /// Process start as UNIX time in seconds.
    pub start_time: f64,
    /// How to reach the endpoint.
    pub endpoint: EndpointDescription,
}

impl Registration {
    /// A registration of `process` with `endpoint`.
    pub fn new(process: &ProcessIdentity, endpoint: EndpointDescription) -> Self {
        Self {
            version: crate::protocol::protocol_version(),
            pid: process.pid,
            name: process.name.clone(),
            start: process.start,
            start_time: process.start_unix,
            endpoint,
        }
    }
}

/// A written registration; dropping it deletes the file (and on Linux the endpoint's
/// socket file).
#[derive(Debug)]
pub struct RegistrationFile {
    path: PathBuf,
    socket: Option<PathBuf>,
}

impl RegistrationFile {
    /// The file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RegistrationFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(socket) = &self.socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// Writes `<pid>.json` into `dir` atomically (temporary file, then rename), readable
/// only by the user.
pub fn register(
    dir: &Path,
    registration: &Registration,
) -> Result<RegistrationFile, RegistryError> {
    let path = dir.join(format!("{}.json", registration.pid));
    let tmp = dir.join(format!("{}.json.tmp", registration.pid));
    let text = serde_json::to_vec_pretty(registration).unwrap_or_default();
    write_private(&tmp, &text).map_err(io_err(&tmp))?;
    std::fs::rename(&tmp, &path).map_err(io_err(&path))?;
    let socket = match &registration.endpoint {
        EndpointDescription::Unix { path } => Some(path.clone()),
        _ => None,
    };
    Ok(RegistrationFile { path, socket })
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data)
}

#[cfg(windows)]
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    std::fs::write(path, data)
}

/// Whether a listed endpoint can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointStatus {
    /// Its process runs; connect with [`crate::client`].
    Live,
    /// Its endpoint kind is not supported by this version.
    Unsupported,
}

/// A registration found in the directory.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    /// The registration.
    pub registration: Registration,
    /// Whether it can be used.
    pub status: EndpointStatus,
}

/// Lists the registrations in `dir`: live ones and those of unsupported kinds. Files that
/// cannot be parsed are skipped; stale registrations (process gone, or its pid reused by
/// another process) are deleted together with their socket files.
pub fn list(dir: &Path) -> Result<Vec<Listed>, RegistryError> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io_err(dir))? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let is_registration = path.extension().is_some_and(|e| e == "json")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.parse::<u32>().is_ok());
        if !is_registration {
            continue;
        }
        let Ok(text) = std::fs::read(&path) else {
            continue;
        };
        let Ok(registration) = serde_json::from_slice::<Registration>(&text) else {
            continue;
        };
        if matches!(
            registration.endpoint,
            EndpointDescription::Unsupported { .. }
        ) {
            found.push(Listed {
                registration,
                status: EndpointStatus::Unsupported,
            });
            continue;
        }
        match platform::process(registration.pid) {
            Ok(Some(p)) if p.start == registration.start && p.name == registration.name => {
                found.push(Listed {
                    registration,
                    status: EndpointStatus::Live,
                });
            }
            Ok(_) => {
                // stale: remove the registration and its socket
                let _ = std::fs::remove_file(&path);
                if let EndpointDescription::Unix { path } = &registration.endpoint {
                    let _ = std::fs::remove_file(path);
                }
            }
            // cannot tell (e.g. no permission): keep it and let connecting decide
            Err(_) => found.push(Listed {
                registration,
                status: EndpointStatus::Live,
            }),
        }
    }
    found.sort_by_key(|l| l.registration.pid);
    Ok(found)
}

/// Name of the lock file in the registry directory that the status TUI holds while it
/// runs, so that only one runs per user.
pub const TUI_LOCK: &str = "tui.lock";

/// Takes the TUI lock in `dir` without waiting: `Some(file)` holds it until the file is
/// dropped (or the process ends), `None` if another process holds it.
pub fn try_lock_tui(dir: &Path) -> io::Result<Option<std::fs::File>> {
    // read and write access: Windows refuses to lock append-only handles
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(TUI_LOCK))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Whether a status TUI holds the lock in `dir`.
pub fn tui_running(dir: &Path) -> io::Result<bool> {
    Ok(try_lock_tui(dir)?.is_none())
}

#[cfg(test)]
mod tests {
    #[test]
    fn tui_lock() {
        let dir = std::env::temp_dir().join(format!("must-tui-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!super::tui_running(&dir).unwrap());
        let held = super::try_lock_tui(&dir).unwrap().expect("free");
        assert!(super::tui_running(&dir).unwrap());
        assert!(super::try_lock_tui(&dir).unwrap().is_none());
        drop(held);
        assert!(!super::tui_running(&dir).unwrap(), "released with the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("must-monitor-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn own_registration(dir: &Path) -> Registration {
        let me = platform::current_process().unwrap();
        Registration::new(
            &me,
            EndpointDescription::Unix {
                path: dir.join(format!("{}.sock", me.pid)),
            },
        )
    }

    #[cfg(unix)]
    #[test]
    fn linux_directories() {
        assert_eq!(
            unix_dir(Some("/run/user/1000".into()), 1000),
            Path::new("/run/user/1000/must")
        );
        // fallback without XDG_RUNTIME_DIR
        assert_eq!(unix_dir(None, 1000), Path::new("/tmp/must-1000"));
        assert_eq!(unix_dir(Some("".into()), 1000), Path::new("/tmp/must-1000"));
    }

    #[test]
    fn endpoint_descriptions() {
        let unix: EndpointDescription =
            serde_json::from_str(r#"{"kind":"unix","path":"/run/user/1000/must/4242.sock"}"#)
                .unwrap();
        assert_eq!(
            unix,
            EndpointDescription::Unix {
                path: "/run/user/1000/must/4242.sock".into()
            }
        );
        assert_eq!(
            serde_json::to_string(&unix).unwrap(),
            r#"{"kind":"unix","path":"/run/user/1000/must/4242.sock"}"#
        );
        let pipe: EndpointDescription =
            serde_json::from_str(r#"{"kind":"pipe","name":"\\\\.\\pipe\\must-S-1-4242"}"#).unwrap();
        assert!(matches!(pipe, EndpointDescription::Pipe { ref name } if name.ends_with("4242")));
        let tcp: EndpointDescription =
            serde_json::from_str(r#"{"kind":"tcp","addr":"127.0.0.1:5000","token":"x"}"#).unwrap();
        assert!(matches!(tcp, EndpointDescription::Unsupported { ref kind, .. } if kind == "tcp"));
        // written back unchanged
        assert!(
            serde_json::to_string(&tcp)
                .unwrap()
                .contains("\"token\":\"x\"")
        );
        assert!(serde_json::from_str::<EndpointDescription>(r#"{"path":"x"}"#).is_err());
    }

    #[test]
    fn registration_lifecycle_and_listing() {
        let dir = temp_dir("lifecycle");
        prepare(&dir).unwrap();
        let registration = own_registration(&dir);
        let file = register(&dir, &registration).unwrap();
        let name = format!("{}.json", std::process::id());
        assert!(dir.join(&name).exists());
        assert!(!dir.join(format!("{name}.tmp")).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(&name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "private file");
        }
        let listed = list(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].registration, registration);
        assert_eq!(listed[0].status, EndpointStatus::Live);
        drop(file);
        assert!(!dir.join(&name).exists());
        assert_eq!(list(&dir).unwrap(), Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_kind_and_garbage() {
        let dir = temp_dir("unknown");
        prepare(&dir).unwrap();
        std::fs::write(
            dir.join("77.json"),
            r#"{"version":"1.0","pid":77,"name":"x","start":1,"start_time":0.0,
               "endpoint":{"kind":"tcp","addr":"127.0.0.1:5000","token":"t"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("78.json"), "not json").unwrap();
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        let file = register(&dir, &own_registration(&dir)).unwrap();
        let listed = list(&dir).unwrap();
        assert_eq!(listed.len(), 2);
        let unsupported = listed.iter().find(|l| l.registration.pid == 77).unwrap();
        assert_eq!(unsupported.status, EndpointStatus::Unsupported);
        assert!(dir.join("77.json").exists(), "unsupported kinds are kept");
        assert!(dir.join("78.json").exists() && dir.join("notes.txt").exists());
        drop(file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_registrations_are_removed() {
        let dir = temp_dir("stale");
        prepare(&dir).unwrap();
        // a process that has exited
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                &["/C", "exit"][..]
            } else {
                &[][..]
            })
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        let socket = dir.join(format!("{pid}.sock"));
        std::fs::write(&socket, "").unwrap();
        let mut gone = own_registration(&dir);
        gone.pid = pid;
        gone.endpoint = EndpointDescription::Unix {
            path: socket.clone(),
        };
        std::mem::forget(register(&dir, &gone).unwrap());
        // our own pid, but another start time: pid reuse
        let mut reused = own_registration(&dir);
        reused.start += 1;
        let reused_path = dir.join(format!("{}.json", reused.pid));
        std::mem::forget(register(&dir, &reused).unwrap());

        assert_eq!(list(&dir).unwrap(), Vec::new());
        assert!(!dir.join(format!("{pid}.json")).exists());
        assert!(
            !socket.exists(),
            "socket file removed with its registration"
        );
        assert!(!reused_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unsafe_directories_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("unsafe");
        prepare(&dir).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(prepare(&dir), Err(RegistryError::Unsafe { .. })));
        let _ = std::fs::remove_dir_all(&dir);
        // owned by another user: root's /
        if current_uid().unwrap() != 0 {
            assert!(matches!(
                prepare(Path::new("/")),
                Err(RegistryError::Unsafe { .. })
            ));
        }
        let file = temp_dir("unsafe-file");
        std::fs::write(&file, "").unwrap();
        assert!(prepare(&file).is_err());
        let _ = std::fs::remove_file(&file);
    }
}
