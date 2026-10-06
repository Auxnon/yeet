//! Interactive picker for pending items.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{StatefulWidget, Widget};
use ratatui::{DefaultTerminal, Frame};
use ratatui_cheese::help::{Binding, Help};
use ratatui_cheese::list::{DefaultHeader, List, ListItem, ListItemContext, ListState};
use ratatui_cheese::theme::Palette;

use crate::warehouse::{self, Item, Kind};

struct Row {
    item: Item,
    marked: bool,
}

impl ListItem for Row {
    fn height(&self) -> u16 {
        2
    }

    fn render(&self, area: Rect, buf: &mut Buffer, ctx: &ListItemContext) {
        let p = &ctx.palette;
        let m = &self.item.meta;
        let (title_fg, sub_fg) = if ctx.selected {
            (p.primary, p.muted)
        } else {
            (p.foreground, p.faint)
        };

        let check = if self.marked { "◉ " } else { "○ " };
        let check_fg = if self.marked { p.success } else { p.faint };
        buf.set_string(area.x, area.y, check, Style::default().fg(check_fg));

        let icon = match (m.kind, m.is_dir) {
            (Kind::Text, _) => "✎ ",
            (Kind::File, true) => "▸ ",
            (Kind::File, false) => "□ ",
        };
        let title = format!("{icon}{}", warehouse::describe(&self.item));
        let max = area.width.saturating_sub(2) as usize;
        buf.set_string(
            area.x + 2,
            area.y,
            truncate(&title, max),
            Style::default().fg(title_fg),
        );

        if area.height > 1 {
            let kind = match m.kind {
                Kind::Text => "text",
                Kind::File if m.is_dir => "folder",
                Kind::File => "file",
            };
            let sub = format!(
                "{kind} · {} · from {} · {}",
                warehouse::human_size(m.size),
                m.from,
                warehouse::human_age(m.sent_at)
            );
            buf.set_string(
                area.x + 2,
                area.y + 1,
                truncate(&sub, max),
                Style::default().fg(sub_fg),
            );
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Show the picker. Returns the chosen items, or `None` if the user cancelled.
/// Nothing in the warehouse is touched here.
pub fn pick(items: Vec<Item>) -> Result<Option<Vec<Item>>> {
    let mut rows: Vec<Row> = items
        .into_iter()
        .map(|item| Row {
            item,
            marked: false,
        })
        .collect();
    let mut state = ListState::new(rows.len());
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut rows, &mut state);
    ratatui::restore();

    Ok(match result? {
        false => None,
        true => {
            let any_marked = rows.iter().any(|r| r.marked);
            let selected = state.selected();
            Some(
                rows.into_iter()
                    .enumerate()
                    .filter(|(i, r)| if any_marked { r.marked } else { *i == selected })
                    .map(|(_, r)| r.item)
                    .collect(),
            )
        }
    })
}

/// Returns `true` when the user confirmed, `false` on cancel.
fn event_loop(
    terminal: &mut DefaultTerminal,
    rows: &mut [Row],
    state: &mut ListState,
) -> Result<bool> {
    let n = rows.len();
    loop {
        terminal.draw(|f| draw(f, rows, state))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(false),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(false);
            }
            KeyCode::Enter => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => state.select_next(n, true),
            KeyCode::Char('k') | KeyCode::Up => state.select_prev(n, true),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::PageDown => state.next_page(n),
            KeyCode::Char('h') | KeyCode::Left | KeyCode::PageUp => state.prev_page(n),
            KeyCode::Char('g') | KeyCode::Home => state.select(0, n),
            KeyCode::Char('G') | KeyCode::End => state.select(n.saturating_sub(1), n),
            KeyCode::Char(' ') => {
                let row = &mut rows[state.selected()];
                row.marked = !row.marked;
                state.select_next(n, false);
            }
            KeyCode::Char('a') => {
                let all = rows.iter().all(|r| r.marked);
                rows.iter_mut().for_each(|r| r.marked = !all);
            }
            _ => {}
        }
    }
}

fn draw(frame: &mut Frame, rows: &[Row], state: &mut ListState) {
    let palette = Palette::charm();
    let area = frame.area();
    let content = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4).min(100),
        area.height.saturating_sub(2),
    );

    let marked = rows.iter().filter(|r| r.marked).count();
    let enter_label = if marked > 0 {
        format!("take {marked} marked")
    } else {
        "take".into()
    };
    let help = Help::default()
        .styles(ratatui_cheese::help::HelpStyles::from_palette(&palette))
        .bindings(vec![
            Binding::new("↑/↓", "move"),
            Binding::new("space", "mark"),
            Binding::new("a", "mark all"),
            Binding::new("enter", enter_label),
            Binding::new("esc/q", "cancel"),
        ]);

    let [title_area, _, list_area, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(help.required_height()),
    ])
    .areas(content);

    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let title = " yeet ";
    frame.buffer_mut().set_string(
        title_area.x,
        title_area.y,
        title,
        Style::default()
            .fg(palette.on_highlight)
            .bg(palette.highlight)
            .bold(),
    );
    frame.buffer_mut().set_string(
        title_area.x + title.len() as u16 + 1,
        title_area.y,
        truncate(
            &format!("files land in {cwd}"),
            content.width.saturating_sub(8) as usize,
        ),
        Style::default().fg(palette.muted),
    );

    let header = DefaultHeader::new("Pending").show_count(true);
    let list = List::new(rows).header(&header).palette(palette.clone());
    StatefulWidget::render(&list, list_area, frame.buffer_mut(), state);
    Widget::render(&help, help_area, frame.buffer_mut());
}
