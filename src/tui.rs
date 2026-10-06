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
use ratatui_cheese::fieldset::{Fieldset, FieldsetFill};
use ratatui_cheese::help::{Binding, Help};
use ratatui_cheese::input::{Input, InputState};
use ratatui_cheese::list::{DefaultHeader, List, ListItem, ListItemContext, ListState};
use ratatui_cheese::theme::Palette;

use crate::dest::{Destination, current_user};
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
    Saved(Destination),
    AddNew,
}

impl ListItem for DestRow {
    fn height(&self) -> u16 {
        1
    }

    fn render(&self, area: Rect, buf: &mut Buffer, ctx: &ListItemContext) {
        let p = &ctx.palette;
        let fg = |dim| match (ctx.selected, dim) {
            (true, _) => p.primary,
            (false, false) => p.foreground,
            (false, true) => p.muted,
        };
        let width = area.width as usize;
        match self {
            DestRow::Saved(d) => {
                let main = format!("→ {}", d.name.as_deref().unwrap_or(&d.host));
                buf.set_string(
                    area.x,
                    area.y,
                    truncate(&main, width),
                    Style::default().fg(fg(false)),
                );
                // With a nickname, show the real target beside it; otherwise just the user.
                let detail = match (&d.name, &d.user) {
                    (Some(_), _) => d.target(),
                    (None, Some(u)) => format!("as {u}"),
                    (None, None) => String::new(),
                };
                let x = main.chars().count() + 2;
                if !detail.is_empty() && x < width {
                    buf.set_string(
                        area.x + x as u16,
                        area.y,
                        truncate(&detail, width - x),
                        Style::default().fg(p.faint),
                    );
                }
            }
            DestRow::AddNew => {
                buf.set_string(
                    area.x,
                    area.y,
                    "+ new destination",
                    Style::default().fg(fg(true)),
                );
            }
        }
    }
}

pub struct DestPick {
    /// `None` if the user cancelled.
    pub chosen: Option<Destination>,
    /// Saved destinations after any edits or removals made in the picker.
    pub destinations: Vec<Destination>,
}

const HOST: usize = 0;
const USER: usize = 1;
const NICK: usize = 2;

struct Editor {
    fields: [InputState; 3],
    focus: usize,
    /// Row being edited, or `None` when adding a new destination.
    editing: Option<usize>,
}

impl Editor {
    fn new(dest: Option<&Destination>, prefill: &str, editing: Option<usize>) -> Self {
        let mut fields = [
            InputState::new().validator(validate_host),
            InputState::new().validator(validate_user),
            InputState::new(),
        ];
        let values = match dest {
            Some(d) => [
                d.host.clone(),
                d.user.clone().unwrap_or_default(),
                d.name.clone().unwrap_or_default(),
            ],
            None => [prefill.to_string(), String::new(), String::new()],
        };
        for (f, v) in fields.iter_mut().zip(values) {
            f.set_value(v);
            f.end();
        }
        let mut editor = Editor {
            fields,
            focus: HOST,
            editing,
        };
        editor.focus_field(HOST);
        editor
    }

    fn focus_field(&mut self, i: usize) {
        self.focus = i;
        for (j, f) in self.fields.iter_mut().enumerate() {
            f.set_focused(j == i);
        }
    }

    fn current(&mut self) -> &mut InputState {
        &mut self.fields[self.focus]
    }

    /// Validate every field; on failure focus the first bad one.
    fn finish(&mut self) -> Option<Destination> {
        if let Some(bad) = (0..3).find(|&i| !self.fields[i].validate()) {
            self.focus_field(bad);
            return None;
        }
        let value = |i: usize| {
            let v = self.fields[i].value().trim();
            (!v.is_empty()).then(|| v.to_string())
        };
        // `user@host` typed into the host field fills in the user.
        let mut dest = Destination::parse(&value(HOST)?);
        dest.user = value(USER).or(dest.user);
        dest.name = value(NICK);
        Some(dest)
    }
}

fn validate_host(v: &str) -> ValidationResult {
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

fn validate_user(v: &str) -> ValidationResult {
    let v = v.trim();
    if v.chars().any(|c| c.is_whitespace() || c == '@') {
        Err("no spaces or @ in user".into())
    } else {
        Ok(None)
    }
}

struct DestApp<'a> {
    rows: Vec<DestRow>,
    list: ListState,
    editor: Option<Editor>,
    sending: &'a str,
    prefill: &'a str,
    network: Option<&'a str>,
}

impl DestApp<'_> {
    fn has_saved(&self) -> bool {
        self.rows.len() > 1
    }

    fn open_new(&mut self) {
        self.editor = Some(Editor::new(None, self.prefill, None));
    }

    fn open_edit(&mut self, i: usize) {
        if let DestRow::Saved(d) = &self.rows[i] {
            self.editor = Some(Editor::new(Some(d), self.prefill, Some(i)));
        }
    }
}

/// Pick where to send. The last row adds a new destination; with nothing
/// saved, the editor opens straight away with the host prefilled.
pub fn pick_destination(
    saved: Vec<Destination>,
    sending: &str,
    prefill: &str,
    network: Option<&str>,
) -> Result<DestPick> {
    let mut rows: Vec<DestRow> = saved.into_iter().map(DestRow::Saved).collect();
    rows.push(DestRow::AddNew);
    let mut app = DestApp {
        list: ListState::new(rows.len()),
        rows,
        editor: None,
        sending,
        prefill,
        network,
    };
    if !app.has_saved() {
        app.open_new();
    }

    let mut terminal = ratatui::init();
    let result = dest_loop(&mut terminal, &mut app);
    ratatui::restore();
    let chosen = result?;

    let destinations = app
        .rows
        .into_iter()
        .filter_map(|r| match r {
            DestRow::Saved(d) => Some(d),
            DestRow::AddNew => None,
        })
        .collect();
    Ok(DestPick {
        chosen,
        destinations,
    })
}

