use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::app::{App, DetailFocus, EditField, LoadState, ModalKind, Screen};
use super::theme;
use crate::routes::VtuberDetail;

/// Pure render function: reads App, writes to the Frame, mutates nothing.
/// This is the "immediate mode" part — every tick, we throw away the last
/// frame and redraw the whole screen from current state. No diffing to
/// think about, no manual redraw logic — just describe what it should
/// look like *right now*.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // The filter bar only makes sense over the list (a modal opened from
    // List still shows it underneath) — Detail/Help keep the plain layout
    // regardless of whatever `/` last left behind.
    let show_filter_bar = matches!(app.screen, Screen::List | Screen::Modal(_))
        && (app.filter_editing || !app.filter.is_empty());

    // Activity (status/spinner) and hints are two separate fixed rows, not
    // one row that swaps between them — a CRUD action's outcome used to
    // blank out the key hints entirely while it was showing, which hid the
    // very keys you'd want next. Now the hints are always visible, and
    // activity gets its own line above them.
    let constraints = if show_filter_bar {
        vec![Constraint::Length(1), Constraint::Min(0), Constraint::Length(1), Constraint::Length(1)]
    } else {
        vec![Constraint::Min(0), Constraint::Length(1), Constraint::Length(1)]
    };
    let chunks = Layout::vertical(constraints).split(area);

    let (filter_area, content_area, activity_area, hints_area) = if show_filter_bar {
        (Some(chunks[0]), chunks[1], chunks[2], chunks[3])
    } else {
        (None, chunks[0], chunks[1], chunks[2])
    };

    if let Some(filter_area) = filter_area {
        draw_filter_bar(frame, filter_area, app);
    }

    if app.screen == Screen::Detail {
        draw_detail(frame, content_area, app);
    } else {
        match &app.load_state {
            LoadState::Loading => draw_message(frame, content_area, "Loading tracked VTubers..."),
            LoadState::Failed(msg) => draw_message(frame, content_area, &format!("Error: {msg}")),
            LoadState::Loaded if app.items.is_empty() => {
                draw_message(frame, content_area, "No VTubers tracked yet.")
            }
            LoadState::Loaded if app.visible_ids().is_empty() && !app.filter.is_empty() => {
                draw_message(frame, content_area, &format!("No VTubers match '{}'.", app.filter))
            }
            LoadState::Loaded if app.visible_ids().is_empty() && app.live_only => {
                draw_message(frame, content_area, "No one is live right now.")
            }
            LoadState::Loaded => draw_list(frame, content_area, app),
        }
    }

    draw_activity_bar(frame, activity_area, app);
    draw_hints_bar(frame, hints_area, app);

    // Overlaid last, on top of whatever's already drawn — closing it
    // returns to exactly what was underneath.
    match app.screen {
        Screen::Help => draw_help(frame, area, app),
        Screen::Modal(ModalKind::ConfirmDelete) => draw_confirm_delete(frame, area, app),
        Screen::Modal(ModalKind::CreateUrl) => draw_create_url(frame, area, app),
        Screen::Modal(ModalKind::Edit) => draw_edit_form(frame, area, app),
        _ => {}
    }
}

const SPINNER_FRAMES: [char; 4] = ['|', '/', '-', '\\'];

/// Status/spinner only — separate from the hints row below it. While
/// `pending > 0` this always wins (there's nothing meaningful in `status`
/// yet; `end_action` only sets it once the action resolves), so the two
/// never fight over the line.
fn draw_activity_bar(frame: &mut Frame, area: Rect, app: &App) {
    if app.pending > 0 {
        let glyph = SPINNER_FRAMES[app.spinner_frame as usize % SPINNER_FRAMES.len()];
        frame.render_widget(Paragraph::new(format!("{glyph} Working...")), area);
        return;
    }

    if let Some(status) = &app.status {
        let text = match status {
            Ok(msg) => msg.clone(),
            Err(msg) => format!("Error: {msg}"),
        };
        frame.render_widget(Paragraph::new(text), area);
    }
    // Nothing pending, no status — leave the row blank. Ratatui redraws
    // from a fresh buffer every frame, so not rendering here is enough to
    // clear whatever was on this line last frame.
}

