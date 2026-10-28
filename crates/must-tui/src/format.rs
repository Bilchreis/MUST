// SPDX-License-Identifier: LGPL-3.0-or-later

//! Turning reports into text: values by their datainfo, status groups, ages.

use must_types::DataInfo;
use serde_json::Value as Json;

/// Longest text shown for a value; longer ones end in `…`.
pub const MAX_VALUE_CHARS: usize = 60;

/// A SECoP status group, by status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusGroup {
    /// 0.
    Disabled,
    /// 1xx.
    Idle,
    /// 3xx.
    Busy,
    /// 2xx.
    Warn,
    /// 4xx.
    Error,
    /// Any other code.
    Unknown,
}

impl StatusGroup {
    /// The group of a status code.
    pub fn of(code: i64) -> Self {
        match code {
            0 => Self::Disabled,
            100..=199 => Self::Idle,
            200..=299 => Self::Warn,
            300..=399 => Self::Busy,
            400..=499 => Self::Error,
            _ => Self::Unknown,
        }
    }

    /// The label shown next to the color.
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "DISABLED",
            Self::Idle => "IDLE",
            Self::Warn => "WARN",
            Self::Busy => "BUSY",
            Self::Error => "ERROR",
            Self::Unknown => "?",
        }
    }

    /// How much attention it needs, for "most severe": errors, then warnings, then busy.
    pub fn severity(self) -> u8 {
        match self {
            Self::Disabled => 0,
            Self::Idle => 1,
            Self::Busy => 2,
            Self::Warn => 3,
            Self::Unknown => 4,
            Self::Error => 5,
        }
    }
}

/// Code and text of a status data report `[[code, "text"], {..}]`.
pub fn status_of(report: &Json) -> Option<(i64, String)> {
    let value = report.get(0)?;
    let code = value.get(0)?.as_i64()?;
    let text = value.get(1).and_then(Json::as_str).unwrap_or_default();
    Some((code, text.to_owned()))
}

/// Class and message of an error report `["Class", "message", {..}]`.
pub fn error_of(report: &Json) -> (String, String) {
    let class = report.get(0).and_then(Json::as_str).unwrap_or("Error");
    let message = report.get(1).and_then(Json::as_str).unwrap_or_default();
    (class.to_owned(), message.to_owned())
}

/// The timestamp `t` of a data or error report.
pub fn timestamp_of(report: &Json, error: bool) -> Option<f64> {
    let qualifiers = report.get(if error { 2 } else { 1 })?;
    qualifiers.get("t")?.as_f64()
}

/// An age like `3s`, `12m`, `5h`, `2d`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn age(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// A value from a data report, formatted by its datainfo (format string, unit, enum
/// names); without datainfo, the JSON text.
pub fn value(datainfo: Option<&DataInfo>, value: &Json) -> String {
    let text = match datainfo {
        Some(info) => with_unit(info, value),
        None => value.to_string(),
    };
    shorten(&text, MAX_VALUE_CHARS)
}

/// Cuts `text` to `max` characters, ending in `…`.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn with_unit(info: &DataInfo, value: &Json) -> String {
    let text = plain(info, value);
    match unit(info) {
        Some(unit) if !unit.is_empty() => format!("{text} {unit}"),
        _ => text,
    }
}

fn unit(info: &DataInfo) -> Option<&str> {
    match info {
        DataInfo::Double(d) => d.unit(),
        DataInfo::Scaled(s) => s.unit(),
        DataInfo::Int(i) => i.unit(),
        _ => None,
    }
}

fn plain(info: &DataInfo, value: &Json) -> String {
    match (info, value) {
        (DataInfo::Double(d), Json::Number(n)) => match n.as_f64() {
            Some(x) => printf(d.fmtstr(), x),
            None => n.to_string(),
        },
        (DataInfo::Scaled(s), Json::Number(n)) => match n.as_i64() {
            Some(raw) => printf(&s.fmtstr(), s.to_physical(raw)),
            None => n.to_string(),
        },
        (DataInfo::Enum(e), Json::Number(n)) => match n.as_i64().and_then(|v| e.name_of(v)) {
            Some(name) => name.to_owned(),
            None => n.to_string(),
        },
        (DataInfo::Blob(_), Json::String(s)) => format!("<{} bytes>", s.len() * 3 / 4),
        (DataInfo::Array(a), Json::Array(items)) => {
            let inner: Vec<String> = items.iter().map(|v| with_unit(a.members(), v)).collect();
            format!("[{}]", inner.join(", "))
        }
        (DataInfo::Tuple(t), Json::Array(items)) => {
            let inner: Vec<String> = t
                .members()
                .iter()
                .zip(items)
                .map(|(m, v)| with_unit(m, v))
                .collect();
            format!("({})", inner.join(", "))
        }
        (DataInfo::Struct(s), Json::Object(map)) => {
            let inner: Vec<String> = s
                .members()
                .iter()
                .filter_map(|(name, m)| Some(format!("{name}: {}", with_unit(m, map.get(name)?))))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        (_, Json::String(s)) => s.clone(),
        (_, other) => other.to_string(),
    }
}

/// Formats `x` with a SECoP format string (`%.<n>f`, `%.<n>e`, `%.<n>g`).
pub fn printf(fmtstr: &str, x: f64) -> String {
    let spec = fmtstr.strip_prefix('%').unwrap_or("");
    let (precision, conversion) = match spec.strip_prefix('.') {
        Some(rest) => {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let conversion = rest[digits.len()..].chars().next();
            (digits.parse::<usize>().ok(), conversion)
        }
        None => (None, spec.chars().next()),
    };
    if !x.is_finite() {
        return x.to_string();
    }
    match conversion {
        Some('f' | 'F') => format!("{x:.*}", precision.unwrap_or(6)),
        Some('e' | 'E') => exponential(x, precision.unwrap_or(6)),
        _ => general(x, precision.unwrap_or(6).max(1)),
    }
}

/// `%e`: mantissa with `precision` decimals, exponent with sign and at least two digits.
fn exponential(x: f64, precision: usize) -> String {
    let text = format!("{x:.precision$e}");
    match text.split_once('e') {
        Some((mantissa, exponent)) => {
            let (sign, digits) = match exponent.strip_prefix('-') {
                Some(d) => ('-', d),
                None => ('+', exponent),
            };
            format!("{mantissa}e{sign}{digits:0>2}")
        }
        None => text,
    }
}

/// `%g`: `precision` significant digits, trailing zeros removed, exponential outside
/// 1e-4 .. 10^precision.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn general(x: f64, precision: usize) -> String {
    if x == 0.0 {
        return "0".to_owned();
    }
    let rounded = format!("{x:.*e}", precision - 1);
    let exponent: i32 = rounded
        .split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0);
    if exponent < -4 || exponent >= precision as i32 {
        let text = exponential(x, precision - 1);
        match text.split_once('e') {
            Some((mantissa, exp)) => format!("{}e{exp}", trim_zeros(mantissa)),
            None => text,
        }
    } else {
        let decimals = usize::try_from(precision as i32 - 1 - exponent).unwrap_or(0);
        trim_zeros(&format!("{x:.decimals$}"))
    }
}

