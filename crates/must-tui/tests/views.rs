// SPDX-License-Identifier: LGPL-3.0-or-later

//! The views, drawn into a test terminal: snapshots of the screens (insta) and checks of
//! colors, filters and pausing.

use must_monitor::protocol::{
    ClientInfo, Direction, Event, LogRecord, NodeInfo, NodeState, ParameterState, ProcessInfo,
    ServerMessage, Snapshot,
};
use must_tui::app::{App, EndpointMsg, Msg, ProcessKey, View};
use must_tui::ui::{self, BINDINGS, Theme};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;
use serde_json::{Value as Json, json};

const NOW: f64 = 1_793_118_200.0;
const LABVIEW: ProcessKey = (4242, 1);
const PYTHON: ProcessKey = (777, 2);

fn theme() -> Theme {
    Theme {
        color: true,
        unicode: true,
        offset: chrono::FixedOffset::east_opt(3600),
    }
}

fn no_color() -> Theme {
    Theme {
        color: false,
        ..theme()
    }
}

fn parameter(module: &str, parameter: &str, report: Json, error: bool) -> ParameterState {
    ParameterState {
        module: module.into(),
        parameter: parameter.into(),
        report,
        error,
    }
}

fn node(id: u64, name: &str, state: NodeState, context: Option<&str>) -> NodeInfo {
    NodeInfo {
        id,
        equipment_id: name.into(),
        address: None,
        state,
        context: context.map(Into::into),
        messages: vec![],
        structure: None,
        parameters: vec![],
        clients: vec![],
        log_file: None,
    }
}

fn structure() -> Json {
    json!({"equipment_id": "cryo", "modules": {"t1": {"accessibles": {
        "value": {"datainfo": {"type": "double", "unit": "K", "fmtstr": "%.3f"}},
        "status": {"datainfo": {"type": "tuple", "members": [
            {"type": "enum", "members": {"IDLE": 100, "BUSY": 300}}, {"type": "string"}]}},
        "target": {"datainfo": {"type": "double", "unit": "K"}},
        "stop": {"datainfo": {"type": "command"}}
    }}}})
}

fn cryo() -> NodeInfo {
    NodeInfo {
        address: Some("0.0.0.0:2055".into()),
        structure: Some(structure()),
        parameters: vec![
            parameter(
                "t1",
                "value",
                json!(["HardwareError", "sensor disconnected", {"t": NOW - 3.0}]),
                true,
            ),
            parameter(
                "t1",
                "status",
                json!([[300, "ramping"], {"t": NOW - 1.0}]),
                false,
            ),
            parameter("t1", "target", json!([300.0, {"t": NOW - 10.0}]), false),
        ],
        clients: vec![ClientInfo {
            id: 3,
            peer: "192.168.1.20:51234".into(),
            connected_at: NOW - 125.0,
            active_all: true,
            active_modules: vec![],
        }],
        log_file: Some("/home/u/.local/state/must/logs/cryo/2026-10-27.log".into()),
        ..node(1, "cryo", NodeState::Running, Some("lv_a"))
    }
}

fn log(time: f64, level: &str, node: Option<u64>, module: Option<&str>, text: &str) -> LogRecord {
    LogRecord {
        time,
        level: level.into(),
        node,
        module: module.map(Into::into),
        message: text.into(),
    }
}

fn process(name: &str, pid: u32) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.into(),
        library_version: "0.1.0".into(),
        start_time: NOW - 3600.0,
    }
}

fn connected(key: ProcessKey, process: ProcessInfo, snapshot: Snapshot) -> Msg {
    Msg::Endpoint {
        key,
        msg: EndpointMsg::Connected { process, snapshot },
    }
}

