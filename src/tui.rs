//! Interactive picker for pending items.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{StatefulWidget, Widget};
use ratatui::{DefaultTerminal, Frame};
use ratatui_cheese::field::ValidationResult;
use ratatui_cheese::help::{Binding, Help};
use ratatui_cheese::input::{Input, InputState};
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
    let marked = rows.iter().filter(|r| r.marked).count();
    let enter_label = if marked > 0 {
        format!("take {marked} marked")
    } else {
        "take".into()
    };
    let help = help_bar(
        &palette,
        vec![
            Binding::new("↑/↓", "move"),
            Binding::new("space", "mark"),
            Binding::new("a", "mark all"),
            Binding::new("enter", enter_label),
            Binding::new("esc/q", "cancel"),
        ],
    );
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let body = chrome(frame, &palette, &format!("files land in {cwd}"), &help);

    let header = DefaultHeader::new("Pending").show_count(true);
    let list = List::new(rows).header(&header).palette(palette.clone());
    StatefulWidget::render(&list, body, frame.buffer_mut(), state);
}

fn help_bar(palette: &Palette, bindings: Vec<Binding>) -> Help {
    Help::default()
        .styles(ratatui_cheese::help::HelpStyles::from_palette(palette))
        .bindings(bindings)
}

/// Draw the shared title bar and help footer; returns the body area between them.
fn chrome(frame: &mut Frame, palette: &Palette, subtitle: &str, help: &Help) -> Rect {
    let area = frame.area();
    let content = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4).min(100),
        area.height.saturating_sub(2),
    );
    let [title_area, _, body, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(help.required_height()),
    ])
    .areas(content);

    let title = " yeet ";
    let buf = frame.buffer_mut();
    buf.set_string(
        title_area.x,
        title_area.y,
        title,
        Style::default()
            .fg(palette.on_highlight)
            .bg(palette.highlight)
            .bold(),
    );
    buf.set_string(
        title_area.x + title.len() as u16 + 1,
        title_area.y,
        truncate(subtitle, content.width.saturating_sub(8) as usize),
        Style::default().fg(palette.muted),
    );
    Widget::render(help, help_area, buf);
    body
}

// ---------------------------------------------------------------------------
// Destination picker
// ---------------------------------------------------------------------------

enum DestRow {
    Saved(String),
    AddNew,
}

impl ListItem for DestRow {
    fn height(&self) -> u16 {
        1
    }

    fn render(&self, area: Rect, buf: &mut Buffer, ctx: &ListItemContext) {
        let p = &ctx.palette;
        let (text, fg) = match self {
            DestRow::Saved(host) => (
                format!("→ {host}"),
                if ctx.selected {
                    p.primary
                } else {
                    p.foreground
                },
            ),
            DestRow::AddNew => (
                "+ new destination".to_string(),
                if ctx.selected { p.primary } else { p.muted },
            ),
        };
        buf.set_string(
            area.x,
            area.y,
            truncate(&text, area.width as usize),
            Style::default().fg(fg),
        );
    }
}

pub struct DestPick {
    /// `None` if the user cancelled.
    pub chosen: Option<String>,
    /// Saved destinations after any removals made in the picker.
    pub destinations: Vec<String>,
}

#[derive(PartialEq)]
enum Mode {
    List,
    Input,
}

struct DestApp<'a> {
    rows: Vec<DestRow>,
    list: ListState,
    input: InputState,
    mode: Mode,
    sending: &'a str,
    prefill: &'a str,
    network: Option<&'a str>,
}

impl DestApp<'_> {
    fn open_input(&mut self) {
        self.input = InputState::new().validator(validate_dest);
        self.input.set_value(self.prefill.to_string());
        self.input.end();
        self.input.set_focused(true);
        self.mode = Mode::Input;
    }

    fn has_saved(&self) -> bool {
        self.rows.len() > 1
    }
}

fn validate_dest(v: &str) -> ValidationResult {
    let v = v.trim();
    if v.is_empty() {
        Err("enter a host".into())
    } else if v.chars().any(char::is_whitespace) {
        Err("no spaces allowed".into())
    } else if v.ends_with(['.', '@', ':']) {
        Err("finish the address".into())
    } else {
        Ok(None)
    }
}

