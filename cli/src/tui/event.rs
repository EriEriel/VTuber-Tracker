use crossterm::event::{Event, KeyCode, KeyEventKind};

use super::app::{App, Screen};

/// Background work an event decided to kick off. `handle_event` only
/// *describes* the intent — `mod.rs` is the one holding the channel `Sender`
/// needed to actually `tokio::spawn` it, so it stays the thing that performs
/// it. Keeps this function a plain synchronous mutator, same as Phase 1.
pub enum Command {
    FetchDetail(String),
    /// Open a VTuber's channel: the URL isn't known yet, `routes` has to
    /// resolve it first.
    OpenProfile(String),
    /// Open an already-known URL — a focused stream/clip's, already sitting
    /// in `app.detail`, so no fetch is needed before opening it.
    OpenUrl(String),
}

/// Applies one already-read crossterm `Event` to `App`, returning a
/// `Command` if the key should also kick off background work.
///
/// Reading the event is `mod.rs`'s job now (`EventStream` inside
/// `tokio::select!`) — this stays a plain synchronous mutator so it's the
/// same shape regardless of where the event came from.
pub fn handle_event(app: &mut App, event: Event) -> Option<Command> {
    let Event::Key(key) = event else { return None };

    // On Windows, key events fire on both press and release — filter to
    // press only so navigation doesn't double-fire.
    if key.kind != KeyEventKind::Press {
        return None;
    }

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Esc => match app.screen {
            Screen::Help | Screen::Detail => app.go_back_to_list(),
            Screen::List => app.should_quit = true,
        },
        KeyCode::Char('h') if app.screen != Screen::List => app.go_back_to_list(),
        // Stacking the help overlay over the detail screen isn't something
        // any phase has asked for yet — Esc/h from Help always lands back on
        // List, so opening it over Detail would lose the detail context.
        KeyCode::Char('?') if app.screen != Screen::Detail => app.toggle_help(),
        KeyCode::Down | KeyCode::Char('j') if app.screen == Screen::List => app.next(),
        KeyCode::Up | KeyCode::Char('k') if app.screen == Screen::List => app.previous(),
        KeyCode::Char('L') if app.screen == Screen::List => app.toggle_live_only(),
        KeyCode::Down | KeyCode::Char('j') if app.screen == Screen::Detail => app.detail_next(),
        KeyCode::Up | KeyCode::Char('k') if app.screen == Screen::Detail => app.detail_previous(),
        KeyCode::Tab if app.screen == Screen::Detail => app.toggle_detail_focus(),
        KeyCode::Enter if app.screen == Screen::List => {
            let id = app.selected().map(|row| row.id.clone())?;
            app.begin_detail(id.clone());
            return Some(Command::FetchDetail(id));
        }
        // Contextual: on List, `o` opens the VTuber's channel (nothing else
        // to open there). On Detail, it opens whichever stream/clip is
        // currently focused instead — to open the channel from Detail, back
        // out to List first and press `o` there.
        KeyCode::Char('o') if app.screen == Screen::List => {
            let id = app.selected().map(|row| row.id.clone())?;
            app.last_error = None;
            return Some(Command::OpenProfile(id));
        }
        KeyCode::Char('o') if app.screen == Screen::Detail => {
            let url = app.focused_url()?;
            app.last_error = None;
            return Some(Command::OpenUrl(url));
        }
        _ => {}
    }

    None
}