fn dest_loop(terminal: &mut DefaultTerminal, app: &mut DestApp) -> Result<Option<Destination>> {
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

        let has_saved = app.has_saved();
        if let Some(editor) = &mut app.editor {
            match key.code {
                KeyCode::Esc if has_saved => app.editor = None,
                KeyCode::Esc => return Ok(None),
                KeyCode::Tab | KeyCode::Down => editor.focus_field((editor.focus + 1) % 3),
                KeyCode::BackTab | KeyCode::Up => editor.focus_field((editor.focus + 2) % 3),
                KeyCode::Enter if editor.focus < NICK => {
                    if editor.current().validate() {
                        editor.focus_field(editor.focus + 1);
                    }
                }
                KeyCode::Enter => {
                    let Some(dest) = editor.finish() else {
                        continue;
                    };
                    match editor.editing {
                        // Editing saves in place and returns to the list.
                        Some(i) => {
                            app.rows[i] = DestRow::Saved(dest);
                            app.editor = None;
                        }
                        None => return Ok(Some(dest)),
                    }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    editor.current().set_value(String::new());
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    editor.current().insert_char(c);
                }
                KeyCode::Backspace => editor.current().delete_before(),
                KeyCode::Delete => editor.current().delete_at(),
                KeyCode::Left => editor.current().move_left(),
                KeyCode::Right => editor.current().move_right(),
                KeyCode::Home => editor.current().home(),
                KeyCode::End => editor.current().end(),
                _ => {}
            }
            continue;
        }

        let n = app.rows.len();
        let i = app.list.selected();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
            KeyCode::Char('j') | KeyCode::Down => app.list.select_next(n, true),
            KeyCode::Char('k') | KeyCode::Up => app.list.select_prev(n, true),
            KeyCode::Char('n') | KeyCode::Char('+') => app.open_new(),
            KeyCode::Char('e') => app.open_edit(i),
            KeyCode::Enter => match &app.rows[i] {
                DestRow::Saved(d) => return Ok(Some(d.clone())),
                DestRow::AddNew => app.open_new(),
            },
            KeyCode::Char('x') | KeyCode::Delete => {
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
    let bindings = match &app.editor {
        None => vec![
            Binding::new("↑/↓", "move"),
            Binding::new("enter", "send"),
            Binding::new("n", "new"),
            Binding::new("e", "edit"),
            Binding::new("x", "forget"),
            Binding::new("esc/q", "cancel"),
        ],
        Some(ed) => {
            let enter = match (ed.focus, ed.editing) {
                (NICK, Some(_)) => "save",
                (NICK, None) => "send",
                _ => "next",
            };
            vec![
                Binding::new("tab/↑↓", "field"),
                Binding::new("ctrl+u", "clear"),
                Binding::new("enter", enter),
                Binding::new("esc", if app.has_saved() { "back" } else { "cancel" }),
            ]
        }
    };
    let help = help_bar(&palette, bindings);
    let body = chrome(frame, &palette, &format!("sending {}", app.sending), &help);

    let Some(editor) = &mut app.editor else {
        let header = DefaultHeader::new("Send to");
        let list = List::new(&app.rows)
            .header(&header)
            .item_spacing(0)
            .palette(palette.clone());
        StatefulWidget::render(&list, body, frame.buffer_mut(), &mut app.list);
        return;
    };

    let title = if editor.editing.is_some() {
        "Edit destination"
    } else {
        "New destination"
    };
    let fieldset = Fieldset::new()
        .title(title)
        .fill(FieldsetFill::Dash)
        .palette(&palette);
    let fs_area = Rect {
        height: body.height.min(16),
        ..body
    };
    let inner = fieldset.inner(fs_area);
    Widget::render(&fieldset, fs_area, frame.buffer_mut());

    let [_, host_area, user_area, nick_area, hint_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(1),
    ])
    .areas(inner);

    let user_desc = match current_user() {
        Some(u) => format!("optional; blank uses your ssh default ({u})"),
        None => "optional; blank uses your ssh default".to_string(),
    };
    let inputs = [
        (
            Input::new("Host")
                .description("IP, hostname, or ~/.ssh/config alias")
                .placeholder("192.168.1.20"),
            host_area,
        ),
        (
            Input::new("User")
                .description(&user_desc)
                .placeholder("(default)"),
            user_area,
        ),
        (
            Input::new("Nickname")
                .description("optional; shown in the list")
                .placeholder("e.g. desktop"),
            nick_area,
        ),
    ];
    for ((input, area), state) in inputs.into_iter().zip(editor.fields.iter_mut()) {
        StatefulWidget::render(&input.palette(&palette), area, frame.buffer_mut(), state);
    }

    if let Some(net) = app.network {
        frame.buffer_mut().set_string(
            hint_area.x,
            hint_area.y,
            truncate(&format!("this machine: {net}"), hint_area.width as usize),
            Style::default().fg(palette.faint),
        );
    }
}
