use super::theme::{ACCENT, AMBER, CHIP_BG, DIM, FAINT, GAUGE_BG, GOOD, RED, TEXT, card};
use super::{Hit, Phase, SqueezeItApp, Workspace};
use humansize::{DECIMAL, format_size};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Widget, Wrap};
use std::sync::atomic::Ordering;
use tracing::Level;
const BADGE_WIDTH: usize = 5;

fn wrapped_line_count(msg: &str, width: usize) -> usize {
    (BADGE_WIDTH + msg.chars().count())
        .div_ceil(width.max(1))
        .max(1)
}

impl SqueezeItApp {
    pub(super) fn header(&self, area: Rect, buf: &mut Buffer) {
        let running = self.running.load(Ordering::SeqCst);
        let cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(16)]).split(area);
        Paragraph::new(Line::from(Span::styled(
            " SQUEEZEIT ",
            Style::new().fg(Color::Black).bg(ACCENT).bold(),
        )))
        .render(cols[0], buf);

        let (dot, label) = if self.quit_requested {
            (AMBER, "CANCELLING")
        } else if running {
            (ACCENT, "PROCESSING")
        } else {
            (FAINT, "IDLE")
        };
        Paragraph::new(Line::from(vec![
            Span::styled("● ", Style::new().fg(dot)),
            Span::styled(label, Style::new().fg(DIM).bold()),
        ]))
        .alignment(Alignment::Right)
        .render(cols[1], buf);
    }

    pub(super) fn folder_card(&mut self, area: Rect, buf: &mut Buffer) {
        self.hits.push((area, Hit::Key(KeyCode::Char('o'))));
        let content = match &self.workspace {
            Some(Workspace::Folder(folder)) => Line::from(Span::styled(
                folder.display().to_string(),
                Style::new().fg(TEXT).bold(),
            )),
            Some(Workspace::Files(files)) => Line::from(vec![
                Span::styled(
                    format!("{} file(s)", files.len()),
                    Style::new().fg(ACCENT).bold(),
                ),
                Span::styled("  in  ", Style::new().fg(FAINT)),
                Span::styled(
                    files
                        .first()
                        .and_then(|f| f.parent())
                        .map(|d| d.display().to_string())
                        .unwrap_or_default(),
                    Style::new().fg(TEXT).bold(),
                ),
            ]),
            None => Line::from(vec![
                Span::styled("nothing selected, press ", Style::new().fg(FAINT)),
                Span::styled("o", Style::new().fg(ACCENT).bold()),
                Span::styled(" for a folder or ", Style::new().fg(FAINT)),
                Span::styled("f", Style::new().fg(ACCENT).bold()),
                Span::styled(" for files", Style::new().fg(FAINT)),
            ]),
        };
        Paragraph::new(content)
            .block(card("WORKSPACE"))
            .render(area, buf);
    }

    pub(super) fn progress_card(&self, area: Rect, buf: &mut Buffer) {
        let s = self.report.snapshot();
        let ratio = f64::from(s.progress()).clamp(0.0, 1.0);
        Gauge::default()
            .block(card("PROGRESS"))
            .gauge_style(Style::new().fg(ACCENT).bg(GAUGE_BG))
            .ratio(ratio)
            .label(Span::styled(
                format!(
                    "{:.0}%   {} / {} files",
                    ratio * 100.0,
                    s.processed(),
                    s.total_files
                ),
                Style::new().fg(TEXT).bold(),
            ))
            .render(area, buf);
    }

    pub(super) fn dashboard_card(&self, area: Rect, buf: &mut Buffer) {
        let s = self.report.snapshot();
        let running = self.running.load(Ordering::SeqCst);
        let block = card("METRICS");
        let inner = block.inner(area);
        block.render(area, buf);
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

        let stat = |glyph: &str, count: u64, label: &str, color: Color| {
            let style = if count > 0 {
                Style::new().fg(color).bold()
            } else {
                Style::new().fg(FAINT)
            };
            vec![
                Span::styled(format!("{glyph} {count}"), style),
                Span::styled(format!(" {label}    "), Style::new().fg(FAINT)),
            ]
        };
        let mut statuses = Vec::new();
        statuses.extend(stat("✓", s.optimized, "smaller", GOOD));
        statuses.extend(stat("⊘", s.locked, "protected", AMBER));
        statuses.extend(stat("·", s.skipped, "skipped", DIM));
        statuses.extend(stat("✗", s.failed, "failed", RED));
        Paragraph::new(Line::from(statuses)).render(rows[0], buf);

        let row = |label: &str, value: String, color: Color| {
            Line::from(vec![
                Span::styled(format!("{label:<9}"), Style::new().fg(DIM)),
                Span::styled(value, Style::new().fg(color).bold()),
            ])
        };
        let cols = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(rows[1].union(rows[2]));
        Paragraph::new(vec![
            row(
                "Saved",
                format!(
                    "{} ({:.1}%)",
                    format_size(s.bytes_saved(), DECIMAL),
                    s.percent_saved()
                ),
                GOOD,
            ),
            row(
                "Speed",
                format!(
                    "{:.1} files/s {}, {:.0}s so far",
                    s.recent_work_per_sec,
                    if running { "now" } else { "average" },
                    s.elapsed_secs(),
                ),
                TEXT,
            ),
        ])
        .render(cols[0], buf);
        Paragraph::new(vec![
            row("Before", format_size(s.bytes_before, DECIMAL), TEXT),
            row("After", format_size(s.bytes_after, DECIMAL), TEXT),
        ])
        .render(cols[1], buf);

        let textures = if s.has_texture_memory() {
            row(
                "Textures",
                format!(
                    "{} -> {} in memory ({:.1}% less)",
                    format_size(s.textures_before, DECIMAL),
                    format_size(s.textures_after, DECIMAL),
                    s.percent_textures_saved(),
                ),
                GOOD,
            )
        } else {
            Line::from(vec![
                Span::styled(format!("{:<9}", "Textures"), Style::new().fg(DIM)),
                Span::styled("not measured for these files", Style::new().fg(FAINT)),
            ])
        };
        Paragraph::new(textures).render(rows[3], buf);
    }

    pub(super) fn activity_card(&self, area: Rect, buf: &mut Buffer) {
        let block = card("ACTIVITY");
        let inner = block.inner(area);
        block.render(area, buf);

        let width = inner.width.max(1) as usize;
        let height = inner.height as usize;

        let mut chosen: Vec<&(Level, String)> = Vec::new();
        let mut used = 0usize;
        for entry in self.log_messages.iter().rev() {
            let est = wrapped_line_count(&entry.1, width);
            if used + est > height && !chosen.is_empty() {
                break;
            }
            used += est;
            chosen.push(entry);
        }
        chosen.reverse();

        let lines: Vec<Line> = chosen
            .iter()
            .map(|(level, line)| {
                let (badge, color) = match *level {
                    Level::ERROR => ("ERR ", RED),
                    Level::WARN => ("WARN", AMBER),
                    Level::INFO => ("INFO", ACCENT),
                    _ => ("DBG ", FAINT),
                };
                Line::from(vec![
                    Span::styled(format!("{badge} "), Style::new().fg(color).bold()),
                    Span::styled(line.clone(), Style::new().fg(DIM)),
                ])
            })
            .collect();
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .render(inner, buf);
    }

    pub(super) fn footer_chips(&self) -> Vec<Chip> {
        let phase = self.phase();
        let mut chips = Vec::new();

        match phase {
            Phase::Running => chips.push(Chip::key("c", "cancel", 'c')),
            Phase::Setup => {
                chips.push(Chip::key("o", "folder", 'o'));
                chips.push(Chip::key("f", "files", 'f'));
                chips.push(Chip::key("s", "start", 's'));
            }
            Phase::Done => {
                chips.push(Chip::key("s", "run again", 's'));
                chips.push(Chip::key("o", "folder", 'o'));
                chips.push(Chip::key("f", "files", 'f'));
                chips.push(Chip::key("a", "settings", 'a'));
            }
        }
        if phase != Phase::Running && self.has_backups() {
            chips.push(Chip::key("r", "restore", 'r'));
        }
        if phase != Phase::Running && self.settings_visible() {
            chips.push(Chip::hint("↑↓", "move"));
            chips.push(Chip::hint("←→", "change"));
        }
        chips.push(Chip::key("q", "quit", 'q'));
        chips
    }

    pub(super) fn footer_height(&self, chips: &[Chip], width: u16) -> u16 {
        let rows = fold_chips(chips, width).len().max(1);
        (rows + usize::from(self.quit_requested)) as u16
    }

    pub(super) fn footer(&mut self, area: Rect, buf: &mut Buffer, chips: &[Chip]) {
        let mut lines = Vec::new();
        for (row, indices) in fold_chips(chips, area.width).into_iter().enumerate() {
            let y = area.y + row as u16;
            let mut x = area.x;
            let mut spans = Vec::new();
            for chip in indices.into_iter().map(|i| &chips[i]) {
                if let Some(code) = chip.code {
                    self.hits.push((
                        Rect {
                            x,
                            y,
                            width: chip.width(),
                            height: 1,
                        },
                        Hit::Key(code),
                    ));
                }
                x += chip.width();
                spans.push(Span::styled(
                    format!(" {} ", chip.key),
                    Style::new().fg(TEXT).bg(CHIP_BG).bold(),
                ));
                spans.push(Span::styled(
                    format!(" {}  ", chip.label),
                    Style::new().fg(DIM),
                ));
            }
            lines.push(Line::from(spans));
        }

        if self.quit_requested {
            lines.push(Line::from(Span::styled(
                "cancelling, exiting when the batch stops",
                Style::new().fg(AMBER).italic(),
            )));
        }
        Paragraph::new(lines).render(area, buf);
    }
}

pub(super) struct Chip {
    key: &'static str,
    label: &'static str,
    code: Option<KeyCode>,
}

impl Chip {
    fn key(key: &'static str, label: &'static str, code: char) -> Self {
        Self {
            key,
            label,
            code: Some(KeyCode::Char(code)),
        }
    }

    fn hint(key: &'static str, label: &'static str) -> Self {
        Self {
            key,
            label,
            code: None,
        }
    }

    fn width(&self) -> u16 {
        (self.key.chars().count() + self.label.chars().count() + 5) as u16
    }

    #[cfg(test)]
    pub(super) fn is_clickable(&self) -> bool {
        self.code.is_some()
    }
}

fn fold_chips(chips: &[Chip], width: u16) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row = Vec::new();
    let mut used = 0;
    for (i, chip) in chips.iter().enumerate() {
        if !row.is_empty() && used + chip.width() > width {
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        used += chip.width();
        row.push(i);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}
