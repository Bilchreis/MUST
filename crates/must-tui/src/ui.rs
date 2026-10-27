// SPDX-License-Identifier: LGPL-3.0-or-later

//! Drawing the state; a pure function of [`App`].

use ratatui::Frame;
use ratatui::widgets::{Block, Paragraph};

use crate::app::App;

/// Draws the whole screen.
pub fn draw(frame: &mut Frame, app: &App) {
    let lines: Vec<String> = app
        .processes
        .values()
        .flat_map(|p| {
            std::iter::once(format!("{} ({})", p.name, p.pid)).chain(
                p.nodes
                    .values()
                    .map(|n| format!("  {} {:?}", n.info.equipment_id, n.info.state)),
            )
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines.join("\n")).block(Block::bordered().title(" must-tui ")),
        frame.area(),
    );
}
