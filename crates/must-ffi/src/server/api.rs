// SPDX-License-Identifier: LGPL-3.0-or-later

//! SHALL's `SECoP_S_*` C functions (`SECoP-exports.h`).
//!
//! Context IDs are strings; NULL is the same context as `""`. Functions without a
//! context argument find their node by name (`SECoP_S_deleteNode`, `SECoP_S_setAddFocus`,
//! `SECoP_S_updateParameter*`) or their request by id (`SECoP_S_putCommandAnswer*`).

#![allow(non_snake_case)]

use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_longlong, c_uint, c_ulonglong, c_ushort, c_void};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use must_server::Reading;
use must_server::monitor::ExternalNode;
use must_types::{DataInfo, Value};
use serde_json::Value as Json;

use super::build::{AccessibleKind, CallFunction, CheckFunction, DraftNode, GetSet, valid_name};
use super::codes::{
    description, err, register_user_error, remove_context_user_errors, unregister_user_error,
};
use super::filter::Filter;
use super::handler::{Answer, AnswerValue};
use super::state::{self, Context, NodeEntry, Pending, State};
use super::status;
use crate::ffi_util::{c_bytes, copy_out, guard, put, sized_bytes};
use crate::variant::{CSECoPbaseType, clone_handle, new_handle};

/// A context ID argument; NULL is the default context `""`.
///
/// # Safety
/// `s` must be NULL or a NUL-terminated string.
unsafe fn context_id(s: *const c_char) -> String {
    // SAFETY: forwarded caller contract
    unsafe { c_bytes(s) }.map_or_else(String::new, |b| String::from_utf8_lossy(b).into_owned())
}

/// A string argument, `None` for NULL.
///
/// # Safety
/// `s` must be NULL or a NUL-terminated string.
unsafe fn text(s: *const c_char) -> Option<String> {
    // SAFETY: forwarded caller contract
    unsafe { c_bytes(s) }.map(|b| String::from_utf8_lossy(b).into_owned())
}

/// JSON from SHALL's `(const char*, int size)` arguments; `Ok(None)` for NULL or empty.
/// As in SHALL, the text ends at the first NUL, so a size that counts the terminator or
/// a whole buffer works.
///
/// # Safety
/// See [`sized_bytes`].
unsafe fn sized_json(s: *const c_char, size: c_int) -> Result<Option<Json>, c_int> {
    // SAFETY: forwarded caller contract
    let text = unsafe { sized_bytes(s, size) }.map(|b| b.split(|&c| c == 0).next().unwrap_or(b));
    match text {
        None => Ok(None),
        Some(b) if b.iter().all(u8::is_ascii_whitespace) => Ok(None),
        Some(b) => serde_json::from_slice(b)
            .map(Some)
            .map_err(|_| err::INVALID_VALUE),
    }
}

/// A copy of the node of a handle, as a value from the application; `Ok(None)` for NULL.
fn handle_value(p: *const CSECoPbaseType) -> Result<Option<AnswerValue>, c_int> {
    if p.is_null() {
        return Ok(None);
    }
    clone_handle(p)
        .map(|n| Some(AnswerValue::Node(n)))
        .ok_or(err::INVALID_VALUE)
}

/// A matrix from SHALL-style plain arguments: `dims` lengths at `len` and `size` bytes
/// at `data`, copied once. `INVALID_VALUE` for negative sizes, or NULL with a size.
///
/// # Safety
/// `len` must be NULL or valid for `dims` reads, `data` NULL or valid for `size` bytes.
unsafe fn matrix_arguments(
    len: *const c_uint,
    dims: c_int,
    data: *const c_void,
    size: c_int,
) -> Result<Value, c_int> {
    let dims = usize::try_from(dims).map_err(|_| err::INVALID_VALUE)?;
    let size = usize::try_from(size).map_err(|_| err::INVALID_VALUE)?;
    if (len.is_null() && dims > 0) || (data.is_null() && size > 0) {
        return Err(err::INVALID_VALUE);
    }
    let len = if dims == 0 {
        Vec::new()
    } else {
        // SAFETY: non-NULL and valid for `dims` reads per the caller's contract
        unsafe { std::slice::from_raw_parts(len, dims) }.to_vec()
    };
    let data = if size == 0 {
        Vec::new()
    } else {
        // SAFETY: non-NULL and valid for `size` bytes per the caller's contract
        unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) }.to_vec()
    };
    Ok(Value::Matrix { len, data })
}

/// Checks the lengths and byte count of a matrix against `info` before any bytes are
/// copied: `INVALID_VALUE` unless `info` is a matrix the shape fits.
///
/// # Safety
/// `len` must be NULL or valid for `dims` reads.
unsafe fn check_matrix_shape(
    info: Option<&DataInfo>,
    len: *const c_uint,
    dims: c_int,
    size: c_int,
) -> Result<(), c_int> {
    let Some(DataInfo::Matrix(m)) = info else {
        return Err(err::INVALID_VALUE);
    };
    let dims = usize::try_from(dims).map_err(|_| err::INVALID_VALUE)?;
    let size = usize::try_from(size).map_err(|_| err::INVALID_VALUE)?;
    if len.is_null() && dims > 0 {
        return Err(err::INVALID_VALUE);
    }
    let len = if dims == 0 {
        &[][..]
    } else {
        // SAFETY: non-NULL and valid for `dims` reads per the caller's contract
        unsafe { std::slice::from_raw_parts(len, dims) }
    };
    m.check(len, size, must_types::Mode::Report)
        .map_err(|_| err::INVALID_VALUE)
}

/// The value of a handle as JSON; `Ok(None)` for NULL.
fn handle_json(p: *const CSECoPbaseType) -> Result<Option<Json>, c_int> {
    if p.is_null() {
        return Ok(None);
    }
    clone_handle(p)
        .map(|n| Some(n.export_value()))
        .ok_or(err::INVALID_VALUE)
}

/// Locks the state and checks that the library is initialized.
fn initialized() -> Result<std::sync::MutexGuard<'static, State>, c_int> {
    let state = state::lock();
    if state.initialized() {
        Ok(state)
    } else {
        Err(err::NOT_INITIALIZED)
    }
}