/// Pick where to send. The last row adds a new destination; with nothing
/// saved, the input opens straight away, prefilled with `prefill`.
pub fn pick_destination(
    saved: Vec<String>,
    sending: &str,
    prefill: &str,
    network: Option<&str>,
) -> Result<DestPick> {
    let mut rows: Vec<DestRow> = saved.into_iter().map(DestRow::Saved).collect();
    rows.push(DestRow::AddNew);
    let mut app = DestApp {
        list: ListState::new(rows.len()),
        rows,
        input: InputState::new(),
        mode: Mode::List,
        sending,
        prefill,
        network,
    };
    if !app.has_saved() {
        app.open_input();
    }

    let mut terminal = ratatui::init();
    let result = dest_loop(&mut terminal, &mut app);
    ratatui::restore();
    let chosen = result?;

    let destinations = app
        .rows
        .into_iter()
        .filter_map(|r| match r {
            DestRow::Saved(h) => Some(h),
            DestRow::AddNew => None,
        })
        .collect();
    Ok(DestPick {
        chosen,
        destinations,
    })
}

fn dest_loop(terminal: &mut DefaultTerminal, app: &mut DestApp) -> Result<Option<String>> {
    loop {
        terminal.draw(|f| draw_dest(f, app))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(None);
        }
        let n = app.rows.len();

        if app.mode == Mode::Input {
            match key.code {
                KeyCode::Esc if app.has_saved() => app.mode = Mode::List,
                KeyCode::Esc => return Ok(None),
                KeyCode::Enter => {
                    if app.input.validate() {
                        return Ok(Some(app.input.value().trim().to_string()));
                    }
                }
                KeyCode::Char(c) => app.input.insert_char(c),
                KeyCode::Backspace => app.input.delete_before(),
                KeyCode::Delete => app.input.delete_at(),
                KeyCode::Left => app.input.move_left(),
                KeyCode::Right => app.input.move_right(),
                KeyCode::Home => app.input.home(),
                KeyCode::End => app.input.end(),
                _ => {}
            }
            continue;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
            KeyCode::Char('j') | KeyCode::Down => app.list.select_next(n, true),
            KeyCode::Char('k') | KeyCode::Up => app.list.select_prev(n, true),
            KeyCode::Char('n') | KeyCode::Char('+') => app.open_input(),
            KeyCode::Enter => match &app.rows[app.list.selected()] {
                DestRow::Saved(host) => return Ok(Some(host.clone())),
                DestRow::AddNew => app.open_input(),
            },
            KeyCode::Char('x') | KeyCode::Delete => {
                let i = app.list.selected();
                if matches!(app.rows[i], DestRow::Saved(_)) {
                    app.rows.remove(i);
                    let n = app.rows.len();
                    app.list = ListState::new(n);
                    app.list.select(i.min(n - 1), n);
                }
            }
            _ => {}
        }
    }
}

fn draw_dest(frame: &mut Frame, app: &mut DestApp) {
    let palette = Palette::charm();
    let bindings = match app.mode {
        Mode::List => vec![
            Binding::new("↑/↓", "move"),
            Binding::new("enter", "send"),
            Binding::new("n", "new"),
            Binding::new("x", "forget"),
            Binding::new("esc/q", "cancel"),
        ],
        Mode::Input => vec![
            Binding::new("enter", "send"),
            Binding::new("esc", if app.has_saved() { "back" } else { "cancel" }),
        ],
    };
    let help = help_bar(&palette, bindings);
    let body = chrome(frame, &palette, &format!("sending {}", app.sending), &help);

    match app.mode {
        Mode::List => {
            let header = DefaultHeader::new("Send to");
            let list = List::new(&app.rows)
                .header(&header)
                .item_spacing(0)
                .palette(palette.clone());
            StatefulWidget::render(&list, body, frame.buffer_mut(), &mut app.list);
        }
        Mode::Input => {
            let [input_area, _, hint_area] = Layout::vertical([
                Constraint::Length(5),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(body);
            let input = Input::new("New destination")
                .description("ssh host: user@host, an ~/.ssh/config alias, or an IP")
                .placeholder("192.168.1.20")
                .palette(&palette);
            StatefulWidget::render(&input, input_area, frame.buffer_mut(), &mut app.input);
            if let Some(net) = app.network {
                frame.buffer_mut().set_string(
                    hint_area.x,
                    hint_area.y,
                    truncate(&format!("this machine: {net}"), hint_area.width as usize),
                    Style::default().fg(palette.faint),
                );
            }
        }
    }
}
