mod cards;
mod settings;
mod theme;

pub use theme::WINDOW_SIZE;

use std::io;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

use self::theme::{ACCENT, BG, COLUMN_GAP, DIM, FAINT, MAX_WIDTH, TIP_LINES, TWO_COL_MIN_WIDTH};
use super::{SqueezeItApp, Workspace};

const PICKER_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "tga", "bmp", "dds", "ytd", "ydr", "ydd", "yft", "rpf",
];

const IDLE_TICK: Duration = Duration::from_millis(250);
const BUSY_TICK: Duration = Duration::from_millis(33);

pub(super) enum Hit {
    Key(KeyCode),
    Focus(usize),
    Bump(usize, bool),
}

impl SqueezeItApp {
    pub fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            self.drain_logs();
            self.save_if_changed();

            if self.running.load(Ordering::SeqCst) && self.headless {
                terminal.draw(|f| Self::quiet_panel(f.area(), f.buffer_mut()))?;
                self.quiet_wait()?;
                continue;
            }

            let running = self.running.load(Ordering::SeqCst);
            if self.quit_requested && !running {
                return Ok(());
            }

            terminal.draw(|f| self.draw(f.area(), f.buffer_mut()))?;

            let timeout = if running { BUSY_TICK } else { IDLE_TICK };
            if event::poll(timeout)? {
                let quit = match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
                    Event::Mouse(mouse) => self.handle_mouse(mouse),
                    Event::Resize(_, _) => {
                        terminal.clear()?;
                        false
                    }
                    _ => false,
                };
                if quit {
                    return Ok(());
                }
            }
        }
    }

    fn quiet_wait(&mut self) -> io::Result<()> {
        loop {
            if self
                .done_rx
                .as_ref()
                .is_some_and(|rx| rx.recv_timeout(Duration::from_millis(500)).is_ok())
            {
                self.done_rx = None;
                return Ok(());
            }
            if !self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            while event::poll(Duration::ZERO)? {
                if let Event::Key(key) = event::read()?
                    && key.kind == KeyEventKind::Press
                {
                    let ctrl_c = key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL);
                    if ctrl_c || matches!(key.code, KeyCode::Char('c') | KeyCode::Esc) {
                        self.cancel.store(true, Ordering::SeqCst);
                    }
                }
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(mouse.column, mouse.row);
                let hit = self.hits.iter().find(|(rect, _)| rect.contains(at));
                match hit {
                    Some((_, Hit::Key(code))) => {
                        let code = *code;
                        return self.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
                    }
                    Some((_, Hit::Focus(i))) => self.selected = *i,
                    Some((_, Hit::Bump(i, forward))) => {
                        let (i, forward) = (*i, *forward);
                        self.selected = i;
                        if !self.running.load(Ordering::SeqCst) {
                            self.adjust_setting(forward);
                        }
                    }
                    None => {}
                }
            }
            MouseEventKind::ScrollUp if self.advanced_open => self.move_selection(-1),
            MouseEventKind::ScrollDown if self.advanced_open => self.move_selection(1),
            _ => {}
        }
        false
    }

    fn handle_key(&mut self, mut key: KeyEvent) -> bool {
        if let KeyCode::Char(c) = key.code {
            key.code = KeyCode::Char(c.to_ascii_lowercase());
        }
        let running = self.running.load(Ordering::SeqCst);

        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if running {
                self.cancel.store(true, Ordering::SeqCst);
                self.quit_requested = true;
                return false;
            }
            return true;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if running {
                    self.cancel.store(true, Ordering::SeqCst);
                    self.quit_requested = true;
                } else {
                    return true;
                }
            }
            KeyCode::Char('o') if !running => {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    self.workspace = Some(Workspace::Folder(folder));
                }
            }
            KeyCode::Char('f') if !running => {
                let picked = rfd::FileDialog::new()
                    .add_filter("Textures", PICKER_EXTENSIONS)
                    .add_filter("All files", &["*"])
                    .pick_files();
                if let Some(files) = picked
                    && !files.is_empty()
                {
                    self.workspace = Some(Workspace::Files(files));
                }
            }
            KeyCode::Char('s') | KeyCode::Enter if !running && self.workspace.is_some() => {
                self.start();
            }
            KeyCode::Char('c') if running => self.cancel.store(true, Ordering::SeqCst),
            KeyCode::Char('r') if !running && self.has_backups() => self.restore(),
            KeyCode::Char('a') | KeyCode::Tab => self.advanced_open = !self.advanced_open,
            KeyCode::Up | KeyCode::Char('k') if self.advanced_open => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') if self.advanced_open => self.move_selection(1),
            KeyCode::Left | KeyCode::Char('h') if self.advanced_open && !running => {
                self.adjust_setting(false);
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ')
                if self.advanced_open && !running =>
            {
                self.adjust_setting(true);
            }
            _ => {}
        }
        false
    }

    fn drain_logs(&mut self) {
        while let Some(msg) = self.log_queue.pop() {
            if self.log_messages.len() >= crate::logging::LOG_CAPACITY {
                self.log_messages.pop_front();
            }
            self.log_messages.push_back(msg);
        }
    }

    fn quiet_panel(area: Rect, buf: &mut Buffer) {
        Block::default()
            .style(Style::new().bg(BG))
            .render(area, buf);
        let rows = Layout::vertical([
            Constraint::Percentage(40),
            Constraint::Length(4),
            Constraint::Min(0),
        ])
        .split(area);
        Paragraph::new(vec![
            Line::from(Span::styled(
                "  PROCESSING ASSETS  ",
                Style::new().fg(Color::Black).bg(ACCENT).bold(),
            )),
            Line::default(),
            Line::from("UI is off so every core goes to the work.".fg(DIM)),
            Line::from("press c to cancel".fg(FAINT).italic()),
        ])
        .alignment(Alignment::Center)
        .render(rows[1], buf);
    }

    fn draw(&mut self, area: Rect, buf: &mut Buffer) {
        self.hits.clear();
        Block::default()
            .style(Style::new().bg(BG))
            .render(area, buf);

        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 0,
        });

        if inner.width >= TWO_COL_MIN_WIDTH {
            let cols = Layout::horizontal([
                Constraint::Length(79),
                Constraint::Length(COLUMN_GAP),
                Constraint::Min(0),
            ])
            .split(inner);
            self.controls_column(cols[0], buf, false);
            self.activity_card(cols[2], buf);
        } else {
            let width = inner.width.min(MAX_WIDTH);
            let bounded = Rect {
                x: inner.x + (inner.width - width) / 2,
                width,
                ..inner
            };
            self.controls_column(bounded, buf, true);
        }
    }

    fn controls_column(&mut self, area: Rect, buf: &mut Buffer, include_activity: bool) {
        let settings_height = self.settings_height(area.height, include_activity);
        let filler = if include_activity {
            Constraint::Min(4)
        } else {
            Constraint::Min(0)
        };
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(settings_height),
            filler,
            Constraint::Length(1),
        ])
        .split(area);

        self.header(rows[0], buf);
        self.folder_card(rows[1], buf);
        self.progress_card(rows[2], buf);
        self.dashboard_card(rows[3], buf);
        if self.advanced_open {
            self.settings_card(rows[4], buf);
        }
        if include_activity {
            self.activity_card(rows[5], buf);
        }
        self.footer(rows[6], buf);
    }

    fn settings_height(&self, available: u16, include_activity: bool) -> u16 {
        if !self.advanced_open {
            return 0;
        }
        let desired = self.row_count() as u16 + 3 + TIP_LINES;
        let fixed = 2 + 3 + 3 + 5 + 1;
        let reserve = if include_activity { 4 } else { 0 };
        desired
            .min(available.saturating_sub(fixed + reserve))
            .max(3 + TIP_LINES + 1)
    }
}
