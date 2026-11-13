// SPDX-License-Identifier: LGPL-3.0-or-later

//! What the registry needs from the operating system: identities of processes (to tell
//! a live registration from a stale one, also after pid reuse) and of the current user;
//! and what launching the status TUI needs: the file of the library a function lives
//! in, and on Windows a process with a console window of its own.

use std::io;
use std::path::PathBuf;

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

/// The file of the executable or library (DLL, shared object) containing the code at
/// `address`, e.g. the address of one of its functions.
pub fn module_path(address: usize) -> io::Result<PathBuf> {
    imp::module_path(address)
}

/// Starts `program` with `args` as a process with a console window of its own (a new
/// console and process group, no handles inherited), so that closing that window ends
/// only that process. Returns its pid.
#[cfg(windows)]
pub fn spawn_in_new_console(program: &std::path::Path, args: &[String]) -> io::Result<u32> {
    imp::spawn_in_new_console(program, args)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    /// The mapped file that contains `address`, from `/proc/self/maps`.
    pub(super) fn module_path(address: usize) -> io::Result<PathBuf> {
        let maps = std::fs::read_to_string("/proc/self/maps")?;
        mapped_file(&maps, address)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "address not in a mapped file"))
    }

    /// The file of the mapping containing `address` in `maps` (the `/proc/<pid>/maps`
    /// format: `start-end perms offset dev inode path`).
    pub(super) fn mapped_file(maps: &str, address: usize) -> Option<PathBuf> {
        maps.lines().find_map(|line| {
            let mut fields = line.splitn(6, ' ');
            let (start, end) = fields.next()?.split_once('-')?;
            let start = usize::from_str_radix(start, 16).ok()?;
            let end = usize::from_str_radix(end, 16).ok()?;
            let path = fields.nth(4)?.trim_start();
            let path = path.strip_suffix(" (deleted)").unwrap_or(path);
            (start <= address && address < end && path.starts_with('/'))
                .then(|| PathBuf::from(path))
        })
    }

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

    /// The module (EXE or DLL) containing `address`.
    pub(super) fn module_path(address: usize) -> io::Result<std::path::PathBuf> {
        use windows_sys::Win32::Foundation::HMODULE;
        use windows_sys::Win32::System::LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            GetModuleFileNameW, GetModuleHandleExW,
        };
        let mut module: HMODULE = std::ptr::null_mut();
        // SAFETY: with FROM_ADDRESS the "name" is an address inside the module; the
        // reference count is not changed, so nothing needs to be released
        let found = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                address as *const u16,
                &raw mut module,
            )
        };
        if found == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u16; 260];
        loop {
            let size = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
            // SAFETY: the buffer holds `size` characters
            let len = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), size) };
            if len == 0 {
                return Err(io::Error::last_os_error());
            }
            if len < size {
                buffer.truncate(len as usize);
                return Ok(std::ffi::OsString::from_wide(&buffer).into());
            }
            // cut off: try a larger buffer
            buffer.resize(buffer.len() * 2, 0);
        }
    }

    pub(super) fn spawn_in_new_console(
        program: &std::path::Path,
        args: &[String],
    ) -> io::Result<u32> {
        use std::os::windows::ffi::OsStrExt;

        use windows_sys::Win32::System::Threading::{
            CREATE_NEW_CONSOLE, CREATE_NEW_PROCESS_GROUP, CreateProcessW, PROCESS_INFORMATION,
            STARTUPINFOW,
        };

        let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain([0]).collect() };
        let application = wide(program.as_os_str());
        let mut line = quote(&program.to_string_lossy());
        for arg in args {
            line.push(' ');
            line.push_str(&quote(arg));
        }
        let mut command_line = wide(std::ffi::OsStr::new(&line));
        // SAFETY: all-zero is a valid STARTUPINFOW; without STARTF_USESTDHANDLES the
        // child uses its new console's handles. cb is set below.
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(0);
        // SAFETY: all-zero is a valid PROCESS_INFORMATION, filled by CreateProcessW
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: NUL-terminated strings that live across the call; no handles are
        // inherited; environment and current directory are the caller's
        let ok = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP,
                std::ptr::null(),
                std::ptr::null(),
                &raw const startup,
                &raw mut info,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        drop(Handle(info.hThread));
        drop(Handle(info.hProcess));
        Ok(info.dwProcessId)
    }

    /// Quotes a command-line argument for the C runtime's argument parsing.
    pub(super) fn quote(arg: &str) -> String {
        if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
            return arg.to_owned();
        }
        let mut quoted = String::from('"');
        let mut backslashes = 0;
        for c in arg.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                    quoted.push('"');
                    backslashes = 0;
                }
                _ => {
                    quoted.push_str(&"\\".repeat(backslashes));
                    quoted.push(c);
                    backslashes = 0;
                }
            }
        }
        quoted.push_str(&"\\".repeat(backslashes * 2));
        quoted.push('"');
        quoted
    }

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

    pub(super) fn module_path(_address: usize) -> io::Result<std::path::PathBuf> {
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
    fn module_of_a_function() {
        // in a test binary, its own functions live in the executable
        let path = module_path((module_of_a_function as *const ()).addr()).unwrap();
        assert_eq!(
            path.canonicalize().unwrap(),
            std::env::current_exe().unwrap().canonicalize().unwrap()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn maps_lines() {
        let maps = "\
55d0c0000000-55d0c0001000 r--p 00000000 08:01 1234 /usr/bin/LabVIEW
7f0000000000-7f0000100000 r-xp 00010000 08:01 5678                       /opt/lv/libSHALLserver64.so
7f0000200000-7f0000201000 rw-p 00000000 00:00 0
7f0000300000-7f0000301000 r-xp 00000000 08:01 9 /tmp/old lib.so (deleted)
";
        assert_eq!(
            imp::mapped_file(maps, 0x7f00_0000_0042),
            Some(PathBuf::from("/opt/lv/libSHALLserver64.so"))
        );
        assert_eq!(
            imp::mapped_file(maps, 0x7f00_0030_0000),
            Some(PathBuf::from("/tmp/old lib.so"))
        );
        assert_eq!(imp::mapped_file(maps, 0x7f00_0020_0000), None, "anonymous");
        assert_eq!(imp::mapped_file(maps, 0x1000), None);
    }

    #[cfg(windows)]
    #[test]
    fn command_line_quoting() {
        assert_eq!(imp::quote("--focus-pid"), "--focus-pid");
        assert_eq!(
            imp::quote(r"C:\Program Files\must\must-tui.exe"),
            r#""C:\Program Files\must\must-tui.exe""#
        );
        assert_eq!(imp::quote(r#"a"b"#), r#""a\"b""#);
        assert_eq!(
            imp::quote(r"C:\dir with space\"),
            r#""C:\dir with space\\""#
        );
        assert_eq!(imp::quote(""), "\"\"");
    }

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
