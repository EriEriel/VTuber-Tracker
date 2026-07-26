mod app;
mod event;
mod theme;
mod ui;

use app::{App, VtuberRow};
use crossterm::{
    cursor::Show,
    event::EventStream,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use event::Command;
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};
use tokio::sync::mpsc;

use crate::routes::{ApiError, VtuberDetail};

/// What can arrive asynchronously and needs to update `App`. Grows with each
/// later phase (an action's outcome, an auto-refresh tick) — Phase 2 adds
/// the detail fetch and the one action (`o`) that can fail in a way with no
/// screen of its own to show it.
enum Message {
    Vtubers(Result<Vec<VtuberRow>, ApiError>),
    Detail {
        id: String,
        result: Result<VtuberDetail, ApiError>,
    },
    ActionFailed(String),
}

/// Entry point called from clap's `Tui` subcommand handler in main.rs.
pub async fn run() -> anyhow::Result<()> {
    install_panic_hook();

    let mut terminal = setup_terminal()?;
    let mut app = App::new();

    let (tx, rx) = mpsc::channel(8);
    // Dispatched as a background task rather than awaited here: with a
    // blocking pre-loop fetch, `LoadState::Loading` was set but never
    // actually rendered, since nothing drew a frame until the fetch had
    // already resolved. Spawning it means the first frame(s) can genuinely
    // show "Loading..." while it's in flight.
    spawn_fetch_vtubers(tx.clone());

    let result = run_loop(&mut terminal, &mut app, tx, rx).await;

    restore_terminal(&mut terminal)?;

    result
}

fn spawn_fetch_vtubers(tx: mpsc::Sender<Message>) {
    tokio::spawn(async move {
        let _ = tx.send(Message::Vtubers(fetch_tracked_vtubers().await)).await;
    });
}

/// The render loop. This is the "immediate mode" core: draw, wait for
/// something to react to, mutate state, repeat. Ratatui doesn't track what
/// changed between frames — it just re-describes the whole UI every tick.
/// That's *why* `draw` in ui.rs is a pure function of `&App`: there's no
/// incremental update to reason about, which is what makes this model easy
/// to keep correct.
///
/// `select!` over crossterm's async `EventStream` and the message channel,
/// rather than the old blocking `event::poll`: a blocking read stalls the
/// whole tokio runtime, so nothing spawned (like the background fetches this
/// dispatches) could ever make progress while waiting on a keypress.
///
/// `tx` is threaded through for the whole loop, not just the startup fetch —
/// `Enter`/`o` need to spawn their own background work as they happen.
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    tx: mpsc::Sender<Message>,
    mut rx: mpsc::Receiver<Message>,
) -> anyhow::Result<()> {
    let mut events = EventStream::new();

    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;

        tokio::select! {
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if let Some(cmd) = event::handle_event(app, event) {
                            dispatch(cmd, tx.clone());
                        }
                    }
                    Some(Err(err)) => return Err(err.into()),
                    // The stream ended (stdin closed) — nothing left to wait
                    // on; let the next loop check re-evaluate should_quit.
                    None => {}
                }
            }
            Some(message) = rx.recv() => handle_message(app, message),
        }
    }
    Ok(())
}

/// Turns a `Command` `handle_event` decided on into an actual background
/// task. Split from `run_loop` because it's the one place allowed to know
/// how each `Command` maps onto a `routes` call — `event::handle_event`
/// deliberately doesn't import `routes` at all.
fn dispatch(cmd: Command, tx: mpsc::Sender<Message>) {
    match cmd {
        Command::FetchDetail(id) => {
            tokio::spawn(async move {
                let result = crate::routes::fetch_vtuber_detail(&id).await;
                let _ = tx.send(Message::Detail { id, result }).await;
            });
        }
        Command::OpenProfile(id) => {
            tokio::spawn(async move {
                match crate::routes::fetch_profile_url(&id).await {
                    Ok(url) => open_and_report(url, &tx).await,
                    Err(err) => {
                        let _ = tx.send(Message::ActionFailed(err.to_string())).await;
                    }
                }
            });
        }
        Command::OpenUrl(url) => {
            tokio::spawn(async move {
                open_and_report(url, &tx).await;
            });
        }
    }
}

async fn open_and_report(url: String, tx: &mpsc::Sender<Message>) {
    if let Err(err) = open::that(url) {
        let _ = tx
            .send(Message::ActionFailed(format!("could not open browser: {err}")))
            .await;
    }
}

fn handle_message(app: &mut App, message: Message) {
    match message {
        Message::Vtubers(Ok(rows)) => app.set_items(rows),
        Message::Vtubers(Err(e)) => app.set_error(e.to_string()),
        Message::Detail { id, result } => app.accept_detail(&id, result),
        Message::ActionFailed(msg) => app.last_error = Some(msg),
    }
}

/// Enter raw mode + alternate screen. Raw mode hands every keypress
/// unprocessed (no line buffering, no Ctrl+C signal) — necessary for j/k
/// nav to feel instant. Alternate screen means we're drawing on a fresh
/// buffer the terminal swaps back out of on exit, so the user's shell
/// history isn't clobbered with frames of list-scrolling.
fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

/// CRITICAL: this must run even on error/panic, or the user's shell is
/// left in raw mode (no line wrap, no echo — looks "broken" until they
/// run `reset`). `run()` covers the Ok/Err paths by always calling this
/// after run_loop returns; `install_panic_hook` covers the unwind path.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    restore_stdout()?;
    // Also tell the `Terminal` the cursor is visible again. `restore_stdout`
    // already emitted the escape, but ratatui tracks the state internally and
    // a stale flag would confuse a future re-entry into the TUI.
    terminal.show_cursor()
}

/// The teardown escape sequences, written to a bare stdout handle instead of
/// through the `Terminal`. That indirection is what makes the panic hook
/// possible: `run_loop` holds `&mut Terminal` for the whole session, so a
/// hook could never borrow it — but these are stateless terminal commands,
/// so a fresh handle to the same fd does exactly the same job.
fn restore_stdout() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, Show)
}

/// Restore the terminal *before* the default hook prints the panic message.
///
/// Order is the whole point. Panic output goes to stderr, which is still
/// pointed at the alternate screen — so a hook that printed first would paint
/// the message onto a buffer the terminal is about to discard, and the user
/// would see a silent exit with no explanation. Leaving the alternate screen
/// first means the message lands on the normal screen where it survives.
///
/// Errors are swallowed: we're already unwinding, and a failed `write!` must
/// not panic inside a panic (that aborts the process, losing the message
/// entirely — the exact outcome this is here to prevent).
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_stdout();
        default_hook(info);
    }));
}

/// The mapper-at-the-boundary point: `routes` owns all the reqwest/serde
/// work and hands back the raw API DTO; `VtuberRow::from` narrows it to what
/// the render layer needs, so nothing in `ui.rs` ever sees a `VtuberChannel`.
async fn fetch_tracked_vtubers() -> Result<Vec<VtuberRow>, ApiError> {
    let channels = crate::routes::fetch_vtubers().await?;

    Ok(channels
        .iter()
        .filter(|c| c.is_tracked)
        .map(VtuberRow::from)
        .collect())
}
