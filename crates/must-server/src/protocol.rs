// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP message framing, parsing and formatting. No I/O happens here.

use std::fmt::Display;
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

/// Capacity the line buffer may keep while it holds little; more is given back.
const KEPT_CAPACITY: usize = 16 * 1024;

/// Splits incoming bytes into lines of at most `max` bytes.
///
/// Lines are taken from the buffer by advancing an offset, so a read holding many
/// lines is split in time linear in its size; the buffer is compacted once per
/// [`feed`](Self::feed).
pub(crate) struct LineSplitter {
    buf: Vec<u8>,
    /// Start of the bytes not yet returned.
    start: usize,
    /// Where the search for the next LF resumes; `buf[start..scan]` holds none.
    scan: usize,
    max: usize,
    /// Dropping the rest of an over-long line, which has already been reported.
    discarding: bool,
}

impl LineSplitter {
    pub fn new(max: usize) -> Self {
        Self {
            buf: Vec::new(),
            start: 0,
            scan: 0,
            max,
            discarding: false,
        }
    }

    /// Appends received bytes.
    pub fn feed(&mut self, data: &[u8]) {
        if self.start > 0 {
            self.buf.drain(..self.start);
            self.scan -= self.start;
            self.start = 0;
        }
        // keep the capacity of an over-long line only while it is needed
        if self.buf.capacity() > KEPT_CAPACITY && self.buf.len() + data.len() <= KEPT_CAPACITY / 2 {
            self.buf.shrink_to(KEPT_CAPACITY);
        }
        self.buf.extend_from_slice(data);
    }

    /// Returns the next complete frame, if any.
    pub fn next_frame(&mut self) -> Option<Frame> {
        loop {
            match self.buf[self.scan..].iter().position(|b| *b == b'\n') {
                Some(offset) => {
                    let end = self.scan + offset;
                    let mut line = &self.buf[self.start..end];
                    self.start = end + 1;
                    self.scan = self.start;
                    if self.discarding {
                        self.discarding = false;
                        continue;
                    }
                    if let [rest @ .., b'\r'] = line {
                        line = rest;
                    }
                    if line.len() > self.max {
                        return Some(Frame::TooLong);
                    }
                    return Some(Frame::Line(String::from_utf8_lossy(line).into_owned()));
                }
                None if self.buf.len() - self.start > self.max + 1 => {
                    // no LF within the limit: report once, then drop until the next LF
                    self.buf.clear();
                    self.start = 0;
                    self.scan = 0;
                    if self.discarding {
                        return None;
                    }
                    self.discarding = true;
                    return Some(Frame::TooLong);
                }
                None => {
                    self.scan = self.buf.len();
                    return None;
                }
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
    /// `check <module>:<accessible> [<value>]`
    Check(Option<&'a str>, Option<&'a str>),
    /// `logging [<module>[:<parameter>]] <level>`: specifier (`None` for the whole node)
    /// and level.
    Logging(Option<&'a str>, Option<&'a str>),
    /// An action must does not know.
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
        "check" => Request::Check(specifier, data),
        // `logging "info"` with one space: no module name starts with `"`
        "logging" => match specifier {
            Some(s) if s.starts_with('"') => {
                Request::Logging(None, Some(&line["logging ".len()..]))
            }
            _ => Request::Logging(specifier, data),
        },
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

// --- formatting; every function returns a complete line without its LF, which the
// connection's writer adds

/// `ISSE,SECoP,<draft-date>,v2.0`
pub(crate) fn identification() -> String {
    format!("ISSE,SECoP,{SECOP_DRAFT_DATE},{SECOP_VERSION}")
}

/// `describing . <structure report>`
pub(crate) fn describing(report: &str) -> String {
    format!("describing . {report}")
}

/// `active [<module>]` or `inactive [<module>]`
pub(crate) fn activation(active: bool, module: Option<&str>) -> String {
    let word = if active { "active" } else { "inactive" };
    match module {
        Some(m) => format!("{word} {m}"),
        None => word.to_owned(),
    }
}

/// `pong <token> [null, {"t": ...}]`, with two spaces when there is no token.
pub(crate) fn pong(token: Option<&str>, t: f64) -> String {
    let report = must_types::data_report(Json::Null, &must_types::Qualifiers::at(t));
    format!("pong {} {report}", token.unwrap_or_default())
}

/// `<action> <specifier> <data report>` for `reply`, `changed`, `done` and `update`.
pub(crate) fn data_message(action: &str, specifier: &str, data_report: &impl Display) -> String {
    format!("{action} {specifier} {data_report}")
}

/// `logging <module> <level>`, or `logging  <level>` for the whole node.
pub(crate) fn logging(module: Option<&str>, level: &Json) -> String {
    format!("logging {} {level}", module.unwrap_or_default())
}

/// `log <module>:<level> <message>`, with the message as a JSON string.
pub(crate) fn log_event(module: &str, level: &str, message: &str) -> String {
    let message = serde_json::to_string(message).unwrap_or_default();
    format!("log {module}:{level} {message}")
}

/// `error_<action> <specifier> <error report>`, with two spaces when there is no
/// specifier.
pub(crate) fn error_message(action: &str, specifier: Option<&str>, error: &SecopError) -> String {
    let report = error_report(error, Some(now()));
    format!("error_{action} {} {report}", specifier.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn many_lines_and_a_partial_one_in_one_read() {
        let mut input = b"ping 1\n".repeat(1000);
        input.extend_from_slice(b"read t1:");
        let f = frames(100, &[&input, b"val", b"ue\r\nping\n"]);
        assert_eq!(f.len(), 1002);
        assert_eq!(f[999], Frame::Line("ping 1".into()));
        assert_eq!(f[1000], Frame::Line("read t1:value".into()));
        assert_eq!(f[1001], Frame::Line("ping".into()));
        // an over-long rest after complete lines
        assert_eq!(
            frames(5, &[b"ok\nxxxxxxxxxx", b"xx\nok\n"]),
            [
                Frame::Line("ok".into()),
                Frame::TooLong,
                Frame::Line("ok".into())
            ]
        );
    }

    #[test]
    fn capacity_of_an_over_long_line_is_given_back() {
        let mut s = LineSplitter::new(1 << 20);
        let mut long = vec![b'x'; 1 << 19];
        long.push(b'\n');
        s.feed(&long);
        assert!(matches!(s.next_frame(), Some(Frame::Line(_))));
        assert!(s.buf.capacity() >= 1 << 19);
        s.feed(b"ping\n");
        assert!(s.buf.capacity() <= KEPT_CAPACITY);
        assert_eq!(s.next_frame(), Some(Frame::Line("ping".into())));
    }
}
