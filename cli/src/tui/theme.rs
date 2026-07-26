// Ratatui twin of `theme.rs`, for the same reason described there: that
// module returns `ColoredString`, whose raw ANSI escape bytes render as
// literal text inside a ratatui buffer instead of colour. `ratatui::style`
// is the TUI's equivalent vocabulary.
//
// Convention: the same concept keeps the same colour in both files, and
// both get updated together. Only concepts with a real call site live
// here — a binary crate's dead-code lint fires on unreachable `pub` items
// the same as private ones, so each function is added in the phase that
// first puts it on screen.

use ratatui::style::{Color, Modifier, Style};

/// A VTuber's display name. Mirrors `theme::name`'s bright cyan bold.
pub fn name() -> Style {
    Style::default()
        .fg(Color::LightCyan)
        .add_modifier(Modifier::BOLD)
}

/// Secondary detail: platform tags, hints, field labels. Mirrors
/// `theme::muted`'s intent (de-emphasised secondary text) with an explicit
/// colour rather than a `DIM` modifier — terminal support for `DIM` is
/// inconsistent, and `DarkGray` is what Phase 0 already shipped and was
/// verified to render.
pub fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// A URL — de-emphasised, since it's for copying rather than reading.
/// Mirrors `theme::url`'s dimmed blue.
pub fn url() -> Style {
    Style::default().fg(Color::Blue).add_modifier(Modifier::DIM)
}

/// A stream's status tag, e.g. `[live]`. Mirrors `theme::status_tag`.
pub fn status_tag(status: &str) -> Style {
    match status {
        "live" => Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD),
        "upcoming" => Style::default().fg(Color::Yellow),
        "ended" => muted(),
        // 'unknown' is a real value in the Stream schema, same as theme.rs.
        _ => Style::default(),
    }
}

/// A VTuber's overall live state, e.g. the detail screen's "Status:" line.
/// Deliberately shares its green with `status_tag("live")`, mirroring
/// `theme::live_status`.
pub fn live_status(is_live: bool) -> Style {
    if is_live {
        Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD)
    } else {
        muted()
    }
}

/// The list's `LIVE` badge for a VTuber Phase 7's ticker just flagged as
/// `WentLive`/`BurstWentLive` this poll, not merely still-live from before.
/// Same green as `live_status`, plus `REVERSED` to stand out from every
/// other already-live row without needing a second colour.
pub fn just_went_live() -> Style {
    Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
}