/// A persistent `/query` line above the list, shown while editing (with a
/// cursor) or whenever a filter is committed and non-empty — so it's always
/// visible *why* the list is narrower than expected, not just while typing.
fn draw_filter_bar(frame: &mut Frame, area: Rect, app: &App) {
    let cursor = if app.filter_editing { "▏" } else { "" };
    let text = format!("/{}{cursor}", app.filter);
    let style = if app.filter_editing { Style::default() } else { theme::muted() };
    frame.render_widget(Paragraph::new(text).style(style), area);
}

fn draw_message(frame: &mut Frame, area: Rect, msg: &str) {
    let block = Block::default()
        .title(" oshihub ")
        .borders(Borders::ALL);
    let p = Paragraph::new(msg).block(block);
    frame.render_widget(p, area);
}

fn draw_list(frame: &mut Frame, area: Rect, app: &App) {
    let visible = app.visible_ids();
    let items: Vec<ListItem> = visible
        .iter()
        .map(|&i| {
            let v = &app.items[i];
            let mut spans = vec![
                Span::styled(v.name.clone(), theme::name()),
                Span::raw("  "),
                Span::styled(format!("[{}]", v.platform), theme::muted()),
            ];
            // Badge, not a filter — shown regardless of live_only, since
            // every row is already live when that's on anyway.
            if app.live_ids.contains(&v.id) {
                spans.push(Span::raw("  "));
                // `newly_live` only holds ids from the *most recent* poll's
                // WentLive/BurstWentLive actions — everyone else already-live
                // gets the plain badge.
                let style = if app.newly_live.contains(&v.id) {
                    theme::just_went_live()
                } else {
                    theme::live_status(true)
                };
                spans.push(Span::styled("LIVE", style));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = match (app.live_only, !app.filter.is_empty()) {
        (true, true) => format!(" Tracked VTubers — live only, filtered ({}) ", visible.len()),
        (true, false) => format!(" Tracked VTubers — live only ({}) ", visible.len()),
        (false, true) => {
            format!(" Tracked VTubers — filtered ({}/{}) ", visible.len(), app.items.len())
        }
        (false, false) => format!(" Tracked VTubers ({}) ", app.items.len()),
    };

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
        .highlight_symbol("> ");

    // render_stateful_widget needs &mut ListState, but `app` here is &App.
    // ListState is Copy, so this reads a copy through the shared borrow.
    let mut state = app.list_state;
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_hints_bar(frame: &mut Frame, area: Rect, app: &App) {
    let hints = if app.filter_editing {
        "Type to filter  ·  Enter commit  ·  Esc clear  ·  ↑/↓ move"
    } else {
        match app.screen {
            Screen::List if app.live_only => {
                "j/k move · Enter detail · o open · s sync · d delete · a add · e edit · / filter · L show all · ? help · q quit"
            }
            Screen::List => {
                "j/k move · Enter detail · o open · s sync · d delete · a add · e edit · / filter · L live only · ? help · q quit"
            }
            Screen::Detail => "j/k move  ·  Tab pane  ·  o open  ·  Esc/h back",
            Screen::Help => "Esc/h close",
            Screen::Modal(ModalKind::ConfirmDelete) => "y / Enter confirm  ·  n / Esc cancel",
            Screen::Modal(ModalKind::CreateUrl) => "Enter create  ·  Esc cancel",
            Screen::Modal(ModalKind::Edit) => "Tab/↑↓ field · Space toggle · Enter save · Esc cancel",
        }
    };
    // Carve a fixed-width slot off the right for the version rather than
    // just appending it to `hints` — appending would make truncation eat
    // into the version string first on a narrow terminal, when the key
    // hints are what actually matter there.
    let version = concat!("v", env!("CARGO_PKG_VERSION"));
    let version_width = version.chars().count() as u16;
    let chunks = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(version_width),
    ])
    .split(area);
    let (hints_area, version_area) = (chunks[0], chunks[2]);

    let p = Paragraph::new(truncate_with_ellipsis(hints, hints_area.width as usize))
        .style(theme::muted());
    frame.render_widget(p, hints_area);
    // `Style::default()`, not `theme::muted()` — every `Block` in this file
    // leaves `border_style` unset (bar the focused/unfocused stream-clip
    // pane), so `Style::default()` *is* "the border colour" everywhere else
    // on screen. Matching it, not the dim hint text next to it.
    frame.render_widget(Paragraph::new(version).style(Style::default()), version_area);
}

/// Hints run up to ~110 chars; a narrow terminal (or a split pane) leaves
/// `area.width` well short of that. Without this, `Paragraph`'s own
/// truncation still clips to the area — it can't paint past it — but it
/// does so silently mid-word, so the row just stops with no sign anything's
/// missing. Cutting here instead reserves the last cell for `…`, so it's
/// obvious the hint list continues past the edge.
///
/// Char-count truncation, not byte-count: every hint string is ASCII plus
/// single-width separators (`·`, `↑`, `↓`), so counting `chars()` already
/// matches terminal cell width — no need for the `unicode-width` crate over
/// one string with a known, narrow charset.
fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let mut truncated: String = text.chars().take(max_width - 1).collect();
    truncated.push('…');
    truncated
}

/// Name/org/platform come straight from the selected `VtuberRow` — already
/// held in `app.items`, no need to fetch them again. Only streams/clips are
/// asynchronous, tracked by `detail_load` the same way the list screen
/// tracks its own load with `load_state`.
///
/// Before a load resolves there's nothing to select yet, so the header is
/// drawn alone over the full area. Once loaded, it shrinks to a fixed-height
/// block and streams/clips render as real `List` widgets below it — that's
/// what lets `o` open *one* of them rather than just linking to all of them.
fn draw_detail(frame: &mut Frame, area: Rect, app: &App) {
    let Some(row) = app.selected() else {
        draw_message(frame, area, "No VTuber selected.");
        return;
    };

    let mut header_lines = vec![
        Line::styled(row.name.clone(), theme::name()),
        Line::from(vec![
            Span::styled("Platform: ", theme::muted()),
            Span::raw(row.platform.clone()),
        ]),
    ];

    if let Some(org) = &row.org {
        let text = match &row.suborg {
            Some(suborg) => format!("{org} / {suborg}"),
            None => org.clone(),
        };
        header_lines.push(Line::from(vec![
            Span::styled("Org:      ", theme::muted()),
            Span::raw(text),
        ]));
    }

    let detail = match &app.detail_load {
        LoadState::Loading => {
            header_lines.push(Line::raw("Loading..."));
            None
        }
        LoadState::Failed(msg) => {
            header_lines.push(Line::raw(format!("Error: {msg}")));
            None
        }
        LoadState::Loaded => app.detail.as_ref(),
    };

    let Some(detail) = detail else {
        let block = Block::default().title(" VTuber Detail ").borders(Borders::ALL);
        frame.render_widget(Paragraph::new(header_lines).block(block), area);
        return;
    };

    let is_live = detail.streams.iter().any(|s| s.status == "live");
    header_lines.push(Line::from(vec![
        Span::styled("Status: ", theme::muted()),
        Span::styled(
            if is_live { "LIVE" } else { "offline" },
            theme::live_status(is_live),
        ),
    ]));

    let chunks = Layout::vertical([
        Constraint::Length(header_lines.len() as u16 + 2), // +2 for the block's borders
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .split(area);

    let header_block = Block::default().title(" VTuber Detail ").borders(Borders::ALL);
    frame.render_widget(Paragraph::new(header_lines).block(header_block), chunks[0]);

    draw_stream_list(frame, chunks[1], detail, app);
    draw_clip_list(frame, chunks[2], detail, app);
}

fn draw_stream_list(frame: &mut Frame, area: Rect, detail: &VtuberDetail, app: &App) {
    let items: Vec<ListItem> = if detail.streams.is_empty() {
        vec![ListItem::new(Line::styled("(none)", theme::muted()))]
    } else {
        detail
            .streams
            .iter()
            .map(|s| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("[{}] ", s.status), theme::status_tag(&s.status)),
                    Span::raw(s.title.clone()),
                    Span::raw("  "),
                    Span::styled(s.url.clone(), theme::url()),
                ]))
            })
            .collect()
    };

    let list = focusable_list(items, " Recent streams ", app.detail_focus == DetailFocus::Streams);
    let mut state = app.stream_state;
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_clip_list(frame: &mut Frame, area: Rect, detail: &VtuberDetail, app: &App) {
    let items: Vec<ListItem> = if detail.clips.is_empty() {
        vec![ListItem::new(Line::styled("(none)", theme::muted()))]
    } else {
        detail
            .clips
            .iter()
            .map(|c| {
                ListItem::new(Line::from(vec![
                    Span::raw(c.title.clone()),
                    Span::raw(" "),
                    Span::styled(format!("({} views)", c.view_count), theme::muted()),
                    Span::raw("  "),
                    Span::styled(c.url.clone(), theme::url()),
                ]))
            })
            .collect()
    };

    let list = focusable_list(items, " Recent clips ", app.detail_focus == DetailFocus::Clips);
    let mut state = app.clip_state;
    frame.render_stateful_widget(list, area, &mut state);
}

