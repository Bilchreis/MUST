// SPDX-License-Identifier: LGPL-3.0-or-later

//! The filter of `SECoP_S_filterStoredCommand[2]` (a MUST addition): which stored
//! requests a host loop takes.

use std::ffi::c_int;

use must_types::validate_identifier;

use super::codes::{action, err};
use super::state::Pending;

/// A parsed filter: accessibles and an action.
#[derive(Debug, PartialEq)]
pub(crate) struct Filter {
    /// Entries of one to three parts (`node`, `node:module`, `node:module:accessible`);
    /// empty: every accessible.
    entries: Vec<Vec<String>>,
    /// 0: any action.
    action: c_int,
}

impl Filter {
    /// Parses `accessibles` (NULL: every accessible), entries separated by commas,
    /// semicolons or ASCII whitespace, and `action` (0 or a `SECoP_S_action`).
    /// `SECoP_S_ERROR_INVALID_NAME` for a malformed entry, `SECoP_S_ERROR_INVALID_VALUE`
    /// for an unknown action.
    pub(crate) fn parse(accessibles: Option<&str>, action: c_int) -> Result<Self, c_int> {
        if !matches!(
            action,
            0 | action::READ | action::CHANGE | action::DO | action::CHECK
        ) {
            return Err(err::INVALID_VALUE);
        }
        let entries = accessibles
            .unwrap_or_default()
            .split(|c: char| c == ',' || c == ';' || c.is_ascii_whitespace())
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let parts: Vec<String> = entry.split(':').map(str::to_owned).collect();
                if parts.len() > 3 || parts.iter().any(|p| validate_identifier(p).is_err()) {
                    Err(err::INVALID_NAME)
                } else {
                    Ok(parts)
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { entries, action })
    }

    /// Whether the filter selects a stored request.
    pub(crate) fn matches(&self, request: &Pending) -> bool {
        if self.action != 0 && self.action != request.action {
            return false;
        }
        if self.entries.is_empty() {
            return true;
        }
        let name = [
            request.node.as_str(),
            request.module.as_str(),
            request.accessible.as_str(),
        ];
        self.entries.iter().any(|parts| {
            parts
                .iter()
                .zip(name)
                .all(|(part, name)| part.eq_ignore_ascii_case(name))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(module: &str, accessible: &str, action: c_int) -> Pending {
        Pending {
            node: "cryo".into(),
            module: module.into(),
            accessible: accessible.into(),
            action,
            value: None,
            reply: tokio::sync::oneshot::channel().0,
        }
    }

    fn selects(filter: &str, action: c_int, requests: &[&Pending]) -> Vec<bool> {
        let filter = Filter::parse(Some(filter), action).expect("valid filter");
        requests.iter().map(|r| filter.matches(r)).collect()
    }

    #[test]
    fn entries() {
        let target = request("tc", "target", action::CHANGE);
        let ramp = request("tc", "ramp", action::READ);
        let valve = request("valve", "value", action::READ);
        let all = [&target, &ramp, &valve];
        assert_eq!(selects("cryo:tc:target", 0, &all), [true, false, false]);
        assert_eq!(selects("cryo:tc", 0, &all), [true, true, false]);
        assert_eq!(selects("cryo", 0, &all), [true, true, true]);
        assert_eq!(selects("other", 0, &all), [false, false, false]);
        assert_eq!(selects("", 0, &all), [true, true, true]);
        assert_eq!(selects("CRYO:TC:Target", 0, &all), [true, false, false]);
        // the separators, and a list as LabVIEW's Array To Spreadsheet String makes it
        assert_eq!(
            selects("cryo:tc:target, cryo:valve", 0, &all),
            [true, false, true]
        );
        assert_eq!(
            selects("cryo:tc:target;cryo:tc:ramp", 0, &all),
            [true, true, false]
        );
        assert_eq!(
            selects("cryo:tc:target\tcryo:valve:value\r\n", 0, &all),
            [true, false, true]
        );
        // the node comes first
        assert_eq!(selects("tc:target", 0, &all), [false, false, false]);
        let filter = Filter::parse(None, 0).unwrap();
        assert!(all.iter().all(|r| filter.matches(r)));
    }

    #[test]
    fn actions() {
        let target = request("tc", "target", action::CHANGE);
        let ramp = request("tc", "ramp", action::READ);
        let stop = request("tc", "stop", action::DO);
        let check = request("tc", "target", action::CHECK);
        let all = [&target, &ramp, &stop, &check];
        assert_eq!(selects("", action::READ, &all), [false, true, false, false]);
        assert_eq!(
            selects("", action::CHANGE, &all),
            [true, false, false, false]
        );
        assert_eq!(
            selects("cryo:tc", action::DO, &all),
            [false, false, true, false]
        );
        assert_eq!(
            selects("cryo:tc:target", action::CHECK, &all),
            [false, false, false, true]
        );
        for bad in [5, -1] {
            assert_eq!(Filter::parse(Some(""), bad), Err(err::INVALID_VALUE));
        }
    }

    #[test]
    fn malformed() {
        for bad in [
            "cryo:tc:target:x",
            "cryo:tc-1",
            "cryo::target",
            "cryo:",
            "1st",
        ] {
            assert_eq!(Filter::parse(Some(bad), 0), Err(err::INVALID_NAME), "{bad}");
        }
        // one bad entry spoils the list
        assert_eq!(
            Filter::parse(Some("cryo:tc, cryo:tc-1"), 0),
            Err(err::INVALID_NAME)
        );
    }
}