fn code(result: Result<c_int, c_int>) -> c_int {
    result.unwrap_or_else(|e| e)
}

// --- library and contexts

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

fn new_context(callbacks: bool) -> Context {
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

/// must addition: registers a SECoP error class (a SECoP identifier, e.g. `_Overheated`
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

/// must addition: unregisters a code that the context registered with
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

// --- building nodes

/// Creates a node listening on all interfaces.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_createNode(
    szID: *const c_char,
    szDesc: *const c_char,
    wPort: c_ushort,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    unsafe { SECoP_S_createNode2(szID, szDesc, std::ptr::null(), wPort, szContextID) }
}

/// Creates a node listening on `szInterface` (an IP address; NULL or empty for all).
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_createNode2(
    szID: *const c_char,
    szDesc: *const c_char,
    szInterface: *const c_char,
    wPort: c_ushort,
    szContextID: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let (id, desc, interface, context) = unsafe {
            (
                text(szID),
                text(szDesc).unwrap_or_default(),
                text(szInterface).unwrap_or_default(),
                context_id(szContextID),
            )
        };
        code(create_node(id, &desc, &interface, wPort, context))
    })
}

fn create_node(
    id: Option<String>,
    desc: &str,
    interface: &str,
    port: u16,
    context: String,
) -> Result<c_int, c_int> {
    let mut state = initialized()?;
    // SHALL's code for a missing or invalid node name
    let id = id
        .filter(|id| valid_name(id))
        .ok_or(err::MISSING_MANDATORY)?;
    if state.node_index(&id).is_some() {
        return Err(err::NAME_ALREADY_USED);
    }
    let ip = match interface.trim() {
        "" => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        "localhost" => IpAddr::V4(Ipv4Addr::LOCALHOST),
        // as in SHALL, an interface that is not an IP address means all interfaces
        other => other.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
    };
    let default_callbacks = state.default_callbacks;
    let ctx = state
        .contexts
        .entry(context.clone())
        .or_insert_with(|| new_context(default_callbacks));
    ctx.current = Some(id.clone());
    let callbacks = ctx.callbacks;
    let monitor = ExternalNode::new(&id, Some(context.as_str()).filter(|c| !c.is_empty()));
    state.nodes.push(NodeEntry {
        context,
        callbacks,
        draft: DraftNode::new(&id, desc, SocketAddr::new(ip, port)),
        running: None,
        monitor,
    });
    if desc.is_empty() {
        let code = err::WARNING_NO_DESCRIPTION;
        state.note(
            code,
            format!("{id}: {}", description(code).to_string_lossy()),
        );
        Ok(code)
    } else {
        Ok(err::SUCCESS)
    }
}

/// Stops and removes a node.
///
/// # Safety
/// `szID` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_deleteNode(szID: *const c_char) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let id = unsafe { text(szID) }.unwrap_or_default();
        let (running, handle) = {
            let mut state = match initialized() {
                Ok(s) => s,
                Err(e) => return e,
            };
            let Some(index) = state.node_index(&id) else {
                return err::INVALID_NODE;
            };
            let entry = state.remove_node(index);
            (entry.running, state.handle())
        };
        state::stop(running.into_iter().collect(), handle);
        err::SUCCESS
    })
}

/// Runs a building step on the node a context builds, recording non-zero results for
/// `SECoP_S_showErrors`.
///
/// # Safety
/// `context` must be NULL or a NUL-terminated string.
unsafe fn build_step(context: *const c_char, f: impl FnOnce(&mut DraftNode) -> c_int) -> c_int {
    // SAFETY: forwarded caller contract
    let context = unsafe { context_id(context) };
    guard(err::INTERNAL, || {
        let mut state = match initialized() {
            Ok(s) => s,
            Err(e) => return e,
        };
        if !state.contexts.contains_key(&context) {
            return err::INVALID_CONTEXT_ID;
        }
        let Some(entry) = state.current_node(&context) else {
            return err::INVALID_NODE;
        };
        if entry.running.is_some() {
            // a complete node cannot change
            return err::INVALID_NODE;
        }
        let code = f(&mut entry.draft);
        let path = entry.draft.focus_path();
        state.note(
            code,
            format!("{path}: {}", description(code).to_string_lossy()),
        );
        code
    })
}

/// Adds a module to the node of the context and focuses it.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addModule(
    szName: *const c_char,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    let name = unsafe { text(szName) }.unwrap_or_default();
    // SAFETY: forwarded caller contract
    unsafe { build_step(szContextID, |node| node.add_module(&name)) }
}

/// Adds a command to the focused module and focuses it.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings; `ptrToFunc` must be NULL
/// or a function with SHALL's `SECoP_S_callFunction` signature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addCommand(
    szKey: *const c_char,
    ptrToFunc: Option<CallFunction>,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    let name = unsafe { text(szKey) }.unwrap_or_default();
    let kind = AccessibleKind::Command {
        function: ptrToFunc,
    };
    // SAFETY: forwarded caller contract
    unsafe { build_step(szContextID, |node| node.add_accessible(&name, kind)) }
}

/// Adds a read-only parameter to the focused module and focuses it.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings; `ptrToGet` must be NULL
/// or a function with SHALL's `SECoP_S_getsetFunction` signature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addReadableParameter(
    szName: *const c_char,
    ptrToGet: Option<GetSet>,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    unsafe { add_parameter(szName, false, ptrToGet, None, szContextID) }
}

/// Adds a writable parameter to the focused module and focuses it.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings; the functions must be
/// NULL or have SHALL's `SECoP_S_getsetFunction` signature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addWritableParameter(
    szName: *const c_char,
    ptrToGet: Option<GetSet>,
    ptrToSet: Option<GetSet>,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    unsafe { add_parameter(szName, true, ptrToGet, ptrToSet, szContextID) }
}