/// The unfocused pane still shows a selection (so `Tab` back to it lands
/// somewhere sane) but without the reversed/bold highlight or a bright
/// border — those are reserved for whichever pane `o` currently acts on.
fn focusable_list<'a>(items: Vec<ListItem<'a>>, title: &str, focused: bool) -> List<'a> {
    let border_style = if focused { Style::default() } else { theme::muted() };
    let highlight_style = if focused {
        Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default()
    };

    List::new(items)
        .block(
            Block::default()
                .title(title.to_string())
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .highlight_style(highlight_style)
        .highlight_symbol(if focused { "> " } else { "  " })
}

/// Doubles as `oshihub config`: same three facts (backend URL, where it
/// came from, whether a token is set), never the token itself.
fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(60, 50, area);
    frame.render_widget(Clear, popup);

    let lines = vec![
        Line::from(vec![
            Span::styled("Backend URL: ", theme::muted()),
            Span::raw(&app.config_url),
        ]),
        Line::from(vec![
            Span::styled("Source:      ", theme::muted()),
            Span::raw(&app.config_source),
        ]),
        Line::from(vec![
            Span::styled("Auth token:  ", theme::muted()),
            Span::raw(&app.config_token),
        ]),
        Line::raw(""),
        Line::styled("j/k, ↑/↓  move          Enter  open detail", theme::muted()),
        Line::styled("o         open browser  Tab    switch pane (in detail)", theme::muted()),
        Line::styled("L         live only      /      filter by name", theme::muted()),
        Line::styled("s         sync selected  d      delete selected (confirms)", theme::muted()),
        Line::styled("a         add from URL   e      edit selected", theme::muted()),
        Line::styled("q         quit           ?      toggle this help", theme::muted()),
        Line::styled("Esc/h     close/back", theme::muted()),
    ];

    let block = Block::default().title(" Help ").borders(Borders::ALL);
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, popup);
}

