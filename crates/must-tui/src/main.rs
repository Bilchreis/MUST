// SPDX-License-Identifier: LGPL-3.0-or-later

//! `must-tui`: shows all must SEC nodes running for the user. See the library crate.

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use must_monitor::client::Options;
use must_monitor::registry;
use must_tui::app::{App, Msg};
use must_tui::{discovery, terminal, ui};
use ratatui::crossterm::event::{self, Event};
use tokio::sync::mpsc;

const USAGE: &str = "\
usage: must-tui [--focus-pid <pid>]

Shows all must SEC nodes running for this user. Press ? for help, q to quit.

  --focus-pid <pid>  select the first node of this process once it appears
  --version          print the version
  --help             print this text";

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn main() -> ExitCode {
    let mut focus_pid = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--focus-pid" => match args.next().and_then(|p| p.parse().ok()) {
                Some(pid) => focus_pid = Some(pid),
                None => return usage_error("--focus-pid needs a process id"),
            },
            "--version" => {
                println!("must-tui {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => return usage_error(&format!("unknown argument {other:?}")),
        }
    }

    let dir = match registry::registry_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("must-tui: no registry directory: {e}");
            return ExitCode::FAILURE;
        }
    };
    // one TUI per user; the lock goes with the process
    let _lock = match registry::try_lock_tui(&dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            eprintln!("must-tui is already running for this user.");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!(
                "must-tui: cannot take {}: {e}",
                dir.join(registry::TUI_LOCK).display()
            );
            return ExitCode::FAILURE;
        }
    };

    // a TUI needs a terminal to draw in and read keys from (not a pipe or a file)
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        eprintln!("must-tui needs a terminal: run it in a terminal window.");
        return ExitCode::FAILURE;
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("must-tui: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut tui = match terminal::init() {
        Ok(tui) => tui,
        Err(e) => {
            eprintln!("must-tui: cannot set up the terminal: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run(&mut tui, dir, focus_pid));
    let _ = terminal::restore();
    runtime.shutdown_timeout(Duration::from_millis(200));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("must-tui: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("must-tui: {message}\n\n{USAGE}");
    ExitCode::from(2)
}

async fn run(
    tui: &mut terminal::Tui,
    dir: std::path::PathBuf,
    focus_pid: Option<u32>,
) -> std::io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (effects_tx, effects_rx) = mpsc::unbounded_channel();
    tokio::spawn(discovery::run(
        dir,
        Options::new("must-tui"),
        tx.clone(),
        effects_rx,
    ));
    // keys and resizes, read on a thread of their own
    let input_tx = tx.clone();
    std::thread::Builder::new()
        .name("must-tui-input".into())
        .spawn(move || {
            loop {
                let msg = match event::read() {
                    Ok(Event::Key(key)) => Msg::Key(key),
                    Ok(Event::Resize(..)) => Msg::Resize,
                    Ok(_) => continue,
                    Err(_) => break,
                };
                if input_tx.send(msg).is_err() {
                    break;
                }
            }
        })?;
    let ticker_tx = tx;
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_millis(250));
        loop {
            ticks.tick().await;
            if ticker_tx.send(Msg::Tick { now: now() }).is_err() {
                break;
            }
        }
    });

    let mut app = App::new(now(), focus_pid);
    let theme = ui::Theme::from_env();
    tui.draw(|frame| ui::draw(frame, &app, theme))?;
    while let Some(msg) = rx.recv().await {
        for effect in app.update(msg) {
            let _ = effects_tx.send(effect);
        }
        // apply whatever else is queued before drawing once
        while let Ok(msg) = rx.try_recv() {
            for effect in app.update(msg) {
                let _ = effects_tx.send(effect);
            }
        }
        if app.quit {
            break;
        }
        tui.draw(|frame| ui::draw(frame, &app, theme))?;
    }
    Ok(())
}