fn trim_zeros(text: &str) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn info(j: &Json) -> DataInfo {
        DataInfo::from_json(j).unwrap()
    }

    #[test]
    fn printf_subset() {
        assert_eq!(printf("%.6g", 4.2), "4.2");
        assert_eq!(printf("%.6g", 295.0), "295");
        assert_eq!(printf("%.3g", 1234.5), "1.23e+03");
        assert_eq!(printf("%.6g", 0.000_012_5), "1.25e-05");
        assert_eq!(printf("%.6g", 0.0), "0");
        assert_eq!(printf("%.2f", 4.256), "4.26");
        assert_eq!(printf("%.3e", 12345.0), "1.234e+04");
        assert_eq!(printf("%.1e", -0.05), "-5.0e-02");
        assert_eq!(printf("%.6g", f64::NAN), "NaN");
    }

    #[test]
    fn values_by_datainfo() {
        let kelvin = info(&json!({"type": "double", "unit": "K"}));
        assert_eq!(value(Some(&kelvin), &json!(4.2)), "4.2 K");
        let fixed = info(&json!({"type": "double", "unit": "K", "fmtstr": "%.3f"}));
        assert_eq!(value(Some(&fixed), &json!(4.2)), "4.200 K");
        let scaled = info(
            &json!({"type": "scaled", "scale": 0.01, "min": 0, "max": 100_000, "unit": "mbar"}),
        );
        assert_eq!(value(Some(&scaled), &json!(12345)), "123.45 mbar");
        let mode = info(&json!({"type": "enum", "members": {"off": 0, "on": 1}}));
        assert_eq!(value(Some(&mode), &json!(1)), "on");
        assert_eq!(value(Some(&mode), &json!(7)), "7");
        let status = info(&json!({"type": "tuple", "members": [
            {"type": "enum", "members": {"IDLE": 100, "BUSY": 300}}, {"type": "string"}]}));
        assert_eq!(
            value(Some(&status), &json!([300, "ramping"])),
            "(BUSY, ramping)"
        );
        let point = info(&json!({"type": "struct", "members": {
            "x": {"type": "double", "unit": "mm"}, "y": {"type": "int", "min": 0, "max": 9}}}));
        assert_eq!(
            value(Some(&point), &json!({"x": 1.5, "y": 3})),
            "{x: 1.5 mm, y: 3}"
        );
        let list = info(&json!({"type": "array", "maxlen": 9, "members": {"type": "double"}}));
        assert_eq!(value(Some(&list), &json!([1.0, 2.5])), "[1, 2.5]");
        assert_eq!(value(None, &json!([1, "a"])), "[1,\"a\"]");
        let long = "x".repeat(100);
        assert_eq!(value(None, &json!(long)).chars().count(), MAX_VALUE_CHARS);
    }

    #[test]
    fn statuses_errors_ages() {
        assert_eq!(StatusGroup::of(300), StatusGroup::Busy);
        assert_eq!(StatusGroup::of(0).label(), "DISABLED");
        assert!(StatusGroup::Error.severity() > StatusGroup::Warn.severity());
        assert!(StatusGroup::Warn.severity() > StatusGroup::Busy.severity());
        assert_eq!(
            status_of(&json!([[300, "ramping"], {"t": 1.0}])),
            Some((300, "ramping".to_owned()))
        );
        assert_eq!(
            error_of(&json!(["HardwareError", "sensor disconnected", {"t": 2.0}])),
            ("HardwareError".to_owned(), "sensor disconnected".to_owned())
        );
        assert_eq!(timestamp_of(&json!([1, {"t": 5.5}]), false), Some(5.5));
        assert_eq!(
            timestamp_of(&json!(["X", "m", {"t": 6.5}]), true),
            Some(6.5)
        );
        assert_eq!(age(3.7), "3s");
        assert_eq!(age(125.0), "2m");
        assert_eq!(age(7200.0), "2h");
        assert_eq!(age(200_000.0), "2d");
    }
}