/// Re-derives the name from `app.selected()` rather than storing a snapshot
/// on `App` — see `event::handle_modal`'s comment: modal input is exclusive,
/// so selection can't drift while this is open, and this way the prompt and
/// the delete it confirms are guaranteed to name the same VTuber.
fn draw_confirm_delete(frame: &mut Frame, area: Rect, app: &App) {
    // Fixed height, not `centered_rect`'s usual `Percentage` — 20% of a
    // short terminal (an 80x24 default undershoots this) can be less than
    // the 5 lines of content, and a long VTuber name can outgrow a
    // `Percentage`-width popup entirely. 8 rows covers the content with a
    // spare line even once `Wrap` below turns a long name into two lines.
    let popup = centered_rect_fixed_height(60, 8, area);
    frame.render_widget(Clear, popup);

    let name = app.selected().map(|row| row.name.as_str()).unwrap_or("this VTuber");
    let lines = vec![
        Line::raw(format!("Delete {name}?")),
        Line::raw(""),
        Line::styled("This also deletes its streams, clips, and snapshots.", theme::muted()),
        Line::raw(""),
        Line::styled("y / Enter  confirm      n / Esc  cancel", theme::muted()),
    ];

    let block = Block::default().title(" Confirm delete ").borders(Borders::ALL);
    // Without `Wrap`, `Paragraph` truncates any line wider than the popup
    // hard at the border — no ellipsis, no second line — which is what
    // made the fixed message ("...streams, clips, and snapshots.", 52
    // chars) read as cut off on anything narrower than ~90 columns, and
    // would do the same to a long VTuber name on the line above it.
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        popup,
    );
}

