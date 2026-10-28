// SPDX-License-Identifier: LGPL-3.0-or-later

//! Drawing the state; a pure function of [`App`] and [`Theme`].

use std::fmt::Write as _;

use chrono::{FixedOffset, Local, TimeZone};
use must_monitor::protocol::{Direction, NodeState};
use must_types::DataInfo;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Row, Table, Wrap};
use serde_json::Value as Json;

use crate::app::{App, Link, Node, Process, View};
use crate::format::{self, StatusGroup};

/// Every key binding, as the help overlay lists it. None of them changes a node.
pub const BINDINGS: &[(&str, &str)] = &[
    ("Tab / Shift+Tab", "next / previous view"),
    ("1 2 3 4", "overview, detail, traffic, logs"),
    (
        "Up / Down, j / k",
        "select a node (overview), scroll (other views)",
    ),
    ("Left / Right", "previous / next node"),
    ("PgUp / PgDn", "scroll by 10 lines"),
    ("Home / End", "oldest / newest lines"),
    ("Enter", "details of the selected node"),
    ("Esc", "back to the overview"),
    ("/", "filter (traffic, logs): Enter keeps it, Esc clears it"),
    ("p", "pause / resume the traffic view"),
    ("l", "log level: all, INFO+, WARN+, ERROR"),
    ("n", "logs of the selected node only, or of all"),
    ("o", "logs of the selected process only, or of all"),
    ("P", "pin the selected process (kept when it is gone)"),
    ("?", "this help"),
    ("q, Ctrl+C", "quit"),
];

/// How to draw: colors on or off (`NO_COLOR`), Unicode or ASCII borders, time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Use colors.
    pub color: bool,
    /// Use Unicode line drawing.
    pub unicode: bool,
    /// Show times in this offset from UTC instead of the local time zone (for tests).
    pub offset: Option<FixedOffset>,
}

impl Theme {
    /// From the environment: no colors with `NO_COLOR`; ASCII borders in the classic
    /// Windows console (not in Windows Terminal, which sets `WT_SESSION`).
    pub fn from_env() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        Self {
            color: !set("NO_COLOR"),
            unicode: !cfg!(windows) || set("WT_SESSION"),
            offset: None,
        }
    }

    fn fg(self, color: Color) -> Style {
        if self.color {
            Style::new().fg(color)
        } else {
            Style::new()
        }
    }

    fn status(self, group: StatusGroup) -> Style {
        self.fg(match group {
            StatusGroup::Disabled | StatusGroup::Unknown => Color::DarkGray,
            StatusGroup::Idle => Color::Green,
            StatusGroup::Warn => Color::Yellow,
            StatusGroup::Busy => Color::Blue,
            StatusGroup::Error => Color::Red,
        })
    }

    fn block(self, title: &str) -> Block<'static> {
        let block = Block::bordered().title(format!(" {title} "));
        if self.unicode {
            block
        } else {
            block.border_set(ascii_border())
        }
    }

    fn dim(self) -> Style {
        self.fg(Color::DarkGray)
    }

    fn error(self) -> Style {
        self.fg(Color::Red)
    }

    fn warn(self) -> Style {
        self.fg(Color::Yellow)
    }
}

fn ascii_border() -> border::Set<'static> {
    border::Set {
        top_left: "+",
        top_right: "+",
        bottom_left: "+",
        bottom_right: "+",
        vertical_left: "|",
        vertical_right: "|",
        horizontal_top: "-",
        horizontal_bottom: "-",
    }
}