/// Two processes: LabVIEW with nodes `cryo` (running, BUSY, an error), `magnet`
/// (failed) and `probe` (building); python with node `test`.
fn app() -> App {
    let mut app = App::new(NOW, None);
    let mut magnet = node(2, "magnet", NodeState::Failed, Some("lv_a"));
    magnet.messages = vec!["magnet:m1:value: no datainfo given".into()];
    let labview = Snapshot {
        process: process("LabVIEW.exe", 4242),
        nodes: vec![
            cryo(),
            magnet,
            node(3, "probe", NodeState::Building, Some("lv_b")),
        ],
        logs: vec![
            log(NOW - 50.0, "INFO", None, None, "library started"),
            log(
                NOW - 40.0,
                "WARN",
                Some(1),
                Some("t1"),
                "reading value failed",
            ),
            log(
                NOW - 30.0,
                "ERROR",
                Some(1),
                Some("t1"),
                "sensor disconnected",
            ),
            log(NOW - 20.0, "ERROR", Some(2), None, "no datainfo"),
        ],
    };
    let mut test = node(1, "test", NodeState::Running, None);
    test.address = Some("127.0.0.1:2056".into());
    test.structure = Some(json!({"modules": {"s": {"accessibles": {
        "status": {"datainfo": {"type": "tuple", "members": [
            {"type": "enum", "members": {"IDLE": 100}}, {"type": "string"}]}}}}}}));
    test.parameters = vec![parameter(
        "s",
        "status",
        json!([[100, "ok"], {"t": NOW}]),
        false,
    )];
    let python = Snapshot {
        process: process("python3", 777),
        nodes: vec![test],
        logs: vec![log(NOW - 25.0, "ERROR", Some(1), None, "python node error")],
    };
    for (key, name) in [(LABVIEW, "LabVIEW.exe"), (PYTHON, "python3")] {
        app.update(Msg::Endpoint {
            key,
            msg: EndpointMsg::Connecting { name: name.into() },
        });
    }
    app.update(connected(LABVIEW, labview.process.clone(), labview));
    app.update(connected(PYTHON, python.process.clone(), python));
    for (key, node, rx, tx) in [(LABVIEW, 1, 2.0, 3.5), (PYTHON, 1, 0.0, 0.0)] {
        app.update(Msg::Endpoint {
            key,
            msg: EndpointMsg::Message(ServerMessage::Counters {
                node,
                rx: 0,
                tx: 0,
                rx_per_second: rx,
                tx_per_second: tx,
            }),
        });
    }
    app
}

fn press(app: &mut App, code: KeyCode) -> Vec<must_tui::app::Effect> {
    app.update(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn terminal(app: &App, width: u16, height: u16, theme: Theme) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app, theme)).unwrap();
    terminal
}

fn screen(app: &App, width: u16, height: u16) -> String {
    terminal(app, width, height, theme()).backend().to_string()
}

/// Foreground colors of the cells that show `needle` (first occurrence).
fn colors_of(terminal: &Terminal<TestBackend>, needle: &str) -> Vec<Color> {
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let cells = &buffer.content;
    for row in cells.chunks(width) {
        let text: String = row.iter().map(ratatui::buffer::Cell::symbol).collect();
        if let Some(byte) = text.find(needle) {
            let start = text[..byte].chars().count();
            return row[start..start + needle.chars().count()]
                .iter()
                .map(|c| c.fg)
                .collect();
        }
    }
    panic!("{needle:?} not on screen");
}

fn message(node: u64, direction: Direction, time: f64, text: &str) -> Msg {
    Msg::Endpoint {
        key: LABVIEW,
        msg: EndpointMsg::Message(ServerMessage::Event(Event::Message {
            node,
            client: 3,
            direction,
            time,
            text: text.into(),
            truncated: false,
        })),
    }
}

#[test]
fn overview_several_hosts() {
    let app = app();
    let small = screen(&app, 80, 24);
    for name in [
        "LabVIEW.exe (4242)",
        "python3 (777)",
        "cryo",
        "magnet",
        "probe",
        "test",
    ] {
        assert!(small.contains(name), "{name} in\n{small}");
    }
    insta::assert_snapshot!("overview_80x24", small);
    insta::assert_snapshot!("overview_120x40", screen(&app, 120, 40));
}

#[test]
fn overview_states_contexts_and_status_colors() {
    let app = app();
    let shown = screen(&app, 100, 20);
    let line = |name: &str| {
        shown
            .lines()
            .find(|l| l.contains(&format!("  {name} ")))
            .unwrap()
            .to_owned()
    };
    assert!(line("probe").contains("building") && line("probe").contains("lv_b"));
    assert!(line("magnet").contains("failed") && line("magnet").contains("lv_a"));
    assert!(line("cryo").contains("running") && line("cryo").contains("BUSY t1"));
    assert!(line("cryo").contains("5.5"), "msg/s");
    let colored = terminal(&app, 100, 20, theme());
    assert!(
        colors_of(&colored, "BUSY t1")
            .iter()
            .all(|c| *c == Color::Blue)
    );
    assert!(
        colors_of(&colored, "IDLE s")
            .iter()
            .all(|c| *c == Color::Green)
    );
}

