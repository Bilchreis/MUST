// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP message framing, parsing and formatting. No I/O happens here.

use std::time::{SystemTime, UNIX_EPOCH};

use must_types::{SECOP_DRAFT_DATE, SECOP_VERSION, SecopError, error_report};
use serde_json::Value as Json;

/// One step of splitting a byte stream into lines.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    /// A complete line, without its LF and an optional preceding CR.
    Line(String),
    /// A line longer than the limit; its bytes were dropped up to and including its LF.
    TooLong,
}

/// Splits incoming bytes into lines of at most `max` bytes.
pub(crate) struct LineSplitter {
    buf: Vec<u8>,
    max: usize,
    /// Dropping the rest of an over-long line, which has already been reported.
    discarding: bool,
}

impl LineSplitter {
    pub fn new(max: usize) -> Self {
        Self {
            buf: Vec::new(),
            max,
            discarding: false,
        }
    }

    /// Appends received bytes.
    pub fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Returns the next complete frame, if any.
    pub fn next_frame(&mut self) -> Option<Frame> {
        loop {
            match self.buf.iter().position(|b| *b == b'\n') {
                Some(pos) => {
                    let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
                    if self.discarding {
                        self.discarding = false;
                        continue;
                    }
                    line.pop(); // LF
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if line.len() > self.max {
                        return Some(Frame::TooLong);
                    }
                    return Some(Frame::Line(String::from_utf8_lossy(&line).into_owned()));
                }
                None if self.buf.len() > self.max + 1 => {
                    // no LF within the limit: report once, then drop until the next LF
                    self.buf.clear();
                    if self.discarding {
                        return None;
                    }
                    self.discarding = true;
                    return Some(Frame::TooLong);
                }
                None => return None,
            }
        }
    }
}