/// # Safety
/// See [`SECoP_S_addWritableParameter`].
unsafe fn add_parameter(
    name: *const c_char,
    writable: bool,
    getter: Option<GetSet>,
    setter: Option<GetSet>,
    context: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    let name = unsafe { text(name) }.unwrap_or_default();
    let kind = AccessibleKind::Parameter {
        writable,
        getter,
        setter,
        initial: None,
    };
    // SAFETY: forwarded caller contract
    unsafe { build_step(context, |node| node.add_accessible(&name, kind)) }
}

/// Sets a property of the focused item.
///
/// # Safety
/// `szKey` and `szContextID` must be NULL or NUL-terminated strings.
unsafe fn add_property(
    key: *const c_char,
    value: Result<Json, c_int>,
    context: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    let key = unsafe { text(key) }.unwrap_or_default();
    // SAFETY: forwarded caller contract
    unsafe {
        build_step(context, |node| match value {
            Ok(value) => node.add_property(&key, value),
            Err(code) => code,
        })
    }
}

/// Sets a property of the focused item to the value of a handle.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addProperty(
    szKey: *const c_char,
    pValue: *const CSECoPbaseType,
    szContextID: *const c_char,
) -> c_int {
    let value = handle_json(pValue).and_then(|v| v.ok_or(err::INVALID_VALUE));
    // SAFETY: forwarded caller contract
    unsafe { add_property(szKey, value, szContextID) }
}

/// Sets a boolean property of the focused item.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addPropertyBoolean(
    szKey: *const c_char,
    bValue: c_longlong,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    unsafe { add_property(szKey, Ok(Json::Bool(bValue != 0)), szContextID) }
}

/// Sets an integer property of the focused item.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addPropertyInteger(
    szKey: *const c_char,
    llValue: c_longlong,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    unsafe { add_property(szKey, Ok(Json::from(llValue)), szContextID) }
}

/// Sets a floating point property of the focused item.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addPropertyDouble(
    szKey: *const c_char,
    dblValue: f64,
    szContextID: *const c_char,
) -> c_int {
    let value = serde_json::Number::from_f64(dblValue)
        .map(Json::Number)
        .ok_or(err::INVALID_VALUE);
    // SAFETY: forwarded caller contract
    unsafe { add_property(szKey, value, szContextID) }
}

/// Sets a string property of the focused item.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addPropertyString(
    szKey: *const c_char,
    szValue: *const c_char,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    let value = unsafe { text(szValue) }
        .map(Json::String)
        .ok_or(err::INVALID_VALUE);
    // SAFETY: forwarded caller contract
    unsafe { add_property(szKey, value, szContextID) }
}

/// Sets a property of the focused item to a value given as JSON text.
///
/// # Safety
/// The string arguments must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_addPropertyJSON(
    szKey: *const c_char,
    szValue: *const c_char,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    let value = unsafe { sized_json(szValue, -1) }.and_then(|v| v.ok_or(err::INVALID_VALUE));
    // SAFETY: forwarded caller contract
    unsafe { add_property(szKey, value, szContextID) }
}

/// Focuses an item by `node`, `node:module` or `node:module:accessible`; later building
/// calls of the node's context apply to it.
///
/// # Safety
/// `szKey` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_setAddFocus(szKey: *const c_char) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let key = unsafe { text(szKey) }.unwrap_or_default();
        let mut state = match initialized() {
            Ok(s) => s,
            Err(e) => return e,
        };
        let (node, rest) = key.split_once(':').unwrap_or((&key, ""));
        let Some(index) = state.node_index(node) else {
            return err::INVALID_NODE;
        };
        let entry = &mut state.nodes[index];
        let code = entry.draft.set_focus(rest);
        if code == err::SUCCESS {
            let (context, name) = (entry.context.clone(), entry.draft.name.clone());
            if let Some(c) = state.contexts.get_mut(&context) {
                c.current = Some(name);
            }
        }
        code
    })
}

/// A must addition, not in SHALL: sets the check function of the focused parameter or
/// command and publishes `checkable: true` for it; NULL clears both.
///
/// # Safety
/// `szContextID` must be NULL or a NUL-terminated string; `ptrToCheck` must be NULL or a
/// function with the `SECoP_S_checkFunction` signature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_setCheckFunction(
    ptrToCheck: Option<CheckFunction>,
    szContextID: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract
    unsafe { build_step(szContextID, |node| node.set_check(ptrToCheck)) }
}

/// Validates the node of the context and starts serving it.
///
/// # Safety
/// `szContextID` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_nodeComplete(szContextID: *const c_char) -> c_int {
    // SAFETY: forwarded caller contract
    let context = unsafe { context_id(szContextID) };
    guard(err::INTERNAL, || code(complete(&context)))
}

fn complete(context: &str) -> Result<c_int, c_int> {
    let ((node, notes), name, handle) = {
        let mut state = initialized()?;
        if !state.contexts.contains_key(context) {
            return Err(err::INVALID_CONTEXT_ID);
        }
        let entry = state.current_node(context).ok_or(err::INVALID_NODE)?;
        if entry.running.is_some() {
            return Ok(err::SUCCESS);
        }
        let built = entry.draft.build(&entry.context, entry.callbacks);
        let name = entry.draft.name.clone();
        match built {
            Ok(built) => (built, name, state.handle().ok_or(err::NOT_INITIALIZED)?),
            Err(problems) => {
                let code = problems.first().map_or(err::INTERNAL, |p| p.code);
                let messages = problems.iter().map(|p| p.message.clone()).collect();
                if let Some(monitor) = &entry.monitor {
                    monitor.failed(messages);
                }
                for p in problems {
                    state.note(p.code, p.message);
                }
                return Err(code);
            }
        }
    };
    let mut warnings = notes;
    warnings.extend(node.warnings().iter().map(ToString::to_string));
    // started without the lock: the node may call the application at once
    let running = node.spawn(&handle).map_err(|e| {
        let message = format!("{name}: {e}");
        let mut state = state::lock();
        if let Some(i) = state.node_index(&name)
            && let Some(monitor) = &state.nodes[i].monitor
        {
            monitor.failed(vec![message.clone()]);
        }
        state.note(err::COMM_FAILED, message);
        err::COMM_FAILED
    })?;
    let mut state = state::lock();
    match state.node_index(&name) {
        Some(i) if state.nodes[i].running.is_none() => {
            if let Some(monitor) = &state.nodes[i].monitor {
                monitor.running(&running, warnings.clone());
            }
            state.nodes[i].running = Some(running);
        }
        _ => {
            // deleted (or completed by another thread) meanwhile
            drop(state);
            state::stop(vec![running], Some(handle));
            return Err(err::INVALID_NODE);
        }
    }
    if warnings.is_empty() {
        Ok(err::SUCCESS)
    } else {
        for warning in warnings {
            state.warnings.push(warning);
        }
        Ok(err::WARNING_MISS_PROPERTIES)
    }
}

