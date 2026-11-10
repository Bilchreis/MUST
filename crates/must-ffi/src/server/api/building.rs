// SPDX-License-Identifier: LGPL-3.0-or-later

//! Building nodes: creating them, adding modules and accessibles, completing them.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_longlong, c_ushort};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use must_server::monitor::ExternalNode;
use serde_json::Value as Json;

use super::context::new_context;
use super::{code, context_id, handle_json, initialized, sized_json, text};
use crate::ffi_util::guard;
use crate::server::build::{
    AccessibleKind, CallFunction, CheckFunction, DraftNode, GetSet, valid_name,
};
use crate::server::codes::{description, err};
use crate::server::state::{self, NodeEntry};
use crate::variant::CSECoPbaseType;

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

/// A MUST addition, not in SHALL: sets the check function of the focused parameter or
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