/// Draws the whole screen.
pub fn draw(frame: &mut Frame, app: &App, theme: Theme) {
    let [tabs, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_tabs(frame, tabs, app, theme);
    match app.view {
        View::Overview => draw_overview(frame, body, app, theme),
        View::Detail => draw_detail(frame, body, app, theme),
        View::Traffic => draw_traffic(frame, body, app, theme),
        View::Logs => draw_logs(frame, body, app, theme),
    }
    draw_footer(frame, footer, app, theme);
    if app.help {
        draw_help(frame, app, theme);
    }
}

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let mut spans = vec![Span::styled(
        " must-tui ",
        Style::new().add_modifier(Modifier::BOLD),
    )];
    for (i, view) in View::ALL.iter().enumerate() {
        let title = format!(" {} {} ", i + 1, view.title());
        if *view == app.view {
            let text = if theme.color {
                title
            } else {
                format!("[{}]", title.trim_end())
            };
            spans.push(Span::styled(
                text,
                Style::new().add_modifier(Modifier::REVERSED),
            ));
        } else {
            spans.push(Span::raw(title));
        }
    }
    let nodes: usize = app.processes.values().map(|p| p.nodes.len()).sum();
    spans.push(Span::styled(
        format!(
            "  {} process{}, {nodes} node{}",
            app.processes.len(),
            if app.processes.len() == 1 { "" } else { "es" },
            if nodes == 1 { "" } else { "s" }
        ),
        theme.dim(),
    ));
    frame.render_widget(Line::from(spans), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let line = if app.ui.editing_filter {
        Line::from(vec![
            Span::raw(" filter: "),
            Span::styled(
                format!("{}_", app.ui.filter),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::styled("   Enter keep · Esc clear", theme.dim()),
        ])
    } else {
        let hints = match app.view {
            View::Overview => "↑↓ select · Enter details · Tab views · P pin · ? help · q quit",
            View::Detail => "↑↓ scroll · ←→ node · Tab views · ? help · q quit",
            View::Traffic => "/ filter · p pause · ←→ node · ↑↓ scroll · ? help · q quit",
            View::Logs => "/ filter · l level · n node · o process · ↑↓ scroll · ? help · q quit",
        };
        let hints = if theme.unicode {
            hints.to_owned()
        } else {
            hints
                .replace("↑↓", "Up/Down")
                .replace("←→", "Left/Right")
                .replace('·', "|")
        };
        Line::styled(format!(" {hints}"), theme.dim())
    };
    frame.render_widget(line, area);
}

// --- overview

/// Columns of the overview: header and width (the node column takes the rest).
fn overview_columns(width: u16) -> Vec<(&'static str, usize)> {
    let mut columns = vec![("CONTEXT", 10)];
    if width >= 100 {
        columns.push(("ADDRESS", 21));
    }
    columns.extend([("STATE", 8), ("CL", 3), ("MSG/S", 6), ("STATUS", 18)]);
    columns
}

fn draw_overview(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let columns = overview_columns(area.width);
    let inner = usize::from(area.width.saturating_sub(2));
    let fixed: usize = columns.iter().map(|(_, w)| w + 1).sum();
    let name_width = inner.saturating_sub(fixed).max(12);

    let mut header = vec![Span::raw(pad("NODE", name_width + 1))];
    header.extend(columns.iter().map(|(h, w)| Span::raw(pad(h, w + 1))));
    let header = Line::from(header).style(Style::new().add_modifier(Modifier::BOLD));

    let mut lines = Vec::new();
    let mut selected_line = None;
    for (key, process) in &app.processes {
        lines.push(process_line(process, app.now, theme));
        if process.link == Link::Connected && process.nodes.is_empty() {
            lines.push(Line::styled("  (no nodes)", theme.dim()));
        }
        for (id, node) in &process.nodes {
            let mut line = node_line(
                node,
                process.gone_since.is_some(),
                &columns,
                name_width,
                theme,
            );
            if app.selected == Some((*key, *id)) {
                selected_line = Some(lines.len());
                line = line.patch_style(Style::new().add_modifier(Modifier::REVERSED));
            }
            lines.push(line);
        }
    }
    if app.processes.is_empty() {
        lines.push(Line::styled(
            "No must nodes are running for this user.",
            theme.dim(),
        ));
    }
    // keep the selection in view
    let visible = usize::from(area.height.saturating_sub(3));
    let offset = selected_line.map_or(0, |l| l.saturating_sub(visible.saturating_sub(1)));
    let mut text = vec![header];
    text.extend(lines.into_iter().skip(offset));
    frame.render_widget(Paragraph::new(text).block(theme.block("Nodes")), area);
}

/// `text` cut or padded to exactly `width` characters.
fn pad(text: &str, width: usize) -> String {
    let cut = format::shorten(text, width.saturating_sub(1));
    format!("{cut:<width$}")
}

fn process_line(process: &Process, now: f64, theme: Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{} ({})", process.name, process.pid),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    if !process.library_version.is_empty() {
        spans.push(Span::styled(
            format!("  must {}", process.library_version),
            theme.dim(),
        ));
    }
    match &process.link {
        Link::Connecting => spans.push(Span::styled("  connecting…", theme.dim())),
        Link::Connected => {}
        Link::Failed(reason) => {
            spans.push(Span::styled(
                format!("  not reachable: {reason}"),
                theme.warn(),
            ));
        }
        Link::Refused(reason) => {
            spans.push(Span::styled(
                format!("  version error: {reason}"),
                theme.error(),
            ));
        }
    }
    if let Some(since) = process.gone_since {
        spans.push(Span::styled(
            format!("  GONE {}", format::age(now - since)),
            theme.warn(),
        ));
    }
    if process.pinned {
        spans.push(Span::raw("  PINNED"));
    }
    if process.lagged > 0 {
        spans.push(Span::styled(
            format!("  lagged: {} events dropped", process.lagged),
            theme.warn(),
        ));
    }
    Line::from(spans)
}

fn node_line(
    node: &Node,
    gone: bool,
    columns: &[(&str, usize)],
    name_width: usize,
    theme: Theme,
) -> Line<'static> {
    let info = &node.info;
    let running = info.state == NodeState::Running && !gone;
    let mut spans = vec![Span::raw(pad(
        &format!("  {}", info.equipment_id),
        name_width + 1,
    ))];
    for (column, width) in columns {
        let w = width + 1;
        let span = match *column {
            "CONTEXT" => Span::raw(pad(info.context.as_deref().unwrap_or_default(), w)),
            "ADDRESS" => Span::raw(pad(info.address.as_deref().unwrap_or_default(), w)),
            "STATE" => {
                let (label, style) = if gone {
                    ("gone", theme.dim())
                } else {
                    let style = match info.state {
                        NodeState::Failed => theme.error(),
                        NodeState::Running => theme.fg(Color::Green),
                        _ => theme.dim(),
                    };
                    (state_label(info.state), style)
                };
                Span::styled(pad(label, w), style)
            }
            "CL" if running => Span::raw(pad(&info.clients.len().to_string(), w)),
            "MSG/S" if running => Span::raw(pad(
                &format!("{:.1}", node.rx_per_second + node.tx_per_second),
                w,
            )),
            "STATUS" if running => match worst_status(node) {
                Some((group, module)) => Span::styled(
                    pad(&format!("{} {module}", group.label()), w),
                    theme.status(group),
                ),
                None => Span::raw(pad("", w)),
            },
            _ => Span::raw(pad("", w)),
        };
        spans.push(span);
    }
    Line::from(spans)
}

