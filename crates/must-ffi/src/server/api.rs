// SPDX-License-Identifier: LGPL-3.0-or-later

//! SHALL's `SECoP_S_*` C functions (`SECoP-exports.h`).
//!
//! Context IDs are strings; NULL is the same context as `""`. Functions without a
//! context argument find their node by name (`SECoP_S_deleteNode`, `SECoP_S_setAddFocus`,
//! `SECoP_S_updateParameter*`) or their request by id (`SECoP_S_putCommandAnswer*`).

#![allow(non_snake_case)]

use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_longlong, c_ulonglong, c_ushort, c_void};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use must_server::Reading;
use must_server::monitor::ExternalNode;
use must_types::Mode;
use serde_json::Value as Json;

use super::build::{AccessibleKind, CallFunction, DraftNode, GetSet, valid_name};
use super::codes::{description, err};
use super::handler::Answer;
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
///
/// # Safety
/// See [`sized_bytes`].
unsafe fn sized_json(s: *const c_char, size: c_int) -> Result<Option<Json>, c_int> {
    // SAFETY: forwarded caller contract
    match unsafe { sized_bytes(s, size) } {
        None => Ok(None),
        Some(b) if b.iter().all(u8::is_ascii_whitespace) => Ok(None),
        Some(b) => serde_json::from_slice(b)
            .map(Some)
            .map_err(|_| err::INVALID_VALUE),
    }
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
/// dropped). Unless `bNodeOnly`, the library's threads end when no context remains (and
/// the status TUI is no longer opened again when closed).
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
        state::stop(std::mem::take(&mut running), handle);
        if let Some(runtime) = runtime {
            state::shut_down(runtime);
            // no node left to show: stop reopening the TUI
            status::show(false);
        }
    });
}

/// Accepted for compatibility; must always serves requests on several threads.
#[unsafe(no_mangle)]
pub extern "C" fn SECoP_S_setManyThreads(_bManyThreads: c_int) {}

/// Converts a `SECoP_S_error` code to a static description.
#[unsafe(no_mangle)]
pub extern "C" fn SECoP_S_getErrorString(iError: c_int) -> *const c_char {
    description(iError).as_ptr()
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
        state.warnings.extend(warnings);
        Ok(err::WARNING_MISS_PROPERTIES)
    }
}

/// Prints (to stderr) and clears the errors and warnings recorded while building nodes.
#[unsafe(no_mangle)]
pub extern "C" fn SECoP_S_showErrors() {
    guard((), || {
        let (errors, warnings) = {
            let mut state = state::lock();
            (
                std::mem::take(&mut state.errors),
                std::mem::take(&mut state.warnings),
            )
        };
        for e in errors {
            eprintln!("SECoP error: {e}");
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
            let value = handle_json(pData)?.ok_or(err::INVALID_VALUE)?;
            update(&name, value, handle_json(pSigma)?, dblTimestamp)
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
            update(&name, value, sigma, dblTimestamp)
        })())
    })
}

fn update(name: &str, value: Json, sigma: Option<Json>, t: f64) -> Result<c_int, c_int> {
    let mut parts = name.splitn(3, ':');
    let (Some(node), Some(module), Some(parameter)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(err::INVALID_PARAMETER);
    };
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
    let AccessibleKind::Parameter { initial, .. } = &mut acc.kind else {
        return Err(err::INVALID_PARAMETER);
    };
    let Some(running) = &entry.running else {
        // checked against the datainfo when the node is complete
        *initial = Some(value);
        return Ok(err::SUCCESS);
    };
    let info = acc
        .datainfo
        .as_ref()
        .and_then(crate::variant::Node::to_datainfo)
        .ok_or(err::INVALID_VALUE)?;
    let value = info
        .decode(&value, Mode::Report)
        .map_err(|_| err::INVALID_VALUE)?;
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

/// Takes the next polled request of a context; from now on it waits for its answer.
fn next_request(context: &str) -> Result<Request, c_int> {
    let mut state = initialized()?;
    let ctx = state
        .contexts
        .get_mut(context)
        .ok_or(err::INVALID_CONTEXT_ID)?;
    let (id, pending) = ctx.queue.pop_front().ok_or(err::NO_DATA)?;
    let request = Request {
        id,
        action: pending.action,
        name: format!("{}:{}:{}", pending.node, pending.module, pending.accessible),
        value: pending.value.clone(),
    };
    state.handed_out.insert(id, pending);
    Ok(request)
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
        let request = match next_request(&context) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let value = request.value.map_or(std::ptr::null_mut(), new_handle);
        // SAFETY: forwarded caller contract
        let fits = unsafe {
            put(pllId, request.id);
            put(piAction, request.action);
            put(ppValue, value);
            write_text(request.name.as_bytes(), szParameter, piParameterSize)
        };
        if fits {
            err::SUCCESS
        } else {
            err::WARNING_BUFFER_TOO_SMALL
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
        let request = match next_request(&context) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let text = request
            .value
            .as_ref()
            .map(|v| serde_json::to_vec(&v.export_value()).unwrap_or_default())
            .unwrap_or_default();
        // SAFETY: forwarded caller contract
        let fits = unsafe {
            put(pllId, request.id);
            put(piAction, request.action);
            let name = write_text(request.name.as_bytes(), szParameter, piParameterSize);
            let value = write_text(&text, szValue, piValueSize);
            name && value
        };
        if fits {
            err::SUCCESS
        } else {
            err::WARNING_BUFFER_TOO_SMALL
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
            let value = handle_json(pValue)?;
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
            answer(llId, iErrorCode, value, sigma, dblTimestamp)
        })())
    })
}

fn answer(
    id: u64,
    code: c_int,
    value: Option<Json>,
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
