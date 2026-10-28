// SPDX-License-Identifier: LGPL-3.0-or-later

//! Putting the terminal into TUI mode and back, also after a panic.

use std::io::{self, Stdout};
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// The terminal in TUI mode.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Enters raw mode and the alternate screen, and makes panics restore the terminal
/// before their message is printed.
pub fn init() -> io::Result<Tui> {
    install_panic_hook(Arc::new(|| {
        let _ = restore();
    }));
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(e);
    }
    Terminal::new(CrosstermBackend::new(stdout))
}

/// Leaves raw mode and the alternate screen and shows the cursor.
pub fn restore() -> io::Result<()> {
    let raw = disable_raw_mode();
    let screen = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        ratatui::crossterm::cursor::Show
    );
    raw.and(screen)
}

/// Runs `restore` before the previous panic hook, so the panic message lands on a
/// normal terminal.
pub fn install_panic_hook(restore: Arc<dyn Fn() + Send + Sync>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn panic_hook_restores_the_terminal_first() {
        static RESTORED: AtomicBool = AtomicBool::new(false);
        install_panic_hook(Arc::new(|| RESTORED.store(true, Ordering::SeqCst)));
        let result = std::panic::catch_unwind(|| panic!("simulated"));
        let _ = std::panic::take_hook(); // back to the default hook
        assert!(result.is_err());
        assert!(RESTORED.load(Ordering::SeqCst));
    }
}