fn state_label(state: NodeState) -> &'static str {
    match state {
        NodeState::Building => "building",
        NodeState::Failed => "failed",
        NodeState::Running => "running",
        NodeState::Stopped => "stopped",
    }
}

/// The most severe module status of a node, with its module.
pub fn worst_status(node: &Node) -> Option<(StatusGroup, String)> {
    node.parameters
        .values()
        .filter(|p| p.parameter == "status" && !p.error)
        .filter_map(|p| {
            let (code, _) = format::status_of(&p.report)?;
            Some((StatusGroup::of(code), p.module.clone()))
        })
        .max_by_key(|(group, _)| group.severity())
}

// --- detail

fn draw_detail(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let (Some(node), Some(process)) = (app.selected_node(), app.selected_process()) else {
        frame.render_widget(
            Paragraph::new("No node selected.").block(theme.block("Detail")),
            area,
        );
        return;
    };
    let clients = node.info.clients.len();
    let clients_height = if clients == 0 {
        3
    } else {
        u16::try_from(clients.min(5)).unwrap_or(5) + 3
    };
    let [top, bottom] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(clients_height)]).areas(area);

    let mut lines = header_lines(node, process, area.width, theme);
    lines.push(Line::default());
    lines.extend(module_lines(node, app.now, theme));
    let title = format!("Detail: {}", node.info.equipment_id);
    let scroll = u16::try_from(app.ui.scroll).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(theme.block(&title))
            .scroll((scroll, 0)),
        top,
    );
    frame.render_widget(clients_table(node, app.now, theme), bottom);
}

