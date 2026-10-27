// SPDX-License-Identifier: LGPL-3.0-or-later

//! What the registry needs from the operating system: identities of processes (to tell
//! a live registration from a stale one, also after pid reuse) and of the current user.

use std::io;

/// A process as identified in registrations.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessIdentity {
    /// Process id.
    pub pid: u32,
    /// Executable name, e.g. `LabVIEW.exe` or `my_node`.
    pub name: String,
    /// Start time in the operating system's own units (Linux: clock ticks since boot;
    /// Windows: 100 ns since 1601). Together with the pid it identifies the process.
    pub start: u64,
    /// Start time as UNIX time in seconds, 0 if unknown.
    pub start_unix: f64,
}

/// The current process.
pub fn current_process() -> io::Result<ProcessIdentity> {
    imp::process(std::process::id())?.ok_or_else(|| io::Error::other("own process not found"))
}

/// The process with this pid, `None` if there is none.
pub fn process(pid: u32) -> io::Result<Option<ProcessIdentity>> {
    imp::process(pid)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::os::unix::fs::MetadataExt;

    use super::ProcessIdentity;

    /// Clock ticks per second of `/proc/<pid>/stat` times (`sysconf(_SC_CLK_TCK)`, 100 on
    /// all current Linux platforms).
    const CLOCK_TICKS: f64 = 100.0;

    pub(super) fn process(pid: u32) -> io::Result<Option<ProcessIdentity>> {
        let content = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        // "pid (comm) state ... starttime ..."; comm may contain spaces and parentheses
        let (head, rest) = content
            .rsplit_once(')')
            .ok_or_else(|| io::Error::other("unexpected /proc stat format"))?;
        let name = head
            .split_once('(')
            .map(|(_, comm)| comm.to_owned())
            .unwrap_or_default();
        // after the comm: field 3 (state) is index 0, field 22 (starttime) index 19
        let start: u64 = rest
            .split_whitespace()
            .nth(19)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::other("unexpected /proc stat format"))?;
        Ok(Some(ProcessIdentity {
            pid,
            name,
            start,
            #[allow(
                clippy::cast_precision_loss,
                reason = "seconds as f64 are precise enough"
            )]
            start_unix: boot_time().map_or(0.0, |boot| boot + start as f64 / CLOCK_TICKS),
        }))
    }

    fn boot_time() -> Option<f64> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        stat.lines()
            .find_map(|l| l.strip_prefix("btime "))
            .and_then(|s| s.trim().parse::<f64>().ok())
    }

    /// The current user's uid.
    pub(crate) fn uid() -> io::Result<u32> {
        Ok(std::fs::metadata("/proc/self")?.uid())
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, LocalFree, STILL_ACTIVE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };

    use super::ProcessIdentity;

    /// 100 ns intervals between 1601-01-01 and 1970-01-01.
    const UNIX_EPOCH_FILETIME: u64 = 116_444_736_000_000_000;

    /// Closes a handle when dropped.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by us and is closed once
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn process(pid: u32) -> io::Result<Option<ProcessIdentity>> {
        // SAFETY: plain call; a null handle means no such process (or no access)
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return Ok(None);
        }
        let handle = Handle(raw);
        let mut code = 0u32;
        // SAFETY: valid handle and out pointer
        if unsafe { GetExitCodeProcess(handle.0, &raw mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // still running reads as STILL_ACTIVE (259; no must host exits with that code)
        if i64::from(code) != i64::from(STILL_ACTIVE) {
            return Ok(None);
        }
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: valid handle and out pointers
        let ok = unsafe {
            GetProcessTimes(
                handle.0,
                &raw mut created,
                &raw mut exited,
                &raw mut kernel,
                &raw mut user,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let start = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let mut buf = vec![0u16; 1024];
        let mut len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: valid handle, buffer of `len` u16
        let ok = unsafe {
            QueryFullProcessImageNameW(handle.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &raw mut len)
        };
        let name = if ok == 0 {
            String::new()
        } else {
            let path = std::ffi::OsString::from_wide(&buf[..len as usize]);
            std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        #[allow(
            clippy::cast_precision_loss,
            reason = "seconds as f64 are precise enough"
        )]
        let start_unix = start.saturating_sub(UNIX_EPOCH_FILETIME) as f64 / 1e7;
        Ok(Some(ProcessIdentity {
            pid,
            name,
            start,
            start_unix,
        }))
    }

    /// The current user's SID as text, e.g. `S-1-5-21-...-1001`.
    pub(crate) fn user_sid() -> io::Result<String> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo handle of the current process, valid out pointer
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(token);
        let mut buf = vec![0u8; 256];
        let mut len = 0u32;
        loop {
            let size = u32::try_from(buf.len()).unwrap_or(u32::MAX);
            // SAFETY: valid token handle, buffer of `size` bytes, out pointer
            let ok = unsafe {
                GetTokenInformation(
                    token.0,
                    TokenUser,
                    buf.as_mut_ptr().cast(),
                    size,
                    &raw mut len,
                )
            };
            if ok != 0 {
                break;
            }
            if len as usize > buf.len() {
                buf.resize(len as usize, 0);
            } else {
                return Err(io::Error::last_os_error());
            }
        }
        // SAFETY: on success the buffer starts with a TOKEN_USER; read unaligned to be safe
        let user: TOKEN_USER = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast()) };
        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: the SID points into `buf`, which is alive; valid out pointer
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &raw mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success `text` is a NUL-terminated wide string allocated with LocalAlloc
        let sid = unsafe {
            let mut n = 0;
            while *text.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
            LocalFree(text.cast());
            s
        };
        Ok(sid)
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use std::io;

    use super::ProcessIdentity;

    pub(super) fn process(_pid: u32) -> io::Result<Option<ProcessIdentity>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "monitoring supports Linux and Windows",
        ))
    }
}

#[cfg(target_os = "linux")]
pub(crate) use imp::uid;
#[cfg(windows)]
pub(crate) use imp::user_sid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process() {
        let me = current_process().unwrap();
        assert_eq!(me.pid, std::process::id());
        assert_ne!(me.name, "");
        assert!(me.start > 0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        assert!(
            me.start_unix > 1.0e9 && me.start_unix <= now + 1.0,
            "{me:?}"
        );
        assert_eq!(process(me.pid).unwrap(), Some(me));
    }

    #[test]
    fn exited_process() {
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
        // reaped: either gone or (rarely) reused by another process with another start
        if let Some(p) = process(pid).unwrap() {
            assert_ne!(p.start, 0);
        }
    }
}