fn draw_create_url(frame: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(60, 25, area);
    frame.render_widget(Clear, popup);

    let mut lines = vec![Line::from(vec![
        Span::raw("URL: "),
        Span::raw(&app.create_input),
        Span::raw("▏"),
    ])];

    if let Some(err) = &app.create_error {
        lines.push(Line::raw(""));
        lines.push(Line::raw(format!("Error: {err}")));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled("Enter  create      Esc  cancel", theme::muted()));

    let block = Block::default().title(" Add VTuber from URL ").borders(Borders::ALL);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_edit_form(frame: &mut Frame, area: Rect, app: &App) {
    let Some(form) = &app.edit else { return };
    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    let tracked_text = if form.is_tracked { "[x] tracked" } else { "[ ] tracked" };
    let mut lines = vec![
        edit_field_line("Name", &form.name, EditField::Name, form.focus),
        edit_field_line("English name", &form.english_name, EditField::EnglishName, form.focus),
        edit_field_line("Photo", &form.photo, EditField::Photo, form.focus),
        edit_field_line("Org", &form.org, EditField::Org, form.focus),
        edit_field_line("Suborg", &form.suborg, EditField::Suborg, form.focus),
        edit_field_line("Tracked", tracked_text, EditField::IsTracked, form.focus),
    ];

    if let Some(err) = &form.error {
        lines.push(Line::raw(""));
        lines.push(Line::raw(format!("Error: {err}")));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Tab/↑↓ field  ·  Space toggle (on Tracked)  ·  Enter save  ·  Esc cancel",
        theme::muted(),
    ));

    let block = Block::default().title(" Edit VTuber ").borders(Borders::ALL);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The focused field gets a `>` marker and bold text; unfocused fields stay
/// plain — same "only the active thing gets emphasis" convention as
/// Detail's `focusable_list`.
fn edit_field_line<'a>(label: &str, value: &'a str, field: EditField, focus: EditField) -> Line<'a> {
    let focused = focus == field;
    let marker = if focused { "> " } else { "  " };
    let style = if focused { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
    Line::from(vec![
        Span::raw(marker),
        Span::styled(format!("{label}: "), theme::muted()),
        Span::styled(value, style),
    ])
}

/// Standard ratatui popup pattern: carve a centred `percent_x` × `percent_y`
/// rect out of `area` via a 3-way vertical split then a 3-way horizontal
/// split, keeping only the middle cell of each.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

/// Same idea as `centered_rect`, but the height is an absolute row count
/// rather than a percentage of the screen. A percentage height shrinks with
/// the terminal — fine for content that's mostly whitespace, but a modal
/// with a fixed number of text lines (like confirm-delete) needs at least
/// that many rows regardless of how tall the terminal is.
fn centered_rect_fixed_height(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}
