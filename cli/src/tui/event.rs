use crossterm::event::{Event, KeyCode, KeyEventKind};

use super::app::{App, EditField, EditPayload, ModalKind, Screen};
use crate::models::Source;

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
    Sync { id: String, source: Source, name: String },
    Delete { id: String, name: String },
    /// Already validated by `App::try_submit_create` — `routes` still
    /// re-parses it (it's the only thing that knows how to build the
    /// request body), but a guaranteed-bad URL never reaches here.
    Create(String),
    /// Already validated by `App::try_submit_edit` — plain data, not a
    /// `routes::UpdateFields`, so this module stays routes-agnostic; `mod.rs`
    /// does the conversion when it actually dispatches.
    Update(EditPayload),
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

    // Filter-edit mode swallows the keymap entirely: `q`, `L`, `j`/`k` and
    // every other letter are all valid *search text*, so this has to
    // short-circuit before the normal match rather than try to guard each
    // arm individually.
    if app.filter_editing {
        handle_filter_edit(app, key.code);
        return None;
    }

    // Same reasoning as filter-edit mode: a modal owns every keypress while
    // it's open (the create-URL modal is a text field; even the confirm
    // modal shouldn't let `q` slip through and quit under a stray keypress).
    if let Screen::Modal(kind) = app.screen {
        return handle_modal(app, kind, key.code);
    }

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Esc => match app.screen {
            // `Screen::Modal(_)` never actually reaches this arm — the
            // short-circuit above handles it — but `Screen` still has to be
            // matched exhaustively, and this is the sane fallback if that
            // invariant ever changes.
            Screen::Help | Screen::Detail | Screen::Modal(_) => app.go_back_to_list(),
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
        KeyCode::Char('/') if app.screen == Screen::List => app.start_filter_edit(),
        KeyCode::Char('s') if app.screen == Screen::List => {
            let row = app.selected()?;
            let (id, source, name) = (row.id.clone(), row.source, row.name.clone());
            app.begin_action();
            return Some(Command::Sync { id, source, name });
        }
        KeyCode::Char('d') if app.screen == Screen::List => {
            app.selected()?;
            app.open_confirm_delete();
        }
        KeyCode::Char('a') if app.screen == Screen::List => app.open_create_url(),
        KeyCode::Char('e') if app.screen == Screen::List => app.open_edit_form(),
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
        //
        // No `begin_action()` here, unlike `s`/`d`/`a`: opening a browser
        // isn't a mutation with a meaningful in-flight state to spin on, and
        // the success path sends no message at all (see `open_and_report`),
        // so a `pending` this incremented would never get decremented back.
        KeyCode::Char('o') if app.screen == Screen::List => {
            let id = app.selected().map(|row| row.id.clone())?;
            return Some(Command::OpenProfile(id));
        }
        KeyCode::Char('o') if app.screen == Screen::Detail => {
            let url = app.focused_url()?;
            return Some(Command::OpenUrl(url));
        }
        _ => {}
    }

    None
}

/// Arrow keys (not `j`/`k` — those are letters someone might be typing)
/// still move the selection live, matching how most fuzzy-finders behave
/// while their input has focus.
fn handle_filter_edit(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char(c) => app.filter_push(c),
        KeyCode::Backspace => app.filter_backspace(),
        KeyCode::Enter => app.commit_filter(),
        KeyCode::Esc => app.clear_filter(),
        KeyCode::Down => app.next(),
        KeyCode::Up => app.previous(),
        _ => {}
    }
}

fn handle_modal(app: &mut App, kind: ModalKind, code: KeyCode) -> Option<Command> {
    match kind {
        // Re-derives from `app.selected()` at confirm time rather than
        // snapshotting when the modal opened: since modal input is
        // exclusive, selection can't drift while it's open, and this way
        // the confirmation text and the delete it fires are guaranteed to
        // agree on which VTuber, by construction.
        ModalKind::ConfirmDelete => match code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let row = app.selected()?;
                let (id, name) = (row.id.clone(), row.name.clone());
                app.go_back_to_list();
                app.begin_action();
                Some(Command::Delete { id, name })
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                app.go_back_to_list();
                None
            }
            _ => None,
        },
        ModalKind::CreateUrl => match code {
            KeyCode::Char(c) => {
                app.create_input.push(c);
                app.create_error = None;
                None
            }
            KeyCode::Backspace => {
                app.create_input.pop();
                None
            }
            KeyCode::Enter => app.try_submit_create().map(Command::Create),
            KeyCode::Esc => {
                app.create_input.clear();
                app.create_error = None;
                app.go_back_to_list();
                None
            }
            _ => None,
        },
        ModalKind::Edit => handle_edit_form(app, code),
    }
}

/// `Tab`/`Down` and `BackTab`/`Up` both cycle fields — the second pair
/// matches how a traditional form usually behaves, on top of the
/// `Tab`-cycles-panes convention Detail already established. `Space` is
/// scoped to only the `IsTracked` field; everywhere else it has to insert a
/// literal space (`edit_push` handles that already), since org/suborg names
/// routinely contain one.
fn handle_edit_form(app: &mut App, code: KeyCode) -> Option<Command> {
    let focus = app.edit.as_ref().map(|f| f.focus);

    match code {
        KeyCode::Tab | KeyCode::Down => {
            app.edit_next_field();
            None
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.edit_previous_field();
            None
        }
        KeyCode::Char(' ') if focus == Some(EditField::IsTracked) => {
            app.edit_toggle_tracked();
            None
        }
        KeyCode::Char(c) => {
            app.edit_push(c);
            None
        }
        KeyCode::Backspace => {
            app.edit_backspace();
            None
        }
        KeyCode::Enter => app.try_submit_edit().map(Command::Update),
        KeyCode::Esc => {
            app.edit = None;
            app.go_back_to_list();
            None
        }
        _ => None,
    }
}
