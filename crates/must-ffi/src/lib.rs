// SPDX-License-Identifier: LGPL-3.0-or-later

//! The SHALL-compatible C ABI of must: SHALL's `SECoP_S_*` and `SECoP_V_*` functions,
//! built as `SHALLserver32`/`SHALLserver64`.

mod ffi_util;
mod server;
pub(crate) mod variant;
mod variant_api;

/// Hooks for tests of the C ABI; not part of SHALL's interface.
#[doc(hidden)]
pub mod testing {
    /// Shortens how long a polled request waits for `SECoP_S_putCommandAnswer` (60 s).
    pub fn set_poll_timeout(timeout: std::time::Duration) {
        crate::server::set_poll_timeout(timeout);
    }
}
