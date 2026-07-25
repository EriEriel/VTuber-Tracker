// Shared colouring for CLI output.
//
// Centralised so the same concept reads the same way everywhere — a VTuber
// who is live and a stream tagged `[live]` use the same green, rather than
// each call site picking its own.
//
// Colour is applied unconditionally here: the `colored` crate decides at
// runtime whether the escape codes are actually emitted, honouring NO_COLOR,
// CLICOLOR/CLICOLOR_FORCE, and whether stdout is a terminal. Piping to a
// file or grep produces clean text with no manual checks at any call site.

use colored::{ColoredString, Colorize};

/// A stream's status tag, e.g. `[live]`.
pub fn status_tag(status: &str) -> ColoredString {
    let tag = format!("[{status}]");
    match status {
        "live" => tag.as_str().bright_green().bold(),
        "upcoming" => tag.as_str().yellow(),
        "ended" => tag.as_str().dimmed(),
        // 'unknown' is a real value in the Stream schema, and unrecognised
        // values shouldn't be swallowed silently.
        _ => tag.as_str().normal(),
    }
}

/// A VTuber's overall live state. Deliberately shares its green with
/// `status_tag("live")`.
pub fn live_status(is_live: bool) -> ColoredString {
    if is_live {
        "LIVE".bright_green().bold()
    } else {
        "offline".dimmed()
    }
}

/// A VTuber's display name.
pub fn name(name: &str) -> ColoredString {
    name.bright_cyan().bold()
}

/// Section heading such as `Recent streams:`.
pub fn heading(text: &str) -> ColoredString {
    text.bold()
}

/// A URL — de-emphasised, since it's for copying rather than reading.
pub fn url(text: &str) -> ColoredString {
    text.blue().dimmed()
}

/// Secondary detail: native names, channel IDs, view counts.
pub fn muted(text: &str) -> ColoredString {
    text.dimmed()
}
