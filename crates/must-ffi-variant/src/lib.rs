// SPDX-License-Identifier: LGPL-3.0-or-later

//! `SHALLvariant32`/`SHALLvariant64`: exports SHALL's `SECoP_V_*` functions by forwarding
//! every call to the same function of `SHALLserver`, so that all handles live in one
//! library, whichever of the two a program calls.
//!
//! `SHALLserver` is loaded on the first call: the copy already loaded in the process if
//! there is one, else the one next to this library, else whatever the system's library
//! search finds. If it cannot be loaded, every function returns its failure value.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_longlong, c_uint, c_void};
use std::sync::OnceLock;

use libloading::Library;

/// SHALL's opaque value type; only pointers to it are passed on.
type V = c_void;

#[cfg(target_pointer_width = "64")]
const SERVER: &str = "SHALLserver64";
#[cfg(target_pointer_width = "32")]
const SERVER: &str = "SHALLserver32";

/// Declares the forwarded functions: a table of `SHALLserver`'s function pointers and,
/// for each, an export calling it, returning the given failure value if `SHALLserver`
/// could not be loaded.
macro_rules! forward {
    ($(fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $(-> $ret:ty)? = $fail:expr;)*) => {
        /// The functions of `SHALLserver`.
        struct Server {
            $($name: unsafe extern "C" fn($($ty),*) $(-> $ret)?,)*
        }

        impl Server {
            fn resolve(lib: &Library) -> Result<Self, libloading::Error> {
                // SAFETY: SHALLserver exports these functions with exactly these
                // signatures (both are compiled from must-ffi), and the library is never
                // unloaded, so the pointers stay valid
                unsafe {
                    Ok(Self {
                        $($name: *lib.get(stringify!($name))?,)*
                    })
                }
            }
        }

        $(
            #[doc = concat!("Forwards to `", stringify!($name), "` of `SHALLserver`.")]
            ///
            /// # Safety
            /// As for the function of `SHALLserver`.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
                match server() {
                    // SAFETY: the same function with the caller's contract
                    Some(s) => unsafe { (s.$name)($($arg),*) },
                    None => $fail,
                }
            }
        )*
    };
}

forward! {
    fn SECoP_V_create(szDescription: *const c_char) -> *mut V = std::ptr::null_mut();
    fn SECoP_V_createSimpleBool(bValue: c_longlong) -> *mut V = std::ptr::null_mut();
    fn SECoP_V_createSimpleDouble(dValue: f64) -> *mut V = std::ptr::null_mut();
    fn SECoP_V_createSimpleInteger(llValue: c_longlong) -> *mut V = std::ptr::null_mut();
    fn SECoP_V_createSimpleScaled(dScale: f64, llValue: c_longlong) -> *mut V = std::ptr::null_mut();
    fn SECoP_V_createSimpleString(szValue: *const c_char) -> *mut V = std::ptr::null_mut();
    fn SECoP_V_createSECoPStatus() -> *mut V = std::ptr::null_mut();
    fn SECoP_V_fromJSON(szJSON: *const c_char, pHint: *const V) -> *mut V = std::ptr::null_mut();
    fn SECoP_V_copy(ppDst: *mut *mut V, pSrc: *const V) -> c_int = 0;
    fn SECoP_V_destroy(ppData: *mut *mut V) = ();
    fn SECoP_V_getType(pData: *const V, szBuffer: *mut c_char, iBufferSize: c_int) -> c_int = -1;
    fn SECoP_V_toJSON(pData: *const V, szJSONBuffer: *mut c_char, iBufferSize: c_int) -> c_int = -1;
    fn SECoP_V_printFile(pOutfile: *mut c_void, pData: *const V, iVerbosity: c_int) = ();
    fn SECoP_V_getInfo(
        pData: *const V,
        uPosition: c_uint,
        piType: *mut c_uint,
        puCount: *mut c_uint,
        pszName: *mut *const c_char,
    ) -> c_int = 0;
    fn SECoP_V_getInfo2(
        pData: *const V,
        uPosition: c_uint,
        piType: *mut c_uint,
        puCount: *mut c_uint,
        szName: *mut c_char,
        puLen: *mut c_uint,
    ) -> c_int = 0;
    fn SECoP_V_modifyMinMaxDouble(pData: *mut V, uPosition: c_uint, dblMinimum: f64, dblMaximum: f64) -> c_int = 0;
    fn SECoP_V_modifyMinMaxInteger(
        pData: *mut V,
        uPosition: c_uint,
        llMinimum: c_longlong,
        llMaximum: c_longlong,
    ) -> c_int = 0;
    fn SECoP_V_getMinMaxDouble(
        pData: *const V,
        uPosition: c_uint,
        pdblMinimum: *mut f64,
        pdblMaximum: *mut f64,
    ) -> c_int = 0;
    fn SECoP_V_getMinMaxInteger(
        pData: *const V,
        uPosition: c_uint,
        pllMinimum: *mut c_longlong,
        pllMaximum: *mut c_longlong,
    ) -> c_int = 0;
    fn SECoP_V_getArrayLength(
        pData: *const V,
        uPosition: c_uint,
        puLength: *mut c_uint,
        puMinimumLength: *mut c_uint,
        puMaximumLength: *mut c_uint,
    ) -> c_int = 0;
    fn SECoP_V_modifyArrayLength(
        pData: *mut V,
        uPosition: c_uint,
        uLength: c_uint,
        uMinimumLength: c_uint,
        uMaximumLength: c_uint,
    ) -> c_int = 0;
    fn SECoP_V_getScale(pData: *const V, uPosition: c_uint, pdblValue: *mut f64) -> c_int = 0;
    fn SECoP_V_modifyScale(pData: *mut V, uPosition: c_uint, dblScaleFactor: f64) -> c_int = 0;
    fn SECoP_V_getDouble(pData: *const V, uPosition: c_uint, uIndex: c_uint, pdblValue: *mut f64) -> c_int = 0;
    fn SECoP_V_modifyDouble(pData: *mut V, uPosition: c_uint, uIndex: c_uint, dblValue: f64) -> c_int = 0;
    fn SECoP_V_getInteger(
        pData: *const V,
        uPosition: c_uint,
        uIndex: c_uint,
        pllValue: *mut c_longlong,
    ) -> c_int = 0;
    fn SECoP_V_modifyInteger(pData: *mut V, uPosition: c_uint, uIndex: c_uint, llValue: c_longlong) -> c_int = 0;
    fn SECoP_V_getString(
        pData: *const V,
        uPosition: c_uint,
        uBufferSize: c_uint,
        pszValue: *mut c_char,
    ) -> c_uint = 0;
    fn SECoP_V_modifyString(pData: *mut V, uPosition: c_uint, szValue: *const c_char, iSize: c_int) -> c_int = 0;
    fn SECoP_V_getEnumStructCount(pData: *const V, uPosition: c_uint) -> c_uint = 0;
    fn SECoP_V_getEnumeration(
        pData: *const V,
        uPosition: c_uint,
        uIndex: c_uint,
        pllEnumVal: *mut c_longlong,
    ) -> *const c_char = std::ptr::null();
    fn SECoP_V_putEnumeration(
        pData: *mut V,
        uPosition: c_uint,
        llEnumVal: c_longlong,
        szEnumName: *const c_char,
    ) -> c_int = 0;
    fn SECoP_V_getStructName(pData: *const V, uPosition: c_uint) -> *const c_char = std::ptr::null();
    fn SECoP_V_compare(pData1: *const V, pData2: *const V) -> c_int = 3; // SECoP_VC_DIFF_TYPE
    fn SECoP_V_getAdditionalCount(pData: *const V, uPosition: c_uint) -> c_uint = 0;
    fn SECoP_V_getAdditionalIndex(pData: *const V, uPosition: c_uint, szName: *const c_char) -> c_uint = c_uint::MAX;
    fn SECoP_V_getAdditionalKey(
        pData: *const V,
        uPosition: c_uint,
        uIndex: c_uint,
        uBufferSize: c_uint,
        pszValue: *mut c_char,
    ) -> c_uint = 0;
    fn SECoP_V_getAdditionalString(
        pData: *const V,
        uPosition: c_uint,
        uIndex: c_uint,
        uBufferSize: c_uint,
        pszValue: *mut c_char,
    ) -> c_uint = 0;
    fn SECoP_V_getAdditionalDouble(pData: *const V, uPosition: c_uint, uIndex: c_uint) -> f64 = f64::NAN;
    fn SECoP_V_getAdditionalInteger(pData: *const V, uPosition: c_uint, uIndex: c_uint) -> c_longlong = 0;
}

