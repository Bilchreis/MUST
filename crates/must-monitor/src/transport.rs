// SPDX-License-Identifier: LGPL-3.0-or-later

//! The local connection between an endpoint and its monitors, behind one seam: the
//! endpoint binds a [`Listener`] that yields byte streams, a monitor calls [`connect`]
//! with the registration's [`EndpointDescription`]. Everything above sees only
//! [`Stream`]s, so another transport (e.g. local TCP) is a new description kind plus a
//! listener and a connector here.
//!
//! - Linux: a Unix domain socket `<registry dir>/<pid>.sock`, mode 0600, in the 0700
//!   registry directory.
//! - Windows: a named pipe `\\.\pipe\must-<user SID>-<pid>` whose security descriptor
//!   allows only the current user and SYSTEM; the first instance is created exclusively,
//!   and clients check that the pipe's server is the registered process.

use std::io;
use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::registry::EndpointDescription;

/// A bidirectional byte stream to or from an endpoint.
pub trait AsyncStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncStream for T {}

/// A connection of the local transport.
pub type Stream = Box<dyn AsyncStream>;

/// The endpoint's side: accepts monitor connections.
pub struct Listener {
    inner: imp::Listener,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener").finish_non_exhaustive()
    }
}

impl Listener {
    /// Opens the endpoint of the current process (`dir` is the registry directory) and
    /// returns how monitors reach it. Must be called within a tokio runtime.
    pub fn bind(dir: &Path) -> io::Result<(Self, EndpointDescription)> {
        let (inner, description) = imp::bind(dir, std::process::id())?;
        Ok((Self { inner }, description))
    }

    /// Waits for the next monitor.
    pub async fn accept(&mut self) -> io::Result<Stream> {
        self.inner.accept().await
    }
}

/// Connects to the endpoint described by a registration of process `pid`.
pub async fn connect(endpoint: &EndpointDescription, pid: u32) -> io::Result<Stream> {
    match endpoint {
        EndpointDescription::Unsupported { kind, .. } => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("endpoint kind `{kind}` is not supported"),
        )),
        _ => imp::connect(endpoint, pid).await,
    }
}

#[cfg(unix)]
mod imp {
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use tokio::net::{UnixListener, UnixStream};

    use super::Stream;
    use crate::registry::EndpointDescription;

    /// Longest socket path the kernel accepts (`sun_path` is 108 bytes with the NUL).
    const MAX_PATH: usize = 107;

    pub(super) struct Listener {
        listener: UnixListener,
        path: PathBuf,
    }

    pub(super) fn bind(dir: &Path, pid: u32) -> io::Result<(Listener, EndpointDescription)> {
        let path = dir.join(format!("{pid}.sock"));
        if path.as_os_str().len() > MAX_PATH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("socket path {} is too long", path.display()),
            ));
        }
        // a leftover of an earlier process with the same pid
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let description = EndpointDescription::Unix { path: path.clone() };
        Ok((Listener { listener, path }, description))
    }

    impl Listener {
        pub(super) async fn accept(&mut self) -> io::Result<Stream> {
            let (stream, _) = self.listener.accept().await?;
            Ok(Box::new(stream))
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub(super) async fn connect(endpoint: &EndpointDescription, _pid: u32) -> io::Result<Stream> {
        match endpoint {
            EndpointDescription::Unix { path } => Ok(Box::new(UnixStream::connect(path).await?)),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "only Unix domain sockets are supported on this platform",
            )),
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::time::Duration;

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;

    use super::Stream;
    use crate::platform::user_sid;
    use crate::registry::EndpointDescription;

    /// `ERROR_PIPE_BUSY`: all instances are connected; try again shortly.
    const ERROR_PIPE_BUSY: i32 = 231;

    /// A security descriptor allowing only the current user and SYSTEM.
    struct Descriptor(PSECURITY_DESCRIPTOR);

    // SAFETY: the descriptor is an immutable block of memory owned by this value
    unsafe impl Send for Descriptor {}
    // SAFETY: as above; it is only read
    unsafe impl Sync for Descriptor {}

    impl Descriptor {
        fn for_current_user() -> io::Result<Self> {
            let sid = user_sid()?;
            // protected DACL: generic all for the user and for SYSTEM, nobody else
            let sddl = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)");
            let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            // SAFETY: NUL-terminated wide string, valid out pointer
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &raw mut descriptor,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(descriptor))
        }

        fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                    .unwrap_or(u32::MAX),
                lpSecurityDescriptor: self.0,
                bInheritHandle: 0,
            }
        }
    }

    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW
            unsafe { LocalFree(self.0.cast()) };
        }
    }

    pub(super) struct Listener {
        name: String,
        descriptor: Descriptor,
        /// The instance waiting for the next monitor.
        next: NamedPipeServer,
    }

    fn create(name: &str, descriptor: &Descriptor, first: bool) -> io::Result<NamedPipeServer> {
        let mut attributes = descriptor.attributes();
        // SAFETY: the attributes and the descriptor they point to outlive the call
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(name, (&raw mut attributes).cast::<c_void>())
        }
    }

    pub(super) fn bind(_dir: &Path, pid: u32) -> io::Result<(Listener, EndpointDescription)> {
        let name = format!(r"\\.\pipe\must-{}-{pid}", user_sid()?);
        let descriptor = Descriptor::for_current_user()?;
        // the first instance is exclusive: nobody else can have created this name
        let next = create(&name, &descriptor, true)?;
        let description = EndpointDescription::Pipe { name: name.clone() };
        Ok((
            Listener {
                name,
                descriptor,
                next,
            },
            description,
        ))
    }

    impl Listener {
        pub(super) async fn accept(&mut self) -> io::Result<Stream> {
            self.next.connect().await?;
            let fresh = create(&self.name, &self.descriptor, false)?;
            let connected = std::mem::replace(&mut self.next, fresh);
            Ok(Box::new(connected))
        }
    }

    pub(super) async fn connect(endpoint: &EndpointDescription, pid: u32) -> io::Result<Stream> {
        let EndpointDescription::Pipe { name } = endpoint else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "only named pipes are supported on this platform",
            ));
        };
        let mut attempts = 0;
        let client = loop {
            match ClientOptions::new().open(name) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < 40 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e),
            }
        };
        let mut server_pid = 0u32;
        // SAFETY: valid pipe handle, valid out pointer
        let ok =
            unsafe { GetNamedPipeServerProcessId(client.as_raw_handle(), &raw mut server_pid) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if server_pid != pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("pipe {name} is served by process {server_pid}, not {pid}"),
            ));
        }
        Ok(Box::new(client))
    }

    /// A SID given as text (full `S-1-...` or an SDDL alias such as `LA` or `SY`) as full
    /// `S-1-...` text, for tests.
    #[cfg(test)]
    pub(super) fn full_sid(text: &str) -> io::Result<String> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSidToSidW,
        };
        use windows_sys::Win32::Security::PSID;
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sid: PSID = std::ptr::null_mut();
        // SAFETY: NUL-terminated wide string, valid out pointer
        if unsafe { ConvertStringSidToSidW(wide.as_ptr(), &raw mut sid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut out: *mut u16 = std::ptr::null_mut();
        // SAFETY: valid SID from above, valid out pointer
        let ok = unsafe { ConvertSidToStringSidW(sid, &raw mut out) };
        // SAFETY: allocated by ConvertStringSidToSidW with LocalAlloc
        unsafe { LocalFree(sid.cast()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: NUL-terminated wide string allocated with LocalAlloc
        let full = unsafe {
            let mut n = 0;
            while *out.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(out, n));
            LocalFree(out.cast());
            s
        };
        Ok(full)
    }

    /// The pipe's DACL as SDDL, for tests.
    #[cfg(test)]
    pub(super) fn dacl_of(
        client: &tokio::net::windows::named_pipe::NamedPipeClient,
    ) -> io::Result<String> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
        };
        use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: valid handle, out pointers
        let status = unsafe {
            GetSecurityInfo(
                client.as_raw_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(
                i32::try_from(status).unwrap_or(-1),
            ));
        }
        let descriptor = Descriptor(descriptor);
        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: valid descriptor, out pointer
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor.0,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &raw mut text,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: NUL-terminated wide string allocated with LocalAlloc
        let sddl = unsafe {
            let mut n = 0;
            while *text.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
            LocalFree(text.cast());
            s
        };
        Ok(sddl)
    }
}

