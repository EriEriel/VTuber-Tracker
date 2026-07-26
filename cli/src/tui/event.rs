use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::time::Duration;

use super::app::App;

/// Poll for input with a short timeout so the loop doesn't block forever.
/// 100ms is plenty for a static list — no animation to keep smooth yet.
pub fn handle_events(app: &mut App) -> std::io::Result<()> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            // On Windows, key events fire on both press and release —
            // filter to press only so navigation doesn't double-fire.
            if key.kind != KeyEventKind::Press {
                return Ok(());
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                KeyCode::Down | KeyCode::Char('j') => app.next(),
                KeyCode::Up | KeyCode::Char('k') => app.previous(),
                KeyCode::Enter => {
                    // Stub for v0.2: open a detail view for the selected VTuber.
                }
                _ => {}
            }
        }
    }
    Ok(())
}