/// A parsed request line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Request<'a> {
    /// An empty line, ignored.
    Empty,
    /// `*IDN?`
    Identify,
    /// `describe [<anything>]`
    Describe,
    /// `activate [<module>]`
    Activate(Option<&'a str>),
    /// `deactivate [<module>]`
    Deactivate(Option<&'a str>),
    /// `ping [<token>]`
    Ping(Option<&'a str>),
    /// `read <module>:<parameter>`
    Read(Option<&'a str>),
    /// `change <module>:<parameter> <value>`
    Change(Option<&'a str>, Option<&'a str>),
    /// `do <module>:<command> [<argument>]`
    Do(Option<&'a str>, Option<&'a str>),
    /// An action must does not implement (`check`, `logging`) or does not know.
    Unsupported {
        action: &'a str,
        specifier: Option<&'a str>,
    },
}

/// Splits a line at its first two spaces into action, specifier and data.
pub(crate) fn parse_request(line: &str) -> Request<'_> {
    if line.is_empty() {
        return Request::Empty;
    }
    let mut parts = line.splitn(3, ' ');
    let action = parts.next().unwrap_or_default();
    let specifier = parts.next().filter(|s| !s.is_empty());
    let data = parts.next().filter(|d| !d.trim().is_empty());
    match action {
        "*IDN?" => Request::Identify,
        "describe" => Request::Describe,
        "activate" => Request::Activate(specifier),
        "deactivate" => Request::Deactivate(specifier),
        "ping" => Request::Ping(specifier),
        "read" => Request::Read(specifier),
        "change" => Request::Change(specifier, data),
        "do" => Request::Do(specifier, data),
        _ => Request::Unsupported { action, specifier },
    }
}

/// Splits `<module>:<accessible>`.
pub(crate) fn split_specifier(specifier: &str) -> Option<(&str, &str)> {
    specifier
        .split_once(':')
        .filter(|(m, a)| !m.is_empty() && !a.is_empty())
}

/// Current time as UNIX seconds.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

// --- formatting; every function returns a complete line including its LF

/// `ISSE,SECoP,<draft-date>,v2.0`
pub(crate) fn identification() -> String {
    format!("ISSE,SECoP,{SECOP_DRAFT_DATE},{SECOP_VERSION}\n")
}

/// `describing . <structure report>`
pub(crate) fn describing(report: &str) -> String {
    format!("describing . {report}\n")
}

/// `active [<module>]` or `inactive [<module>]`
pub(crate) fn activation(active: bool, module: Option<&str>) -> String {
    let word = if active { "active" } else { "inactive" };
    match module {
        Some(m) => format!("{word} {m}\n"),
        None => format!("{word}\n"),
    }
}

/// `pong <token> [null, {"t": ...}]`, with two spaces when there is no token.
pub(crate) fn pong(token: Option<&str>, t: f64) -> String {
    let report = must_types::data_report(Json::Null, &must_types::Qualifiers::at(t));
    format!("pong {} {report}\n", token.unwrap_or_default())
}

/// `<action> <specifier> <data report>` for `reply`, `changed`, `done` and `update`.
pub(crate) fn data_message(action: &str, specifier: &str, data_report: &Json) -> String {
    format!("{action} {specifier} {data_report}\n")
}

/// `error_<action> <specifier> <error report>`, with two spaces when there is no
/// specifier.
pub(crate) fn error_message(action: &str, specifier: Option<&str>, error: &SecopError) -> String {
    let report = error_report(error, Some(now()));
    format!(
        "error_{action} {} {report}\n",
        specifier.unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use must_types::ErrorClass;
    use serde_json::json;

    fn frames(max: usize, input: &[&[u8]]) -> Vec<Frame> {
        let mut s = LineSplitter::new(max);
        let mut out = Vec::new();
        for chunk in input {
            s.feed(chunk);
            while let Some(f) = s.next_frame() {
                out.push(f);
            }
        }
        out
    }

    // --- 3.1 framing and parsing

    #[test]
    fn crlf_and_lf() {
        assert_eq!(
            frames(100, &[b"read t1:value\r\nping 1\n"]),
            [
                Frame::Line("read t1:value".into()),
                Frame::Line("ping 1".into())
            ]
        );
    }

    #[test]
    fn lines_split_across_reads() {
        assert_eq!(
            frames(100, &[b"rea", b"d t1:va", b"lue\n"]),
            [Frame::Line("read t1:value".into())]
        );
    }

    #[test]
    fn too_long_line_reported_once_and_skipped() {
        let long = vec![b'x'; 50];
        let f = frames(10, &[&long, &long, b"\nping\n"]);
        assert_eq!(f, [Frame::TooLong, Frame::Line("ping".into())]);
        // a complete long line in one read is reported too
        assert_eq!(
            frames(3, &[b"abcdef\nok\n"]),
            [Frame::TooLong, Frame::Line("ok".into())]
        );
        // exactly at the limit is fine, also with CR
        assert_eq!(frames(3, &[b"abc\r\n"]), [Frame::Line("abc".into())]);
    }

    #[test]
    fn data_containing_spaces() {
        assert_eq!(
            parse_request(r#"change t1:target {"a": 1, "b": 2}"#),
            Request::Change(Some("t1:target"), Some(r#"{"a": 1, "b": 2}"#))
        );
    }

    #[test]
    fn requests() {
        assert_eq!(parse_request(""), Request::Empty);
        assert_eq!(parse_request("*IDN?"), Request::Identify);
        assert_eq!(parse_request("describe"), Request::Describe);
        assert_eq!(parse_request("describe ."), Request::Describe);
        assert_eq!(parse_request("activate"), Request::Activate(None));
        assert_eq!(parse_request("activate t1"), Request::Activate(Some("t1")));
        assert_eq!(parse_request("deactivate"), Request::Deactivate(None));
        assert_eq!(parse_request("ping"), Request::Ping(None));
        assert_eq!(parse_request("ping 42"), Request::Ping(Some("42")));
        assert_eq!(
            parse_request("read t1:value"),
            Request::Read(Some("t1:value"))
        );
        assert_eq!(
            parse_request("do t1:stop"),
            Request::Do(Some("t1:stop"), None)
        );
        assert_eq!(
            parse_request("do t1:stop null"),
            Request::Do(Some("t1:stop"), Some("null"))
        );
        assert_eq!(
            parse_request("change t1:target"),
            Request::Change(Some("t1:target"), None)
        );
        assert_eq!(
            parse_request("meas:volt?"),
            Request::Unsupported {
                action: "meas:volt?",
                specifier: None
            }
        );
        assert_eq!(
            parse_request("check t1:target 5"),
            Request::Unsupported {
                action: "check",
                specifier: Some("t1:target")
            }
        );
        assert_eq!(
            parse_request(r#"logging  "debug""#),
            Request::Unsupported {
                action: "logging",
                specifier: None
            }
        );
    }

    #[test]
    fn specifiers() {
        assert_eq!(split_specifier("t1:value"), Some(("t1", "value")));
        assert_eq!(split_specifier("t1"), None);
        assert_eq!(split_specifier(":value"), None);
        assert_eq!(split_specifier("t1:"), None);
    }

    // --- 3.2 formatting

    #[test]
    fn formatted_messages() {
        assert_eq!(identification(), "ISSE,SECoP,2026-07-07,v2.0\n");
        assert_eq!(
            describing(r#"{"modules":{}}"#),
            "describing . {\"modules\":{}}\n"
        );
        assert_eq!(activation(true, None), "active\n");
        assert_eq!(activation(true, Some("t1")), "active t1\n");
        assert_eq!(activation(false, None), "inactive\n");
        assert_eq!(activation(false, Some("t1")), "inactive t1\n");
        assert_eq!(pong(Some("42"), 1.5), "pong 42 [null,{\"t\":1.5}]\n");
        assert_eq!(pong(None, 1.5), "pong  [null,{\"t\":1.5}]\n");
        for action in ["reply", "changed", "done", "update"] {
            assert_eq!(
                data_message(action, "t1:value", &json!([295.13, {"t": 1.0}])),
                format!("{action} t1:value [295.13,{{\"t\":1.0}}]\n")
            );
        }
    }

    #[test]
    fn error_messages() {
        let e = SecopError::new(ErrorClass::NoSuchModule, "tx is not configured");
        let line = error_message("read", Some("tx:target"), &e);
        assert!(
            line.starts_with(
                "error_read tx:target [\"NoSuchModule\",\"tx is not configured\",{\"t\":"
            ),
            "{line}"
        );
        assert!(line.ends_with("}]\n"));

        let e = SecopError::new(ErrorClass::ProtocolError, "unknown action");
        let line = error_message("meas:volt?", None, &e);
        assert!(
            line.starts_with("error_meas:volt?  [\"ProtocolError\""),
            "{line}"
        );
    }
}
