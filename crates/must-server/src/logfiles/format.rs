// SPDX-License-Identifier: LGPL-3.0-or-later

//! Node folders, file names and the text of the lines.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Write as _};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};

use crate::events::{ConnectionId, NodeEvent};

/// The folder of a node: its `equipment_id` with every character other than ASCII
/// letters, digits, `-`, `_` and `.` replaced by `_` (and never `.`, `..` or empty).
pub(crate) fn node_folder(equipment_id: &str) -> String {
    let folder: String = equipment_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if folder.chars().all(|c| c == '.') {
        "_".repeat(folder.len().max(1))
    } else {
        folder
    }
}

/// `YYYY-MM-DD.log`, or `YYYY-MM-DD-<pid>.log` when another process writes the first.
pub(crate) fn day_file(date: NaiveDate, pid: Option<u32>) -> String {
    match pid {
        None => format!("{}.log", date.format("%Y-%m-%d")),
        Some(pid) => format!("{}-{pid}.log", date.format("%Y-%m-%d")),
    }
}

/// The date of a log file name made by [`day_file`]; `None` for any other name.
pub(crate) fn parse_day_file(name: &str) -> Option<NaiveDate> {
    let stem = name.strip_suffix(".log")?;
    let (date, pid) = (stem.get(..10)?, stem.get(10..)?);
    if !(pid.is_empty()
        || pid
            .strip_prefix('-')
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
    {
        return None;
    }
    if !date.bytes().enumerate().all(|(i, b)| {
        if i == 4 || i == 7 {
            b == b'-'
        } else {
            b.is_ascii_digit()
        }
    }) {
        return None;
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

/// Local time with milliseconds and UTC offset, e.g. `2026-10-27T16:42:10.123+01:00`.
pub(crate) fn timestamp<Tz: TimeZone>(time: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    time.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()
}

/// The time of a node's lines: the text of [`timestamp`], made once per second, because
/// formatting a local time for every line is a large part of what logging costs.
#[derive(Default)]
pub(crate) struct LineClock {
    second: Option<i64>,
    date: NaiveDate,
    text: String,
}

impl LineClock {
    /// The current [`timestamp`] and local date.
    pub(crate) fn now(&mut self) -> (&str, NaiveDate) {
        self.at(Utc::now(), &Local)
    }

    fn at<Tz: TimeZone>(&mut self, time: DateTime<Utc>, zone: &Tz) -> (&str, NaiveDate)
    where
        Tz::Offset: Display,
    {
        let second = time.timestamp();
        if self.second != Some(second) {
            let local = time.with_timezone(zone);
            self.text = timestamp(&local);
            self.date = local.date_naive();
            self.second = Some(second);
        }
        // the digits before the offset, which is always `+hh:mm`
        let start = self.text.len() - "123+01:00".len();
        let millis = time.timestamp_subsec_millis().min(999);
        #[allow(clippy::cast_possible_truncation, reason = "digits")]
        let digits = [
            b'0' + (millis / 100) as u8,
            b'0' + (millis / 10 % 10) as u8,
            b'0' + (millis % 10) as u8,
        ];
        self.text.replace_range(
            start..start + digits.len(),
            std::str::from_utf8(&digits).unwrap_or("000"),
        );
        (&self.text, self.date)
    }
}

/// The line stating that `n` lines were dropped (without time).
pub(crate) fn dropped(n: u64) -> String {
    format!("* dropped {n} lines")
}

/// Turns node events into line texts (without time); remembers how to name the clients.
#[derive(Default)]
pub(crate) struct LineFormatter {
    clients: HashMap<ConnectionId, String>,
}

impl LineFormatter {
    /// The text of the line for `event`, or `None` if it is not logged.
    pub(crate) fn format(&mut self, event: &NodeEvent) -> Option<String> {
        match event {
            NodeEvent::Started { addr } => Some(format!(
                "* node started on {addr} (MUST {})",
                env!("CARGO_PKG_VERSION")
            )),
            NodeEvent::Stopped => Some("* node stopped".to_owned()),
            NodeEvent::ClientConnected { id, peer } => {
                self.clients.insert(*id, format!("[{id} {peer}]"));
                Some(format!("* {} connected", self.client(*id)))
            }
            NodeEvent::ClientDisconnected { id, reason } => {
                let line = format!("* {} disconnected: {reason}", self.client(*id));
                self.clients.remove(id);
                Some(line)
            }
            NodeEvent::MessageReceived { id, line } => {
                Some(format!("> {} {line}", self.client(*id)))
            }
            NodeEvent::MessageSent { id, line } => {
                // updates are logged once, when published (`Updated`)
                let action = line.split(' ').next().unwrap_or_default();
                if matches!(action, "update" | "error_update") {
                    None
                } else {
                    Some(format!("< {} {line}", self.client(*id)))
                }
            }
            NodeEvent::Updated { line, .. } => Some(format!("U {line}")),
            NodeEvent::Log {
                level,
                module,
                message,
            } => {
                let mut text = level.as_str().to_owned();
                text.push(' ');
                if let Some(module) = module {
                    let _ = write!(text, "{module}: ");
                }
                text.push_str(message);
                Some(text)
            }
            // `activate`/`deactivate` and their replies are logged as traffic; the file
            // itself is not news in the file
            NodeEvent::ClientActivated { .. }
            | NodeEvent::ClientDeactivated { .. }
            | NodeEvent::LogFile { .. } => None,
        }
    }

    fn client(&self, id: ConnectionId) -> Cow<'_, str> {
        match self.clients.get(&id) {
            Some(client) => client.into(),
            None => format!("[{id}]").into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use chrono::FixedOffset;

    use super::*;
    use crate::events::LogLevel;

    #[test]
    fn unusual_equipment_id() {
        assert_eq!(node_folder("lab:cryo/1"), "lab_cryo_1");
        assert_eq!(node_folder("HZB-cryo_2.a"), "HZB-cryo_2.a");
        assert_eq!(node_folder(r"a\b c*?"), "a_b_c__");
        assert_eq!(node_folder("Kälte"), "K_lte");
        assert_eq!(node_folder(".."), "__");
        assert_eq!(node_folder(""), "_");
    }

    #[test]
    fn day_files() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        assert_eq!(day_file(date, None), "2026-10-27.log");
        assert_eq!(day_file(date, Some(4242)), "2026-10-27-4242.log");
        assert_eq!(parse_day_file("2026-10-27.log"), Some(date));
        assert_eq!(parse_day_file("2026-10-27-4242.log"), Some(date));
        for other in [
            "notes.txt",
            "2026-10-27.txt",
            "2026-10-27-.log",
            "2026-10-27-x1.log",
            "2026-13-27.log",
            "2026-1-027.log",
            "x2026-10-27.log",
            "2026-10-27.log.bak",
        ] {
            assert_eq!(parse_day_file(other), None, "{other}");
        }
    }

    fn at(h: u32, m: u32, s: u32, ms: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 27, h, m, s)
            .unwrap()
            + chrono::Duration::milliseconds(i64::from(ms))
    }

    #[test]
    fn clock_matches_timestamp() {
        let mut clock = LineClock::default();
        // within a second, across seconds and days, with the clock going back
        for time in [
            at(16, 42, 10, 0),
            at(16, 42, 10, 7),
            at(16, 42, 10, 45),
            at(16, 42, 10, 999),
            at(16, 42, 11, 123),
            at(23, 59, 59, 999),
            at(23, 59, 59, 999) + chrono::Duration::milliseconds(1),
            at(16, 42, 10, 500),
        ] {
            let (text, date) = clock.at(time.to_utc(), time.offset());
            assert_eq!(text, timestamp(&time));
            assert_eq!(date, time.date_naive());
        }
        // the offset changes with the second it applies to
        let summer = FixedOffset::east_opt(7200).unwrap();
        let time = at(16, 42, 12, 5).to_utc();
        assert_eq!(
            clock.at(time, &summer).0,
            timestamp(&time.with_timezone(&summer))
        );
    }

    #[test]
    fn readable_lines() {
        let mut f = LineFormatter::default();
        let peer: SocketAddr = "192.168.1.20:51234".parse().unwrap();
        let line = |f: &mut LineFormatter, e: NodeEvent| {
            f.format(&e)
                .map(|text| format!("{} {text}", timestamp(&at(16, 42, 10, 123))))
        };
        assert_eq!(
            line(&mut f, NodeEvent::ClientConnected { id: 3, peer }).unwrap(),
            "2026-10-27T16:42:10.123+01:00 * [3 192.168.1.20:51234] connected"
        );
        assert_eq!(
            line(
                &mut f,
                NodeEvent::MessageReceived {
                    id: 3,
                    line: Arc::from("change t1:target 300")
                }
            )
            .unwrap(),
            "2026-10-27T16:42:10.123+01:00 > [3 192.168.1.20:51234] change t1:target 300"
        );
        assert_eq!(
            line(
                &mut f,
                NodeEvent::MessageSent {
                    id: 3,
                    line: Arc::from("changed t1:target [300,{\"t\":1.5}]")
                }
            )
            .unwrap(),
            "2026-10-27T16:42:10.123+01:00 < [3 192.168.1.20:51234] changed t1:target [300,{\"t\":1.5}]"
        );
        // copies of updates sent to clients are not logged
        for sent in ["update t1:value [1,{}]", "error_update t1:value [\"x\"]"] {
            let e = NodeEvent::MessageSent {
                id: 3,
                line: Arc::from(sent),
            };
            assert_eq!(f.format(&e), None);
        }
        assert_eq!(
            f.format(&NodeEvent::ClientDisconnected {
                id: 3,
                reason: "closed by client".into()
            })
            .unwrap(),
            "* [3 192.168.1.20:51234] disconnected: closed by client"
        );
        // an unknown client (connected while lines were dropped)
        let e = NodeEvent::MessageReceived {
            id: 9,
            line: Arc::from("ping"),
        };
        assert_eq!(f.format(&e).unwrap(), "> [9] ping");

        let e = NodeEvent::Log {
            level: LogLevel::Warn,
            module: Some("t1".into()),
            message: "reading value failed".into(),
        };
        assert_eq!(f.format(&e).unwrap(), "WARN t1: reading value failed");
        let e = NodeEvent::Log {
            level: LogLevel::Info,
            module: None,
            message: "hello".into(),
        };
        assert_eq!(f.format(&e).unwrap(), "INFO hello");
        assert_eq!(f.format(&NodeEvent::Stopped).unwrap(), "* node stopped");
        let e = NodeEvent::ClientActivated {
            id: 3,
            module: None,
        };
        assert_eq!(f.format(&e), None);
        // a gap after `Lagged`
        assert_eq!(dropped(17), "* dropped 17 lines");
    }

    #[test]
    fn update_line() {
        let mut f = LineFormatter::default();
        let e = NodeEvent::Updated {
            module: "t1".into(),
            parameter: "value".into(),
            line: Arc::from("update t1:value [4.2,{\"t\":1793118130.456}]"),
        };
        assert_eq!(
            format!(
                "{} {}",
                timestamp(&at(16, 42, 10, 456)),
                f.format(&e).unwrap()
            ),
            "2026-10-27T16:42:10.456+01:00 U update t1:value [4.2,{\"t\":1793118130.456}]"
        );
    }
}
