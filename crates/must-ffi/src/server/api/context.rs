// SPDX-License-Identifier: LGPL-3.0-or-later

//! Initializing the library and its contexts.

#![allow(non_snake_case)]

use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_void};

use super::{context_id, text};
use crate::ffi_util::guard;
use crate::server::codes::{
    description, err, register_user_error, remove_context_user_errors, unregister_user_error,
};
use crate::server::state::{self, Context};
use crate::server::status;

/// Initializes the library for a context. `pApplication` is ignored (no Qt); `bGUI`
/// requests the status display.
///
/// # Safety
/// `szContextID` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_initLibrary(
    _pApplication: *mut c_void,
    bGUI: c_int,
    bEnableFunctionPointers: c_int,
    szContextID: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let context = unsafe { context_id(szContextID) };
        let callbacks = bEnableFunctionPointers != 0;
        let (running, handle) = {
            let mut state = state::lock();
            if state.runtime.is_none() {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("must-shall")
                    .build();
                match runtime {
                    Ok(runtime) => state.runtime = Some(runtime),
                    Err(_) => return err::NO_MEMORY,
                }
            }
            state.default_callbacks = callbacks;
            // as in SHALL, initializing a context again starts it afresh
            let running = state.remove_context_nodes(&context);
            state.contexts.insert(context, new_context(callbacks));
            (running, state.handle())
        };
        state::stop(running, handle);
        // C and LabVIEW hosts have no tracing subscriber: capture logs for monitoring
        must_server::monitor::install_log_capture();
        if bGUI != 0 {
            status::show(true);
        }
        err::SUCCESS
    })
}

pub(super) fn new_context(callbacks: bool) -> Context {
    Context {
        callbacks,
        current: None,
        queue: VecDeque::new(),
    }
}

/// Stops the nodes of a context and forgets the context (its queued requests are
/// dropped). Unless `bNodeOnly`, the context's user error codes are removed and the
/// library's threads end when no context remains.
///
/// # Safety
/// `szContextID` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_doneLibrary(bNodeOnly: c_int, szContextID: *const c_char) {
    guard((), || {
        // SAFETY: forwarded caller contract
        let context = unsafe { context_id(szContextID) };
        let (mut running, handle, runtime) = {
            let mut state = state::lock();
            let handle = state.handle();
            let mut running = state.remove_context_nodes(&context);
            state.contexts.remove(&context);
            let mut runtime = None;
            if bNodeOnly == 0 && state.contexts.is_empty() {
                // no context means no node; clear up anything left regardless
                let all: Vec<usize> = (0..state.nodes.len()).rev().collect();
                for i in all {
                    running.extend(state.remove_node(i).running);
                }
                state.errors.clear();
                state.warnings.clear();
                runtime = state.runtime.take();
            }
            (std::mem::take(&mut running), handle, runtime)
        };
        if bNodeOnly == 0 {
            remove_context_user_errors(&context);
        }
        state::stop(std::mem::take(&mut running), handle);
        if let Some(runtime) = runtime {
            state::shut_down(runtime);
        }
    });
}

/// Accepted for compatibility; must always serves requests on several threads.
#[unsafe(no_mangle)]
pub extern "C" fn SECoP_S_setManyThreads(_bManyThreads: c_int) {}

/// Converts a `SECoP_S_error` code to a static description; `<name>: <text>` for a code
/// of [`SECoP_S_registerUserError`], valid until the process ends.
#[unsafe(no_mangle)]
pub extern "C" fn SECoP_S_getErrorString(iError: c_int) -> *const c_char {
    guard(c"".as_ptr(), || description(iError).as_ptr())
}

/// MUST addition: registers a SECoP error class (a SECoP identifier, e.g. `_Overheated`
/// or `HardwareError`) with a text for a context and returns a new code of -1000 or
/// below, which the application can return from a callback or pass to
/// `SECoP_S_putCommandAnswer[2]` for requests of that context; the client then gets an
/// error report with that class and text. Registering a name again for the context
/// returns its code and replaces the text. Fails with `SECoP_S_ERROR_INVALID_CONTEXT_ID`,
/// `SECoP_S_ERROR_INVALID_NAME` or `SECoP_S_ERROR_NAME_ALREADY_USED` (a name differing
/// only in case).
///
/// # Safety
/// The strings must be NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_registerUserError(
    szSECoPerror: *const c_char,
    szMessageText: *const c_char,
    szContextID: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let (name, message, context) = unsafe {
            (
                text(szSECoPerror),
                text(szMessageText),
                context_id(szContextID),
            )
        };
        // the state lock is released before the registry is locked
        if !state::lock().contexts.contains_key(&context) {
            return err::INVALID_CONTEXT_ID;
        }
        register_user_error(
            &context,
            name.as_deref(),
            message.as_deref().unwrap_or_default(),
        )
    })
}

/// MUST addition: unregisters a code that the context registered with
/// [`SECoP_S_registerUserError`]; it is never reused. `SECoP_S_ERROR_INVALID_VALUE` for
/// any other code.
///
/// # Safety
/// `szContextID` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_unregisterUserError(
    iError: c_int,
    szContextID: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let context = unsafe { context_id(szContextID) };
        unregister_user_error(&context, iError)
    })
}

/// Shows or hides the status display.
#[unsafe(no_mangle)]
pub extern "C" fn SECoP_S_showStatusWindow(bShowGUI: c_int) {
    guard((), || status::show(bShowGUI != 0));
}
