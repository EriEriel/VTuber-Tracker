mod app;
mod event;
mod ui;

use app::{App, VtuberRow};
use crossterm::{
    cursor::Show,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

/// Entry point called from clap's `Tui` subcommand handler in main.rs.
pub async fn run() -> anyhow::Result<()> {
    install_panic_hook();

    let mut terminal = setup_terminal()?;

    let mut app = App::new();

    // v0.1: one blocking fetch before the loop starts. No spinner logic,
    // no background task — just show "Loading..." for one frame (or zero,
    // if API is fast), then populate. Refresh-on-keypress is a v0.2
    // problem once this feels solid.
    match fetch_tracked_vtubers().await {
        Ok(rows) => app.set_items(rows),
        Err(e) => app.set_error(e.to_string()),
    }

    let result = run_loop(&mut terminal, &mut app);

    restore_terminal(&mut terminal)?;

    result
}

/// The render loop. This is the "immediate mode" core: draw, poll input,
/// mutate state, repeat. Ratatui doesn't track what changed between frames —
/// it just re-describes the whole UI every tick. That's *why* `draw` in
/// ui.rs is a pure function of `&App`: there's no incremental update to
/// reason about, which is what makes this model easy to keep correct.
///
/// Deliberately sync, even though `run()` is async: `event::poll` blocks the
/// thread, so nothing else on the runtime progresses while we're in here.
/// Fine while the only fetch happens before the loop; the moment a
/// background refresh exists, this has to become a `select!` over a channel
/// and crossterm's async `EventStream` instead.
fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        event::handle_events(app)?;
    }
    Ok(())
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
///
/// `map_err` rather than a bare `?`: `routes` returns `Box<dyn Error>`, which
/// is not itself an `Error` and isn't `Send + Sync`, so anyhow's blanket
/// `From` impl doesn't apply. Flattening to a message is fine here — the TUI
/// only ever renders it as a string.
async fn fetch_tracked_vtubers() -> anyhow::Result<Vec<VtuberRow>> {
    let channels = crate::routes::fetch_vtubers()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(channels
        .iter()
        .filter(|c| c.is_tracked)
        .map(VtuberRow::from)
        .collect())
}
