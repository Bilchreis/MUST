// SPDX-License-Identifier: LGPL-3.0-or-later

//! Running nodes: updates, log messages, polling for requests and answering them.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_uint, c_ulonglong, c_void};

use must_server::{AppLogLevel, Reading};
use must_types::DataInfo;
use serde_json::Value as Json;

use super::{
    check_matrix_shape, code, context_id, handle_json, handle_value, initialized, matrix_arguments,
    sized_json, text,
};
use crate::ffi_util::{copy_out, guard, put};
use crate::server::build::AccessibleKind;
use crate::server::codes::err;
use crate::server::filter::Filter;
use crate::server::handler::{Answer, AnswerValue};
use crate::server::state::{Pending, State};
use crate::variant::{CSECoPbaseType, new_handle};

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

/// MUST addition: pushes a new matrix value of a parameter (`node:module:parameter`)
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

/// MUST addition, not in SHALL: logs `szMessage` for the module `szModule`
/// (`node:module`) of a running node at `SECoP_S_LOG_ERROR` (1), `SECoP_S_LOG_WARNING`
/// (2), `SECoP_S_LOG_INFO` (3) or `SECoP_S_LOG_DEBUG` (4). Clients that asked for the
/// module's log messages with SECoP's `logging` receive it as a `log` event; error,
/// warning and info records also reach the status display and the log files. Invalid
/// UTF-8 in the text is replaced. Safe to call from any thread, also from a callback.
///
/// # Safety
/// `szModule` and `szMessage` must be NULL or NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_logMessage(
    szModule: *const c_char,
    iLevel: c_int,
    szMessage: *const c_char,
) -> c_int {
    guard(err::INTERNAL, || {
        // SAFETY: forwarded caller contract
        let (name, message) = unsafe { (text(szModule), text(szMessage)) };
        code(log_message(&name.unwrap_or_default(), iLevel, message))
    })
}

fn log_message(name: &str, level: c_int, message: Option<String>) -> Result<c_int, c_int> {
    // the codes of `SECoP_S_updateParameter`: an empty name, then the first missing part
    if name.is_empty() {
        return Err(err::INVALID_NAME);
    }
    let (node, module) = name.split_once(':').ok_or(err::INVALID_NODE)?;
    let state = initialized()?;
    let entry = &state.nodes[state.node_index(node).ok_or(err::INVALID_NODE)?];
    let running = entry.running.as_ref().ok_or(err::INVALID_NODE)?;
    let m = entry
        .draft
        .module_index(module)
        .ok_or(err::INVALID_MODULE)?;
    let level = match level {
        1 => AppLogLevel::Error,
        2 => AppLogLevel::Warning,
        3 => AppLogLevel::Info,
        4 => AppLogLevel::Debug,
        _ => return Err(err::INVALID_VALUE),
    };
    let message = message.ok_or(err::INVALID_VALUE)?;
    running
        .log(&entry.draft.modules[m].name, level, message)
        .map(|()| err::SUCCESS)
        .map_err(|_| err::INVALID_MODULE)
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

/// MUST addition: hands out the oldest polled request of a context that the filter
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

/// MUST addition: hands out the oldest polled request of a context that the filter
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

/// MUST addition: answers a polled request with a matrix from plain arguments, see
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
