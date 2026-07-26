use crossterm::event::{Event, KeyCode, KeyEventKind};

use super::app::{App, Screen};

/// Applies one already-read crossterm `Event` to `App`.
///
/// Reading the event is `mod.rs`'s job now (`EventStream` inside
/// `tokio::select!`) — this stays a plain synchronous mutator so it's the
/// same shape regardless of where the event came from.
pub fn handle_event(app: &mut App, event: Event) {
    let Event::Key(key) = event else { return };

    // On Windows, key events fire on both press and release — filter to
    // press only so navigation doesn't double-fire.
    if key.kind != KeyEventKind::Press {
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Esc => match app.screen {
            Screen::Help => app.screen = Screen::List,
            Screen::List => app.should_quit = true,
        },
        KeyCode::Char('h') if app.screen == Screen::Help => app.screen = Screen::List,
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Down | KeyCode::Char('j') => app.next(),
        KeyCode::Up | KeyCode::Char('k') => app.previous(),
        KeyCode::Enter => {
            // Stub for Phase 2: open a detail view for the selected VTuber.
        }
        _ => {}
    }
}
