// SPDX-License-Identifier: LGPL-3.0-or-later

use serde_json::json;
use tokio::sync::oneshot;

use super::building::*;
use super::context::*;
use super::runtime::*;
use super::*;
use crate::server::build::AccessibleKind;
use crate::server::handler::Answer;
use crate::server::state::Pending;
use crate::server::state::lock;

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
            SECoP_S_addPropertyJSON(c"datainfo".as_ptr(), cr#"{"type":"double"}"#.as_ptr(), ctx),
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

/// The context of the `SECoP_S_logMessage` test.
const LOG_CONTEXT: &core::ffi::CStr = c"api-tests-log";

fn log(name: &core::ffi::CStr, level: c_int, message: Option<&core::ffi::CStr>) -> c_int {
    let message = message.map_or(std::ptr::null(), core::ffi::CStr::as_ptr);
    // SAFETY: NUL-terminated strings or NULL
    unsafe { SECoP_S_logMessage(name.as_ptr(), level, message) }
}

#[test]
fn log_message_errors() {
    let ctx = LOG_CONTEXT.as_ptr();
    // SAFETY: NUL-terminated strings; a node in polling mode on a free port
    unsafe {
        assert_eq!(
            SECoP_S_initLibrary(std::ptr::null_mut(), 0, 0, ctx),
            err::SUCCESS
        );
        let iface = c"127.0.0.1".as_ptr();
        let rc = SECoP_S_createNode2(c"lognode".as_ptr(), c"logs".as_ptr(), iface, 0, ctx);
        assert_eq!(rc, err::SUCCESS);
        assert_eq!(SECoP_S_addModule(c"mod1".as_ptr(), ctx), err::SUCCESS);
        let description = c"description".as_ptr();
        let rc = SECoP_S_addPropertyString(description, c"a module".as_ptr(), ctx);
        assert_eq!(rc, err::SUCCESS);
    }
    let message = Some(c"x");

    // not complete yet
    assert_eq!(log(c"lognode:mod1", 3, message), err::INVALID_NODE);
    // SAFETY: a NUL-terminated string
    assert!(unsafe { SECoP_S_nodeComplete(ctx) } >= 0);

    assert_eq!(log(c"lognode:mod1", 3, message), err::SUCCESS);
    assert_eq!(
        log(c"LOGNODE:MOD1", 4, message),
        err::SUCCESS,
        "case is ignored"
    );
    assert_eq!(log(c"lognode:nope", 3, message), err::INVALID_MODULE);
    assert_eq!(log(c"lognode:mod1:value", 3, message), err::INVALID_MODULE);
    assert_eq!(log(c"other:mod1", 3, message), err::INVALID_NODE);
    assert_eq!(log(c"lognode", 3, message), err::INVALID_NODE);
    assert_eq!(log(c"", 3, message), err::INVALID_NAME);
    for level in [0, 5, -1] {
        assert_eq!(log(c"lognode:mod1", level, message), err::INVALID_VALUE);
    }
    assert_eq!(log(c"lognode:mod1", 1, None), err::INVALID_VALUE);
    assert_eq!(
        log(c"lognode:mod1", 2, Some(c"\xff not UTF-8")),
        err::SUCCESS,
        "invalid bytes are replaced"
    );

    // SAFETY: a NUL-terminated string
    unsafe { SECoP_S_doneLibrary(0, ctx) };
    // the node is gone (or, without other contexts, the library is uninitialized)
    assert!(log(c"lognode:mod1", 3, message) < 0);
}