/// Prints (to stderr) and clears the errors and warnings recorded while building nodes.
#[unsafe(no_mangle)]
pub extern "C" fn SECoP_S_showErrors() {
    guard((), || {
        let ((dropped_errors, errors), (dropped_warnings, warnings)) = {
            let mut state = state::lock();
            (state.errors.take(), state.warnings.take())
        };
        if dropped_errors > 0 {
            eprintln!("SECoP error: {dropped_errors} earlier errors were dropped");
        }
        for e in errors {
            eprintln!("SECoP error: {e}");
        }
        if dropped_warnings > 0 {
            eprintln!("SECoP warning: {dropped_warnings} earlier warnings were dropped");
        }
        for w in warnings {
            eprintln!("SECoP warning: {w}");
        }
    });
}

// --- runtime

/// Pushes a new value of a parameter (`node:module:parameter`); before
/// `SECoP_S_nodeComplete` it becomes the parameter's initial value.
///
/// # Safety
/// `szParameterName` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_updateParameter(
    szParameterName: *const c_char,
    pData: *const CSECoPbaseType,
    pSigma: *const CSECoPbaseType,
    dblTimestamp: f64,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let name = unsafe { text(szParameterName) }.unwrap_or_default();
        code((|| {
            let value = handle_value(pData)?.ok_or(err::INVALID_VALUE)?;
            let sigma = handle_json(pSigma)?;
            update(&name, |_| Ok(value), sigma, dblTimestamp)
        })())
    })
}

/// Pushes a new value of a parameter given as JSON text, see
/// [`SECoP_S_updateParameter`].
///
/// # Safety
/// The strings must be NULL, or valid for their size (NUL-terminated if negative).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_updateParameter2(
    szParameterName: *const c_char,
    szValue: *const c_char,
    iValueSize: c_int,
    szSigma: *const c_char,
    iSigmaSize: c_int,
    dblTimestamp: f64,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let name = unsafe { text(szParameterName) }.unwrap_or_default();
        code((|| {
            // SAFETY: forwarded caller contract
            let value = unsafe { sized_json(szValue, iValueSize) }?.ok_or(err::INVALID_VALUE)?;
            // SAFETY: forwarded caller contract
            let sigma = unsafe { sized_json(szSigma, iSigmaSize) }?;
            update(&name, |_| Ok(AnswerValue::Json(value)), sigma, dblTimestamp)
        })())
    })
}

/// must addition: pushes a new matrix value of a parameter (`node:module:parameter`)
/// from plain arguments, see [`SECoP_S_updateParameter`]: `iDims` lengths at `puLen` and
/// `iDataSize` raw element bytes at `pData`, which are copied once and not kept. Name
/// errors give the codes of `SECoP_S_updateParameter2`; a parameter that is not a matrix,
/// or lengths and bytes that do not fit its datainfo, give
/// `SECoP_S_ERROR_INVALID_VALUE`, and nothing is sent.
///
/// # Safety
/// `szParameterName` must be NULL or a NUL-terminated string; `puLen` NULL or valid for
/// `iDims` reads; `pData` NULL or valid for `iDataSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_updateParameterMatrix(
    szParameterName: *const c_char,
    puLen: *const c_uint,
    iDims: c_int,
    pData: *const c_void,
    iDataSize: c_int,
    dblTimestamp: f64,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let name = unsafe { text(szParameterName) }.unwrap_or_default();
        let value = |info: Option<&DataInfo>| {
            // SAFETY: forwarded caller contract
            unsafe {
                check_matrix_shape(info, puLen, iDims, iDataSize)?;
                matrix_arguments(puLen, iDims, pData, iDataSize).map(AnswerValue::Value)
            }
        };
        code(update(&name, value, None, dblTimestamp))
    })
}

/// Pushes a new value of a parameter. `value` makes the value once the name is resolved,
/// from the parameter's datainfo (`None` if it has none yet).
fn update(
    name: &str,
    value: impl FnOnce(Option<&DataInfo>) -> Result<AnswerValue, c_int>,
    sigma: Option<Json>,
    t: f64,
) -> Result<c_int, c_int> {
    // SHALL's codes: an empty name, then the first missing part
    if name.is_empty() {
        return Err(err::INVALID_NAME);
    }
    let (node, rest) = name.split_once(':').ok_or(err::INVALID_NODE)?;
    let (module, parameter) = rest.split_once(':').ok_or(err::INVALID_MODULE)?;
    let mut state = initialized()?;
    let index = state.node_index(node).ok_or(err::INVALID_NODE)?;
    let entry = &mut state.nodes[index];
    let m = entry
        .draft
        .module_index(module)
        .ok_or(err::INVALID_MODULE)?;
    let draft = &mut entry.draft.modules[m];
    let a = draft
        .accessible_index(parameter)
        .ok_or(err::INVALID_PARAMETER)?;
    let module = draft.name.clone();
    let acc = &mut draft.accessibles[a];
    // as in SHALL, a constant can't be updated
    if acc.props.contains_key("constant") {
        return Err(err::INVALID_PARAMETER);
    }
    let AccessibleKind::Parameter { initial, .. } = &mut acc.kind else {
        return Err(err::INVALID_PARAMETER);
    };
    let Some(running) = &entry.running else {
        // checked against the datainfo when the node is complete
        let info = acc
            .datainfo
            .as_ref()
            .and_then(crate::variant::Node::to_datainfo);
        let value = value(info.as_ref())?
            .into_json(info.as_ref())
            .ok_or(err::INVALID_VALUE)?;
        *initial = Some(value);
        return Ok(err::SUCCESS);
    };
    if acc.info.is_none() {
        acc.info = acc
            .datainfo
            .as_ref()
            .and_then(crate::variant::Node::to_datainfo);
    }
    let info = acc.info.as_ref();
    let value = value(info)?;
    let info = info.ok_or(err::INVALID_VALUE)?;
    let value = value.into_value(info).map_err(|_| err::INVALID_VALUE)?;
    let mut reading = Reading::new(value);
    if t.is_finite() && t > 0.0 {
        reading = reading.at(t);
    }
    if let Some(sigma) = sigma {
        reading = reading.with_uncertainty(sigma);
    }
    running
        .update(&module, &acc.name, reading)
        .map(|()| err::SUCCESS)
        .map_err(|_| err::INVALID_VALUE)
}