#[test]
fn detail_busy_module_error_log_file_clients() {
    let mut app = app();
    app.update(Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    assert_eq!(app.view, View::Detail);
    let shown = screen(&app, 100, 30);
    assert!(shown.contains("t1  BUSY ramping"), "{shown}");
    assert!(
        shown.contains("! HardwareError: sensor disconnected  3s"),
        "{shown}"
    );
    assert!(shown.contains("300 K  10s"), "{shown}");
    assert!(shown.contains("/home/u/.local/state/must/logs/cryo/2026-10-27.log"));
    assert!(shown.contains("192.168.1.20:51234") && shown.contains("2m ago"));
    assert!(!shown.contains("stop"), "commands are not listed");
    insta::assert_snapshot!("detail_cryo", shown);

    // colors, and none with NO_COLOR, with the text label in both
    let colored = terminal(&app, 100, 30, theme());
    assert!(
        colors_of(&colored, "BUSY ramping")
            .iter()
            .all(|c| *c == Color::Blue)
    );
    assert!(
        colors_of(&colored, "! HardwareError")
            .iter()
            .all(|c| *c == Color::Red)
    );
    let plain = terminal(&app, 100, 30, no_color());
    for cell in &plain.backend().buffer().content {
        assert_eq!(cell.fg, Color::Reset, "no colors with NO_COLOR");
    }
    assert!(plain.backend().to_string().contains("BUSY ramping"));
}

#[test]
fn detail_failed_node_and_no_log_file() {
    let mut app = app();
    press(&mut app, KeyCode::Down); // magnet
    press(&mut app, KeyCode::Enter);
    let shown = screen(&app, 100, 20);
    assert!(shown.contains("magnet  failed"));
    assert!(
        shown.contains("error   magnet:m1:value: no datainfo given"),
        "{shown}"
    );
    assert!(shown.contains("no log file (file logging off or stopped)"));
    insta::assert_snapshot!("detail_magnet", shown);

    // the log file follows the node's log_file events
    let mut app = app_with_cryo_detail();
    app.update(Msg::Endpoint {
        key: LABVIEW,
        msg: EndpointMsg::Message(ServerMessage::Event(Event::LogFile {
            node: 1,
            path: Some("/home/u/.local/state/must/logs/cryo/2026-10-28-4242.log".into()),
        })),
    });
    assert!(screen(&app, 100, 20).contains("cryo/2026-10-28-4242.log"));
    app.update(Msg::Endpoint {
        key: LABVIEW,
        msg: EndpointMsg::Message(ServerMessage::Event(Event::LogFile {
            node: 1,
            path: None,
        })),
    });
    assert!(screen(&app, 100, 20).contains("no log file"));
}

fn app_with_cryo_detail() -> App {
    let mut app = app();
    press(&mut app, KeyCode::Char('2'));
    app
}

#[test]
fn long_log_file_paths_are_shortened_in_the_middle() {
    let mut app = app_with_cryo_detail();
    let path = format!("/very/{}/cryo/2026-10-27.log", "deep/".repeat(30));
    app.update(Msg::Endpoint {
        key: LABVIEW,
        msg: EndpointMsg::Message(ServerMessage::Event(Event::LogFile {
            node: 1,
            path: Some(path),
        })),
    });
    let shown = screen(&app, 80, 24);
    let line = shown.lines().find(|l| l.contains("log  ")).unwrap();
    assert!(line.contains("/very/") && line.contains('…') && line.contains("2026-10-27.log"));
}

#[test]
fn live_traffic_pause_and_filter() {
    let mut app = app();
    let effects = press(&mut app, KeyCode::Char('3'));
    assert_eq!(
        effects,
        vec![must_tui::app::Effect::Traffic {
            key: LABVIEW,
            node: 1,
            enabled: true
        }],
        "subscribed while the view is open"
    );
    app.update(message(1, Direction::Received, NOW, "read t1:value"));
    app.update(message(
        1,
        Direction::Sent,
        NOW + 0.002,
        "reply t1:value [4.2,{\"t\":1.0}]",
    ));
    let shown = screen(&app, 80, 12);
    assert!(shown.contains("> [3] read t1:value"), "{shown}");
    assert!(shown.contains("< [3] reply t1:value"), "{shown}");
    insta::assert_snapshot!("traffic", shown);

    // pause: new messages are kept but not shown, then shown on resume
    press(&mut app, KeyCode::Char('p'));
    app.update(message(1, Direction::Received, NOW + 1.0, "read t1:target"));
    app.update(message(
        1,
        Direction::Sent,
        NOW + 1.1,
        "reply t1:target [300]",
    ));
    assert_eq!(app.visible_traffic().len(), 2);
    let shown = screen(&app, 80, 12);
    assert!(
        shown.contains("PAUSED, 2 new") && !shown.contains("t1:target"),
        "{shown}"
    );
    press(&mut app, KeyCode::Char('p'));
    assert_eq!(app.visible_traffic().len(), 4);

    // filter
    press(&mut app, KeyCode::Char('/'));
    for c in "target".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.visible_traffic().len(), 2);

    // shortened messages are marked
    app.update(Msg::Endpoint {
        key: LABVIEW,
        msg: EndpointMsg::Message(ServerMessage::Event(Event::Message {
            node: 1,
            client: 3,
            direction: Direction::Sent,
            time: NOW + 2.0,
            text: "describing . {\"target\"".into(),
            truncated: true,
        })),
    });
    assert!(screen(&app, 100, 12).contains("…[cut]"));

    // leaving the view unsubscribes
    let effects = press(&mut app, KeyCode::Esc);
    assert_eq!(
        effects,
        vec![must_tui::app::Effect::Traffic {
            key: LABVIEW,
            node: 1,
            enabled: false
        }]
    );
}

