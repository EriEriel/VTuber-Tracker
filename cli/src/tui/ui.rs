use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use super::app::{App, DetailFocus, LoadState, Screen};
use super::theme;
use crate::routes::VtuberDetail;

/// Pure render function: reads App, writes to the Frame, mutates nothing.
/// This is the "immediate mode" part — every tick, we throw away the last
/// frame and redraw the whole screen from current state. No diffing to
/// think about, no manual redraw logic — just describe what it should
/// look like *right now*.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

    if app.screen == Screen::Detail {
        draw_detail(frame, chunks[0], app);
    } else {
        match &app.load_state {
            LoadState::Loading => draw_message(frame, chunks[0], "Loading tracked VTubers..."),
            LoadState::Failed(msg) => draw_message(frame, chunks[0], &format!("Error: {msg}")),
            LoadState::Loaded if app.items.is_empty() => {
                draw_message(frame, chunks[0], "No VTubers tracked yet.")
            }
            LoadState::Loaded if app.live_only && app.visible_ids().is_empty() => {
                draw_message(frame, chunks[0], "No one is live right now.")
            }
            LoadState::Loaded => draw_list(frame, chunks[0], app),
        }
    }

    draw_status_bar(frame, chunks[1], app);

    // Overlaid last, on top of whatever chunks[0]/chunks[1] already drew —
    // closing it returns to exactly what was underneath.
    if app.screen == Screen::Help {
        draw_help(frame, area, app);
    }
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
                spans.push(Span::styled("LIVE", theme::live_status(true)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = if app.live_only {
        format!(" Tracked VTubers — live only ({}) ", visible.len())
    } else {
        format!(" Tracked VTubers ({}) ", app.items.len())
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

fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    // An action failure (currently only `o`) has no screen of its own to
    // show it on, so it takes over the status bar until the next action or
    // screen change clears it.
    if let Some(err) = &app.last_error {
        let p = Paragraph::new(format!("Error: {err}"));
        frame.render_widget(p, area);
        return;
    }

    let hints = match app.screen {
        Screen::List if app.live_only => {
            "j/k move  ·  Enter detail  ·  o open  ·  L show all  ·  ? help  ·  q quit"
        }
        Screen::List => "j/k move  ·  Enter detail  ·  o open  ·  L live only  ·  ? help  ·  q quit",
        Screen::Detail => "j/k move  ·  Tab pane  ·  o open  ·  Esc/h back",
        Screen::Help => "Esc/h close",
    };
    let p = Paragraph::new(hints).style(theme::muted());
    frame.render_widget(p, area);
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
        Line::styled("L         live only      ?      toggle this help", theme::muted()),
        Line::styled("q         quit           Esc/h  close/back", theme::muted()),
    ];

    let block = Block::default().title(" Help ").borders(Borders::ALL);
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, popup);
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
