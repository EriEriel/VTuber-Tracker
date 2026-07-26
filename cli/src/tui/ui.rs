use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use super::app::{App, LoadState};

/// Pure render function: reads App, writes to the Frame, mutates nothing.
/// This is the "immediate mode" part — every tick, we throw away the last
/// frame and redraw the whole screen from current state. No diffing to
/// think about, no manual redraw logic — just describe what it should
/// look like *right now*.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    match &app.load_state {
        LoadState::Loading => draw_message(frame, area, "Loading tracked VTubers..."),
        LoadState::Failed(msg) => draw_message(frame, area, &format!("Error: {msg}")),
        LoadState::Loaded if app.items.is_empty() => {
            draw_message(frame, area, "No VTubers tracked yet.")
        }
        LoadState::Loaded => draw_list(frame, area, app),
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
                Span::raw(v.name.clone()),
                Span::raw("  "),
                Span::styled(format!("[{}]", v.platform), Style::default().fg(Color::DarkGray)),
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
    // Simplest fix for v0.1: clone the state (it's just an Option<usize>).
    let mut state = app.list_state.clone();
    frame.render_stateful_widget(list, area, &mut state);
}
