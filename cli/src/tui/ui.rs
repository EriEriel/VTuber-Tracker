use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use super::app::{App, LoadState, Screen};
use super::theme;

/// Pure render function: reads App, writes to the Frame, mutates nothing.
/// This is the "immediate mode" part — every tick, we throw away the last
/// frame and redraw the whole screen from current state. No diffing to
/// think about, no manual redraw logic — just describe what it should
/// look like *right now*.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

    match &app.load_state {
        LoadState::Loading => draw_message(frame, chunks[0], "Loading tracked VTubers..."),
        LoadState::Failed(msg) => draw_message(frame, chunks[0], &format!("Error: {msg}")),
        LoadState::Loaded if app.items.is_empty() => {
            draw_message(frame, chunks[0], "No VTubers tracked yet.")
        }
        LoadState::Loaded => draw_list(frame, chunks[0], app),
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
    let items: Vec<ListItem> = app
        .items
        .iter()
        .map(|v| {
            let line = Line::from(vec![
                Span::styled(v.name.clone(), theme::name()),
                Span::raw("  "),
                Span::styled(format!("[{}]", v.platform), theme::muted()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Tracked VTubers ({}) ", app.items.len()))
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
        .highlight_symbol("> ");

    // render_stateful_widget needs &mut ListState, but `app` here is &App.
    // ListState is Copy, so this reads a copy through the shared borrow.
    let mut state = app.list_state;
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let hints = match app.screen {
        Screen::List => "j/k move  ·  ? help  ·  q quit",
        Screen::Help => "Esc/h close",
    };
    let p = Paragraph::new(hints).style(theme::muted());
    frame.render_widget(p, area);
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
        Line::styled("j/k, ↑/↓  move          ?      toggle this help", theme::muted()),
        Line::styled("q         quit          Esc/h  close", theme::muted()),
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