/// What `SECoP_S_getStoredCommand[2]` hands out.
struct Request {
    id: u64,
    action: c_int,
    name: String,
    /// The value of a `change`, the argument of a `do`.
    value: Option<crate::variant::Node>,
}

/// Takes the next polled request of a context that `filter` selects (any without
/// filter); from now on it waits for its answer.
fn next_request(context: &str, filter: Option<&Filter>) -> Result<Request, c_int> {
    let mut state = initialized()?;
    let id = state.take_matching(context, filter)?;
    let pending = state.handed_out.get_mut(&id).ok_or(err::INTERNAL)?;
    Ok(Request {
        id,
        action: pending.action,
        name: format!("{}:{}:{}", pending.node, pending.module, pending.accessible),
        // handed out once, so moved rather than copied (a matrix can be large)
        value: pending.value.take(),
    })
}

/// Hands out a request through the caller's buffers, as `SECoP_S_getStoredCommand`
/// does: id, action, name, and the value as a new handle.
///
/// # Safety
/// Pointers must be NULL or valid; `name` for `*name_size` bytes.
unsafe fn hand_out(
    request: Request,
    id: *mut c_ulonglong,
    action: *mut c_int,
    name: *mut c_char,
    name_size: *mut c_int,
    value: *mut *mut CSECoPbaseType,
) -> c_int {
    // SAFETY: forwarded caller contract
    let fits = unsafe {
        put(id, request.id);
        put(action, request.action);
        // only made for a caller that takes it: nobody could destroy it otherwise
        if !value.is_null() {
            value.write(request.value.map_or(std::ptr::null_mut(), new_handle));
        }
        write_text(request.name.as_bytes(), name, name_size)
    };
    if fits {
        err::SUCCESS
    } else {
        err::WARNING_BUFFER_TOO_SMALL
    }
}

/// Hands out a request through the caller's buffers, as `SECoP_S_getStoredCommand2`
/// does: id, action, name, and the value as JSON text.
///
/// # Safety
/// Pointers must be NULL or valid; the buffers for their given sizes.
unsafe fn hand_out_text(
    request: &Request,
    id: *mut c_ulonglong,
    action: *mut c_int,
    name: *mut c_char,
    name_size: *mut c_int,
    value: *mut c_char,
    value_size: *mut c_int,
) -> c_int {
    let text = request
        .value
        .as_ref()
        .map(|v| serde_json::to_vec(&v.export_value()).unwrap_or_default())
        .unwrap_or_default();
    // SAFETY: forwarded caller contract
    let fits = unsafe {
        put(id, request.id);
        put(action, request.action);
        let name = write_text(request.name.as_bytes(), name, name_size);
        let value = write_text(&text, value, value_size);
        name && value
    };
    if fits {
        err::SUCCESS
    } else {
        err::WARNING_BUFFER_TOO_SMALL
    }
}

/// Writes `data` to a caller's buffer of `*size` bytes, NUL-terminated and cut if
/// needed, and sets `*size` to the length of `data` (without NUL), as SHALL does.
/// Returns whether it fit.
///
/// # Safety
/// `buffer` must be NULL or valid for writes of `*size` bytes; `size` NULL or valid.
unsafe fn write_text(data: &[u8], buffer: *mut c_char, size: *mut c_int) -> bool {
    // SAFETY: forwarded caller contract
    unsafe {
        let capacity = if size.is_null() {
            0
        } else {
            usize::try_from(size.read()).unwrap_or(0)
        };
        copy_out(data, buffer, capacity);
        put(size, c_int::try_from(data.len()).unwrap_or(c_int::MAX));
        data.len() < capacity
    }
}

/// Hands out the next polled request of a context: its id, action, name
/// (`node:module:accessible`) and, as a new handle the caller destroys, the value of a
/// `change` or the argument of a `do` (NULL for a `read`). A name buffer that is too
/// small gives `SECoP_S_WARNING_BUFFER_TOO_SMALL`; the request is handed out anyway.
///
/// # Safety
/// Pointers must be NULL or valid; `szParameter` for `*piParameterSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_getStoredCommand(
    pllId: *mut c_ulonglong,
    piAction: *mut c_int,
    szParameter: *mut c_char,
    piParameterSize: *mut c_int,
    ppValue: *mut *mut CSECoPbaseType,
    szContextID: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let context = unsafe { context_id(szContextID) };
        match next_request(&context, None) {
            // SAFETY: forwarded caller contract
            Ok(request) => unsafe {
                hand_out(
                    request,
                    pllId,
                    piAction,
                    szParameter,
                    piParameterSize,
                    ppValue,
                )
            },
            Err(e) => e,
        }
    })
}