fn header_lines(node: &Node, process: &Process, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let info = &node.info;
    let mut first = vec![
        Span::styled(
            info.equipment_id.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  {}", state_label(info.state))),
    ];
    if let Some(address) = &info.address {
        first.push(Span::raw(format!(" on {address}")));
    }
    if let Some(context) = &info.context {
        first.push(Span::raw(format!("  context {context}")));
    }
    first.push(Span::styled(
        format!("  {} ({})", process.name, process.pid),
        theme.dim(),
    ));
    let mut lines = vec![Line::from(first)];
    let log = match &info.log_file {
        Some(path) => {
            let room = usize::from(width.saturating_sub(10)).max(20);
            Span::raw(shorten_middle(path, room))
        }
        None => Span::styled("no log file (file logging off or stopped)", theme.dim()),
    };
    lines.push(Line::from(vec![Span::raw("log  "), log]));
    let failed = info.state == NodeState::Failed;
    for message in &info.messages {
        let (label, style) = if failed {
            ("error   ", theme.error())
        } else {
            ("warning ", theme.warn())
        };
        lines.push(Line::from(vec![
            Span::styled(label, style),
            Span::raw(message.clone()),
        ]));
    }
    lines
}

/// Cuts the middle of a long path: `/home/u/…/cryo/2026-10-27.log`.
pub fn shorten_middle(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    let tail = max * 2 / 3;
    let head = max.saturating_sub(tail + 1);
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}…{end}")
}

/// Modules, their status and their parameters, in the order of the structure report.
fn module_lines(node: &Node, now: f64, theme: Theme) -> Vec<Line<'static>> {
    let Some(modules) = node
        .info
        .structure
        .as_ref()
        .and_then(|s| s.get("modules"))
        .and_then(Json::as_object)
    else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for (mname, module) in modules {
        let mut head = vec![Span::styled(
            mname.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        )];
        if let Some(state) = node.parameters.get(&(mname.clone(), "status".to_owned()))
            && !state.error
            && let Some((code, text)) = format::status_of(&state.report)
        {
            let group = StatusGroup::of(code);
            head.push(Span::raw("  "));
            head.push(Span::styled(
                format!("{} {text}", group.label()),
                theme.status(group).add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(Line::from(head));
        let Some(accessibles) = module.get("accessibles").and_then(Json::as_object) else {
            continue;
        };
        for (pname, accessible) in accessibles {
            let datainfo_json = accessible.get("datainfo");
            if datainfo_json
                .and_then(|d| d.get("type"))
                .and_then(Json::as_str)
                == Some("command")
            {
                continue;
            }
            let datainfo = datainfo_json.and_then(|d| DataInfo::from_json(d).ok());
            lines.push(parameter_line(
                node,
                mname,
                pname,
                datainfo.as_ref(),
                now,
                theme,
            ));
        }
    }
    lines
}

fn parameter_line(
    node: &Node,
    module: &str,
    parameter: &str,
    datainfo: Option<&DataInfo>,
    now: f64,
    theme: Theme,
) -> Line<'static> {
    let name = Span::raw(format!("  {parameter:<18} "));
    let Some(state) = node
        .parameters
        .get(&(module.to_owned(), parameter.to_owned()))
    else {
        return Line::from(vec![name, Span::styled("-", theme.dim())]);
    };
    let age = format::timestamp_of(&state.report, state.error)
        .map(|t| format!("  {}", format::age(now - t)))
        .unwrap_or_default();
    if state.error {
        let (class, message) = format::error_of(&state.report);
        Line::from(vec![
            name,
            Span::styled(format!("! {class}: {message}"), theme.error()),
            Span::styled(age, theme.dim()),
        ])
    } else {
        let value = state.report.get(0).cloned().unwrap_or(Json::Null);
        Line::from(vec![
            name,
            Span::raw(format::value(datainfo, &value)),
            Span::styled(age, theme.dim()),
        ])
    }
}

fn clients_table(node: &Node, now: f64, theme: Theme) -> Table<'static> {
    let rows: Vec<Row> = node
        .info
        .clients
        .iter()
        .map(|c| {
            let active = if c.active_all {
                "all".to_owned()
            } else if c.active_modules.is_empty() {
                "-".to_owned()
            } else {
                c.active_modules.join(", ")
            };
            Row::new(vec![
                c.id.to_string(),
                c.peer.clone(),
                format!("{} ago", format::age(now - c.connected_at)),
                active,
            ])
        })
        .collect();
    let title = format!("Clients ({})", node.info.clients.len());
    Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Min(8),
        ],
    )
    .header(
        Row::new(vec!["ID", "ADDRESS", "CONNECTED", "ACTIVATED"])
            .style(Style::new().add_modifier(Modifier::BOLD)),
    )
    .block(theme.block(&title))
}

// --- traffic