#[cfg(test)]
#[allow(
    clippy::await_holding_lock,
    reason = "the guard serializes whole tests"
)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    /// The endpoint name is per process (on Windows the pipe is created exclusively), so
    /// tests that bind it run one at a time.
    static BIND: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn bind_lock() -> std::sync::MutexGuard<'static, ()> {
        BIND.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("must-transport-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        dir
    }

    #[tokio::test]
    async fn bind_accept_connect() {
        let _bind = bind_lock();
        let dir = temp_dir("roundtrip");
        let (mut listener, description) = Listener::bind(&dir).unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let mut stream = listener.accept().await.unwrap();
                let mut buf = [0u8; 5];
                stream.read_exact(&mut buf).await.unwrap();
                stream.write_all(&buf).await.unwrap();
            }
            listener
        });
        for _ in 0..2 {
            let mut client = connect(&description, std::process::id()).await.unwrap();
            client.write_all(b"hello").await.unwrap();
            let mut buf = [0u8; 5];
            client.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"hello");
        }
        let listener = server.await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let EndpointDescription::Unix { path } = &description else {
                panic!("{description:?}")
            };
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "socket only for the user");
            drop(listener);
            assert!(!path.exists(), "socket file removed with the listener");
        }
        #[cfg(windows)]
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unsupported_kind() {
        let tcp = EndpointDescription::Unsupported {
            kind: "tcp".into(),
            fields: serde_json::Map::new(),
        };
        let err = connect(&tcp, 1).await.err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn pipe_only_for_the_user() {
        let _bind = bind_lock();
        let dir = temp_dir("dacl");
        let (mut listener, description) = Listener::bind(&dir).unwrap();
        let EndpointDescription::Pipe { name } = &description else {
            panic!("{description:?}")
        };
        // keep accepting, so that every client finds a free instance
        tokio::spawn(async move {
            let mut accepted = Vec::new();
            while let Ok(stream) = listener.accept().await {
                accepted.push(stream);
            }
        });
        let client = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(name)
            .unwrap();
        let dacl = imp::dacl_of(&client).unwrap();
        let sid = crate::platform::user_sid().unwrap();
        // only allow entries for the user and SYSTEM, no inherited ones
        assert!(dacl.starts_with("D:P"), "{dacl}");
        let entries: Vec<&str> = dacl.split('(').skip(1).collect();
        assert_ne!(entries.len(), 0);
        // SDDL may abbreviate well-known SIDs (e.g. `LA` for the built-in administrator)
        let system = imp::full_sid("SY").unwrap();
        for entry in entries {
            let trustee = entry.trim_end_matches(')').rsplit(';').next().unwrap();
            let trustee = imp::full_sid(trustee).unwrap();
            assert!(
                trustee == sid || trustee == system,
                "unexpected trustee {trustee} in {dacl}"
            );
        }
        // wrong pid: refused
        let err = connect(&description, std::process::id() + 1)
            .await
            .err()
            .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }
}