/// Hands out the next polled request of a context like
/// [`SECoP_S_getStoredCommand`], with the value as JSON text (empty for a `read` and a
/// `do` without argument).
///
/// # Safety
/// Pointers must be NULL or valid; the buffers for their given sizes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_getStoredCommand2(
    pllId: *mut c_ulonglong,
    piAction: *mut c_int,
    szParameter: *mut c_char,
    piParameterSize: *mut c_int,
    szValue: *mut c_char,
    piValueSize: *mut c_int,
    szContextID: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let context = unsafe { context_id(szContextID) };
        match next_request(&context, None) {
            // SAFETY: forwarded caller contract
            Ok(request) => unsafe {
                hand_out_text(
                    &request,
                    pllId,
                    piAction,
                    szParameter,
                    piParameterSize,
                    szValue,
                    piValueSize,
                )
            },
            Err(e) => e,
        }
    })
}

/// The filter of `SECoP_S_filterStoredCommand[2]`.
///
/// # Safety
/// `accessibles` must be NULL or a NUL-terminated string.
unsafe fn filter(accessibles: *const c_char, action: c_int) -> Result<Filter, c_int> {
    // SAFETY: forwarded caller contract
    let accessibles = unsafe { text(accessibles) };
    Filter::parse(accessibles.as_deref(), action)
}

/// must addition: hands out the oldest polled request of a context that the filter
/// selects, like [`SECoP_S_getStoredCommand`]. The filter comes first, as SHALL-LabVIEW's
/// `SECoP-filterStoredCommand.vi` passes it, and the context ID last.
///
/// # Safety
/// Pointers must be NULL or valid; strings NUL-terminated; `szAccessible` for
/// `*piAccessibleSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_filterStoredCommand(
    szFilterAccessibles: *const c_char,
    iFilterAction: c_int,
    pllId: *mut c_ulonglong,
    piAction: *mut c_int,
    szAccessible: *mut c_char,
    piAccessibleSize: *mut c_int,
    ppValue: *mut *mut CSECoPbaseType,
    szContextID: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let (context, filter) = unsafe {
            (
                context_id(szContextID),
                filter(szFilterAccessibles, iFilterAction),
            )
        };
        match filter.and_then(|f| next_request(&context, Some(&f))) {
            // SAFETY: forwarded caller contract
            Ok(request) => unsafe {
                hand_out(
                    request,
                    pllId,
                    piAction,
                    szAccessible,
                    piAccessibleSize,
                    ppValue,
                )
            },
            Err(e) => e,
        }
    })
}

/// must addition: hands out the oldest polled request of a context that the filter
/// selects, like [`SECoP_S_getStoredCommand2`]; see [`SECoP_S_filterStoredCommand`].
///
/// # Safety
/// Pointers must be NULL or valid; strings NUL-terminated; the buffers for their given
/// sizes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_filterStoredCommand2(
    szFilterAccessibles: *const c_char,
    iFilterAction: c_int,
    pllId: *mut c_ulonglong,
    piAction: *mut c_int,
    szAccessible: *mut c_char,
    piAccessibleSize: *mut c_int,
    szValue: *mut c_char,
    piValueSize: *mut c_int,
    szContextID: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let (context, filter) = unsafe {
            (
                context_id(szContextID),
                filter(szFilterAccessibles, iFilterAction),
            )
        };
        match filter.and_then(|f| next_request(&context, Some(&f))) {
            // SAFETY: forwarded caller contract
            Ok(request) => unsafe {
                hand_out_text(
                    &request,
                    pllId,
                    piAction,
                    szAccessible,
                    piAccessibleSize,
                    szValue,
                    piValueSize,
                )
            },
            Err(e) => e,
        }
    })
}

/// Answers a polled request: an error code, and for success the value (a `read` or the
/// value set by a `change`; a `change` without value reports the requested one) or the
/// result of a `do`.
///
/// # Safety
/// Always safe to call with NULL or valid handles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_putCommandAnswer(
    llId: c_ulonglong,
    iErrorCode: c_int,
    pValue: *const CSECoPbaseType,
    pSigma: *const CSECoPbaseType,
    dblTimestamp: f64,
) -> c_int {
    guard(err::INTERNAL, || {
        code((|| {
            let value = handle_value(pValue)?;
            let sigma = handle_json(pSigma)?;
            answer(llId, iErrorCode, value, sigma, dblTimestamp)
        })())
    })
}

/// Answers a polled request with JSON text, see [`SECoP_S_putCommandAnswer`].
///
/// # Safety
/// The strings must be NULL, or valid for their size (NUL-terminated if negative).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_putCommandAnswer2(
    llId: c_ulonglong,
    iErrorCode: c_int,
    szValue: *const c_char,
    iValueSize: c_int,
    szSigma: *const c_char,
    iSigmaSize: c_int,
    dblTimestamp: f64,
) -> c_int {
    guard(err::INTERNAL, || {
        code((|| {
            // SAFETY: forwarded caller contract
            let value = unsafe { sized_json(szValue, iValueSize) }?;
            // SAFETY: forwarded caller contract
            let sigma = unsafe { sized_json(szSigma, iSigmaSize) }?;
            answer(
                llId,
                iErrorCode,
                value.map(AnswerValue::Json),
                sigma,
                dblTimestamp,
            )
        })())
    })
}

/// must addition: answers a polled request with a matrix from plain arguments, see
/// [`SECoP_S_putCommandAnswer`]: `iDims` lengths at `puLen` and `iDataSize` raw element
/// bytes at `pData`, copied once and not kept, as the read value, the value after a
/// change or the command result. With a negative `iErrorCode` the matrix arguments are
/// not read and may be NULL. A request whose value is not a matrix, or lengths and bytes
/// that do not fit, give `SECoP_S_ERROR_INVALID_VALUE`, and the request stays handed out.
///
/// # Safety
/// `puLen` must be NULL or valid for `iDims` reads; `pData` NULL or valid for
/// `iDataSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_putCommandAnswerMatrix(
    llId: c_ulonglong,
    iErrorCode: c_int,
    puLen: *const c_uint,
    iDims: c_int,
    pData: *const c_void,
    iDataSize: c_int,
    dblTimestamp: f64,
) -> c_int {
    guard(err::INTERNAL, || {
        code((|| {
            if iErrorCode < 0 {
                return answer(llId, iErrorCode, None, None, dblTimestamp);
            }
            {
                let mut state = initialized()?;
                let info = answer_info(&mut state, llId)?;
                // SAFETY: forwarded caller contract
                unsafe { check_matrix_shape(info.as_ref(), puLen, iDims, iDataSize) }?;
            }
            // SAFETY: forwarded caller contract
            let value = unsafe { matrix_arguments(puLen, iDims, pData, iDataSize) }?;
            answer(
                llId,
                iErrorCode,
                Some(AnswerValue::Value(value)),
                None,
                dblTimestamp,
            )
        })())
    })
}