fn clock(time: f64, theme: Theme) -> String {
    #[allow(clippy::cast_possible_truncation)]
    let millis = (time * 1000.0).round() as i64;
    let formatted = match theme.offset {
        Some(offset) => offset
            .timestamp_millis_opt(millis)
            .single()
            .map(|t| t.format("%H:%M:%S%.3f").to_string()),
        None => Local
            .timestamp_millis_opt(millis)
            .single()
            .map(|t| t.format("%H:%M:%S%.3f").to_string()),
    };
    formatted.unwrap_or_else(|| format!("{time:.3}"))
}

/// The last `rows` of `lines` before the scroll position (back from the newest).
fn window<T>(lines: &[T], rows: usize, scroll: usize) -> &[T] {
    let end = lines.len().saturating_sub(scroll);
    let start = end.saturating_sub(rows);
    &lines[start..end]
}

fn draw_traffic(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let Some(node) = app.selected_node() else {
        frame.render_widget(
            Paragraph::new("No node selected.").block(theme.block("Traffic")),
            area,
        );
        return;
    };
    let mut title = format!("Traffic: {}", node.info.equipment_id);
    if app.ui.paused_at.is_some() {
        let _ = write!(title, " | PAUSED, {} new", app.held_back());
    }
    if !app.ui.filter.is_empty() {
        let _ = write!(title, " | filter \"{}\"", app.ui.filter);
    }
    let running = node.info.state == NodeState::Running
        && app
            .selected_process()
            .is_some_and(|p| p.gone_since.is_none());
    let lines = app.visible_traffic();
    let rows = usize::from(area.height.saturating_sub(2));
    let text: Vec<Line> = if lines.is_empty() {
        let waiting = if running {
            "Waiting for messages…"
        } else {
            "The node is not running."
        };
        vec![Line::styled(waiting, theme.dim())]
    } else {
        window(&lines, rows, app.ui.scroll)
            .iter()
            .map(|l| {
                let (arrow, style) = match l.direction {
                    Direction::Received => (">", theme.fg(Color::Cyan)),
                    Direction::Sent => ("<", Style::new()),
                };
                let mut spans = vec![
                    Span::styled(format!("{} ", clock(l.time, theme)), theme.dim()),
                    Span::styled(format!("{arrow} [{}] ", l.client), style),
                    Span::raw(l.text.clone()),
                ];
                if l.truncated {
                    spans.push(Span::styled(" …[cut]", theme.warn()));
                }
                Line::from(spans)
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(text).block(theme.block(&title)), area);
}

// --- logs

fn draw_logs(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let mut title = format!("Logs | level {}", app.ui.log_level.label());
    if app.ui.log_node_only {
        let name = app
            .selected
            .map_or_else(String::new, |(key, id)| app.node_name(key, Some(id)));
        let _ = write!(title, " | node {name}");
    }
    if app.ui.log_process_only
        && let Some(p) = app.selected_process()
    {
        let _ = write!(title, " | process {} ({})", p.name, p.pid);
    }
    if !app.ui.filter.is_empty() {
        let _ = write!(title, " | filter \"{}\"", app.ui.filter);
    }
    let lines = app.visible_logs();
    let rows = usize::from(area.height.saturating_sub(2));
    let text: Vec<Line> = window(&lines, rows, app.ui.scroll)
        .iter()
        .map(|l| {
            let style = match l.record.level.as_str() {
                "ERROR" => theme.error(),
                "WARN" => theme.warn(),
                _ => Style::new(),
            };
            let source = app.node_name(l.key, l.record.node);
            let module = l
                .record
                .module
                .as_deref()
                .filter(|m| !m.is_empty())
                .map(|m| format!("{m}: "))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(format!("{} ", clock(l.record.time, theme)), theme.dim()),
                Span::styled(format!("{:<5} ", l.record.level), style),
                Span::styled(
                    format!("{source} "),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{module}{}", l.record.message)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(text).block(theme.block(&title)), area);
}

// --- help

fn draw_help(frame: &mut Frame, app: &App, theme: Theme) {
    let area = frame.area();
    let width = area.width.min(76);
    let height = u16::try_from(BINDINGS.len() + 4)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let mut lines: Vec<Line> = BINDINGS
        .iter()
        .map(|(keys, what)| {
            Line::from(vec![
                Span::styled(
                    format!(" {keys:<18}"),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw(*what),
            ])
        })
        .collect();
    lines.push(Line::default());
    lines.push(Line::styled(
        " Read-only: nothing here changes a node. Any key closes this help.",
        theme.dim(),
    ));
    let _ = app;
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(theme.block("Keys"))
            .wrap(Wrap { trim: false }),
        popup,
    );
}