#[test]
fn logs_filter_errors_of_one_node() {
    let mut app = app(); // cryo selected
    press(&mut app, KeyCode::Char('4'));
    assert_eq!(app.visible_logs().len(), 5, "all processes, merged");
    let times: Vec<f64> = app.visible_logs().iter().map(|l| l.record.time).collect();
    assert!(times.is_sorted(), "by time");
    for _ in 0..3 {
        press(&mut app, KeyCode::Char('l'));
    }
    press(&mut app, KeyCode::Char('n'));
    let shown: Vec<_> = app
        .visible_logs()
        .iter()
        .map(|l| l.record.message.clone())
        .collect();
    assert_eq!(shown, ["sensor disconnected"]);
    let screen = screen(&app, 100, 10);
    assert!(screen.contains("level ERROR | node cryo"), "{screen}");
    assert!(
        screen.contains("ERROR cryo t1: sensor disconnected"),
        "{screen}"
    );
    insta::assert_snapshot!("logs_errors_of_cryo", screen);
    // the text search
    press(&mut app, KeyCode::Char('n'));
    press(&mut app, KeyCode::Char('/'));
    for c in "python".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.visible_logs().len(), 1);
}

#[test]
fn version_error_and_unreachable_processes() {
    let mut app = App::new(NOW, None);
    for (key, msg) in [
        (
            (1, 1),
            EndpointMsg::Connecting {
                name: "old.exe".into(),
            },
        ),
        (
            (1, 1),
            EndpointMsg::Refused("protocol 2.0 not supported, endpoint speaks 1.1".into()),
        ),
        (
            (2, 2),
            EndpointMsg::Connecting {
                name: "stuck".into(),
            },
        ),
        ((2, 2), EndpointMsg::Failed("no answer within 5 s".into())),
    ] {
        app.update(Msg::Endpoint { key, msg });
    }
    app.update(Msg::Endpoint {
        key: (1, 1),
        msg: EndpointMsg::Message(ServerMessage::Lagged { dropped: 0 }),
    });
    let shown = screen(&app, 100, 10);
    assert!(
        shown.contains("old.exe (1)  version error: protocol 2.0 not supported"),
        "{shown}"
    );
    assert!(
        shown.contains("stuck (2)  not reachable: no answer within 5 s"),
        "{shown}"
    );
    let mut app = self::app();
    app.update(Msg::Endpoint {
        key: LABVIEW,
        msg: EndpointMsg::Message(ServerMessage::Lagged { dropped: 17 }),
    });
    assert!(screen(&app, 100, 10).contains("lagged: 17 events dropped"));
}

#[test]
fn help_lists_all_bindings() {
    let mut app = app();
    press(&mut app, KeyCode::Char('?'));
    let shown = screen(&app, 80, 24);
    for (keys, what) in BINDINGS {
        assert!(
            shown.contains(keys) && shown.contains(what),
            "{keys} in\n{shown}"
        );
    }
    insta::assert_snapshot!("help_80x24", shown);
    // nothing in it writes: the only actions are views, scrolling, filters, pin, quit
    for (_, what) in BINDINGS {
        for verb in ["change", "write", "set ", "send", "stop the", "do "] {
            assert!(!what.contains(verb), "{what}");
        }
    }
    press(&mut app, KeyCode::Char('x'));
    assert!(!app.help, "any key closes it");
}

#[test]
fn resize_redraws_cleanly() {
    let mut app = app();
    for view in ['1', '2', '3', '4'] {
        press(&mut app, KeyCode::Char(view));
        let mut terminal = terminal(&app, 120, 40, theme());
        terminal.backend_mut().resize(80, 24);
        terminal.autoresize().unwrap();
        terminal.draw(|f| ui::draw(f, &app, theme())).unwrap();
        let fresh = terminal_string(&app, 80, 24);
        assert_eq!(
            terminal.backend().to_string(),
            fresh,
            "view {view} after resize"
        );
    }
}

fn terminal_string(app: &App, width: u16, height: u16) -> String {
    terminal(app, width, height, theme()).backend().to_string()
}

#[test]
fn ascii_borders_for_the_classic_windows_console() {
    let ascii = Theme {
        unicode: false,
        ..theme()
    };
    let shown = terminal(&app(), 80, 24, ascii).backend().to_string();
    assert!(shown.is_ascii() || !shown.contains('┌'), "{shown}");
    assert!(!shown.contains('┌') && !shown.contains('↑'));
}