/// The datainfo of the value that answers the handed-out request `id`: the parameter's,
/// or the result of a command (`None` for a command without result).
fn answer_info(state: &mut State, id: u64) -> Result<Option<DataInfo>, c_int> {
    let pending = state.handed_out.get(&id).ok_or(err::INVALID_NAME)?;
    let (node, module, accessible) = (
        pending.node.clone(),
        pending.module.clone(),
        pending.accessible.clone(),
    );
    let entry = state
        .node_index(&node)
        .map(|i| &mut state.nodes[i])
        .ok_or(err::INVALID_NAME)?;
    let draft = entry
        .draft
        .module_index(&module)
        .map(|m| &mut entry.draft.modules[m])
        .ok_or(err::INVALID_NAME)?;
    let acc = draft
        .accessible_index(&accessible)
        .map(|a| &mut draft.accessibles[a])
        .ok_or(err::INVALID_NAME)?;
    if acc.info.is_none() {
        acc.info = acc
            .datainfo
            .as_ref()
            .and_then(crate::variant::Node::to_datainfo);
    }
    Ok(match &acc.info {
        Some(DataInfo::Command(c)) => c.result().cloned(),
        other => other.clone(),
    })
}

fn answer(
    id: u64,
    code: c_int,
    value: Option<AnswerValue>,
    sigma: Option<Json>,
    t: f64,
) -> Result<c_int, c_int> {
    // unknown: never handed out, answered before, or timed out
    let pending: Pending = initialized()?
        .handed_out
        .remove(&id)
        .ok_or(err::INVALID_NAME)?;
    let answer = Answer {
        code,
        value,
        sigma,
        t,
    };
    // the request may have timed out meanwhile
    pending
        .reply
        .send(answer)
        .map(|()| err::SUCCESS)
        .map_err(|_| err::INVALID_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::lock;
    use serde_json::json;
    use tokio::sync::oneshot;

    /// A context of its own: the library state is global and shared with other tests.
    const CONTEXT: &std::ffi::CStr = c"api-tests-matrix";
    const IMAGE: &std::ffi::CStr =
        cr#"{"type":"matrix","elementtype":"<f4","names":["x","y"],"maxlen":[100,100]}"#;
    const LEN: [c_uint; 2] = [2, 3];

    fn six_floats() -> Vec<u8> {
        [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect()
    }

    /// Builds (without completing) node `mtx` with module `m`: parameters `image` (the
    /// matrix) and `plain` (a double), and a command `cmd` with a double result.
    fn build() {
        let ctx = CONTEXT.as_ptr();
        // SAFETY: valid NUL-terminated strings throughout
        unsafe {
            assert_eq!(SECoP_S_initLibrary(std::ptr::null_mut(), 0, 0, ctx), 0);
            assert_eq!(
                SECoP_S_createNode2(
                    c"mtx".as_ptr(),
                    c"d".as_ptr(),
                    c"127.0.0.1".as_ptr(),
                    0,
                    ctx
                ),
                0
            );
            assert_eq!(SECoP_S_addModule(c"m".as_ptr(), ctx), 0);
            assert_eq!(
                SECoP_S_addReadableParameter(c"image".as_ptr(), None, ctx),
                0
            );
            assert_eq!(
                SECoP_S_addPropertyJSON(c"datainfo".as_ptr(), IMAGE.as_ptr(), ctx),
                0
            );
            assert_eq!(SECoP_S_setAddFocus(c"mtx:m".as_ptr()), 0);
            assert_eq!(
                SECoP_S_addReadableParameter(c"plain".as_ptr(), None, ctx),
                0
            );
            assert_eq!(
                SECoP_S_addPropertyJSON(
                    c"datainfo".as_ptr(),
                    cr#"{"type":"double"}"#.as_ptr(),
                    ctx
                ),
                0
            );
            assert_eq!(SECoP_S_setAddFocus(c"mtx:m".as_ptr()), 0);
            assert_eq!(SECoP_S_addCommand(c"cmd".as_ptr(), None, ctx), 0);
            assert_eq!(
                SECoP_S_addPropertyJSON(
                    c"datainfo".as_ptr(),
                    cr#"{"type":"command","result":{"type":"double"}}"#.as_ptr(),
                    ctx
                ),
                0
            );
        }
    }

    fn initial(parameter: &str) -> Option<Json> {
        let state = lock();
        let entry = &state.nodes[state.node_index("mtx").unwrap()];
        let module = &entry.draft.modules[entry.draft.module_index("m").unwrap()];
        let acc = &module.accessibles[module.accessible_index(parameter).unwrap()];
        match &acc.kind {
            AccessibleKind::Parameter { initial, .. } => initial.clone(),
            AccessibleKind::Command { .. } => None,
        }
    }

    /// Hands out a request for `accessible` as `SECoP_S_getStoredCommand` would.
    fn hand_out(accessible: &str) -> (u64, oneshot::Receiver<Answer>) {
        let (reply, answer) = oneshot::channel();
        let mut state = lock();
        state.next_id += 1;
        let id = state.next_id;
        state.handed_out.insert(
            id,
            Pending {
                node: "mtx".into(),
                module: "m".into(),
                accessible: accessible.into(),
                action: 1,
                value: None,
                reply,
            },
        );
        (id, answer)
    }

    fn is_handed_out(id: u64) -> bool {
        lock().handed_out.contains_key(&id)
    }

    /// `SECoP_S_updateParameterMatrix` with a Rust name and slices.
    fn update_matrix(name: &std::ffi::CStr, len: &[c_uint], data: Option<&[u8]>) -> c_int {
        let (ptr, size) = data.map_or((std::ptr::null(), 0), |d| {
            (
                d.as_ptr().cast::<c_void>(),
                c_int::try_from(d.len()).unwrap(),
            )
        });
        // SAFETY: the pointers are valid for the given counts
        unsafe {
            SECoP_S_updateParameterMatrix(
                name.as_ptr(),
                len.as_ptr(),
                c_int::try_from(len.len()).unwrap(),
                ptr,
                size,
                f64::NAN,
            )
        }
    }

    fn answer_matrix(id: u64, code: c_int, len: &[c_uint], data: &[u8]) -> c_int {
        // SAFETY: the pointers are valid for the given counts
        unsafe {
            SECoP_S_putCommandAnswerMatrix(
                id,
                code,
                len.as_ptr(),
                c_int::try_from(len.len()).unwrap(),
                data.as_ptr().cast::<c_void>(),
                c_int::try_from(data.len()).unwrap(),
                f64::NAN,
            )
        }
    }

    #[test]
    fn matrix_functions() {
        build();
        update_parameter_matrix();
        put_command_answer_matrix();
        // SAFETY: a valid string
        unsafe { SECoP_S_doneLibrary(1, CONTEXT.as_ptr()) };
    }

    fn update_parameter_matrix() {
        let data = six_floats();

        // not a matrix parameter
        assert_eq!(
            update_matrix(c"mtx:m:plain", &LEN, Some(&data)),
            err::INVALID_VALUE
        );
        assert_eq!(initial("plain"), None, "nothing stored");
        // size does not fit: bytes, then dimensions
        assert_eq!(
            update_matrix(c"mtx:m:image", &LEN, Some(&data[..20])),
            err::INVALID_VALUE
        );
        assert_eq!(
            update_matrix(c"mtx:m:image", &[6], Some(&data)),
            err::INVALID_VALUE
        );
        // negative sizes, or NULL with a size
        // SAFETY: NULL pointers with negative or zero counts are not read
        unsafe {
            assert_eq!(
                SECoP_S_updateParameterMatrix(
                    c"mtx:m:image".as_ptr(),
                    LEN.as_ptr(),
                    -2,
                    std::ptr::null(),
                    0,
                    f64::NAN
                ),
                err::INVALID_VALUE
            );
            assert_eq!(
                SECoP_S_updateParameterMatrix(
                    c"mtx:m:image".as_ptr(),
                    LEN.as_ptr(),
                    2,
                    std::ptr::null(),
                    24,
                    f64::NAN
                ),
                err::INVALID_VALUE
            );
        }
        assert_eq!(initial("image"), None, "nothing stored");
        // unknown names: the codes of updateParameter2
        for name in [
            c"",
            c"nonode:m:image",
            c"mtx:nomodule:image",
            c"mtx:m:nothing",
            c"mtx",
        ] {
            // SAFETY: valid strings
            let expected = unsafe {
                SECoP_S_updateParameter2(name.as_ptr(), c"1".as_ptr(), -1, std::ptr::null(), 0, 0.0)
            };
            assert!(expected < 0, "{name:?}");
            assert_eq!(update_matrix(name, &LEN, Some(&data)), expected, "{name:?}");
        }
        // an empty matrix without data
        assert_eq!(update_matrix(c"mtx:m:image", &[0, 0], None), err::SUCCESS);
        assert_eq!(initial("image"), Some(json!({"len": [0, 0], "blob": ""})));
        // the spec example
        assert_eq!(
            update_matrix(c"mtx:m:image", &LEN, Some(&data)),
            err::SUCCESS
        );
        assert_eq!(
            initial("image"),
            Some(json!({"len": [2, 3], "blob": "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA"}))
        );
    }

    fn put_command_answer_matrix() {
        let data = six_floats();

        // requests without a matrix value stay handed out and can still be answered
        let (plain, mut plain_answer) = hand_out("plain");
        let (cmd, _cmd_answer) = hand_out("cmd");
        for id in [plain, cmd] {
            assert_eq!(
                answer_matrix(id, err::SUCCESS, &LEN, &data),
                err::INVALID_VALUE
            );
            assert!(is_handed_out(id));
        }
        // SAFETY: valid strings
        let code = unsafe {
            SECoP_S_putCommandAnswer2(plain, 0, c"1.5".as_ptr(), -1, std::ptr::null(), 0, 0.0)
        };
        assert_eq!(code, err::SUCCESS);
        let answer = plain_answer.try_recv().unwrap();
        assert!(matches!(answer.value, Some(AnswerValue::Json(ref j)) if *j == json!(1.5)));

        // size does not fit: still handed out; then the right size answers it
        let (image, mut image_answer) = hand_out("image");
        assert_eq!(
            answer_matrix(image, err::SUCCESS, &LEN, &data[..20]),
            err::INVALID_VALUE
        );
        assert!(is_handed_out(image));
        assert_eq!(
            answer_matrix(image, err::SUCCESS, &LEN, &data),
            err::SUCCESS
        );
        assert!(!is_handed_out(image));
        let answer = image_answer.try_recv().unwrap();
        let Some(AnswerValue::Value(Value::Matrix { len, data: bytes })) = answer.value else {
            panic!("{answer:?}")
        };
        assert_eq!((len, bytes), (LEN.to_vec(), data.clone()));
        // answered before: unknown
        assert_eq!(
            answer_matrix(image, err::SUCCESS, &LEN, &data),
            err::INVALID_NAME
        );

        // an error code does not read the matrix arguments
        let (failed, mut failed_answer) = hand_out("image");
        // SAFETY: NULL matrix arguments with an error code
        let code = unsafe {
            SECoP_S_putCommandAnswerMatrix(
                failed,
                err::COMM_FAILED,
                std::ptr::null(),
                -1,
                std::ptr::null(),
                -1,
                f64::NAN,
            )
        };
        assert_eq!(code, err::SUCCESS);
        let answer = failed_answer.try_recv().unwrap();
        assert_eq!(answer.code, err::COMM_FAILED);
        assert!(answer.value.is_none());

        lock().handed_out.remove(&cmd);
    }
}
