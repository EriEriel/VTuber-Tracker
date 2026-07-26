// Ratatui twin of `theme.rs`, for the same reason described there: that
// module returns `ColoredString`, whose raw ANSI escape bytes render as
// literal text inside a ratatui buffer instead of colour. `ratatui::style`
// is the TUI's equivalent vocabulary.
//
// Convention: the same concept keeps the same colour in both files, and
// both get updated together. Only concepts Phase 1 actually renders live
// here — `theme.rs`'s `status_tag`/`live_status`/`url` land in the phases
// that first put a stream, a live badge, or a URL on screen (3, 3, 2).

use ratatui::style::{Color, Modifier, Style};

/// A VTuber's display name. Mirrors `theme::name`'s bright cyan bold.
pub fn name() -> Style {
    Style::default()
        .fg(Color::LightCyan)
        .add_modifier(Modifier::BOLD)
}

/// Secondary detail: platform tags, hints. Mirrors `theme::muted`'s intent
/// (de-emphasised secondary text) with an explicit colour rather than a
/// `DIM` modifier — terminal support for `DIM` is inconsistent, and
/// `DarkGray` is what Phase 0 already shipped and was verified to render.
pub fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}