/// `SHALLserver`'s functions, loaded on first use; `None` if it could not be loaded.
fn server() -> Option<&'static Server> {
    static SERVER_FNS: OnceLock<Option<Server>> = OnceLock::new();
    SERVER_FNS
        .get_or_init(|| {
            let loaded = load().and_then(|lib| {
                let server = Server::resolve(&lib)?;
                // never unloaded: handles and function pointers refer into it
                std::mem::forget(lib);
                Ok(server)
            });
            loaded
                .map_err(|e| eprintln!("SHALLvariant: cannot use {SERVER}: {e}"))
                .ok()
        })
        .as_ref()
}

/// Loads `SHALLserver`. On Linux the library's run path is `$ORIGIN` (see build.rs), so
/// the loader finds an already loaded copy by its SONAME first and else the file next to
/// this library.
#[cfg(unix)]
fn load() -> Result<Library, libloading::Error> {
    // SAFETY: SHALLserver has no initialisation code with preconditions
    unsafe { Library::new(libloading::library_filename(SERVER)) }
}

/// Loads `SHALLserver`: the copy already loaded in the process, else the file next to
/// this library, else via the default DLL search.
#[cfg(windows)]
fn load() -> Result<Library, libloading::Error> {
    use libloading::os::windows::Library as WinLibrary;
    let file = libloading::library_filename(SERVER);
    if let Ok(lib) = WinLibrary::open_already_loaded(&file) {
        return Ok(lib.into());
    }
    if let Some(dir) = windows::own_dir() {
        // SAFETY: SHALLserver has no initialisation code with preconditions
        if let Ok(lib) = unsafe { Library::new(dir.join(&file)) } {
            return Ok(lib);
        }
    }
    // SAFETY: as above
    unsafe { Library::new(file) }
}

#[cfg(windows)]
mod windows {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;
    const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(flags: u32, name: *const c_void, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
    }

    /// The directory of this DLL.
    pub(super) fn own_dir() -> Option<PathBuf> {
        let mut module = std::ptr::null_mut();
        // SAFETY: with FROM_ADDRESS the name is an address inside this DLL; the handle is
        // written to `module`
        let found = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                own_dir as *const c_void,
                &raw mut module,
            )
        };
        if found == 0 {
            return None;
        }
        let mut buf = vec![0u16; 32_768];
        // SAFETY: `buf` holds the given number of UTF-16 units
        let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), 32_768) } as usize;
        if len == 0 || len >= buf.len() {
            return None;
        }
        let path = PathBuf::from(OsString::from_wide(&buf[..len]));
        path.parent().map(PathBuf::from)
    }
}
