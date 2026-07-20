use std::sync::atomic::Ordering;

use humansize::{DECIMAL, format_size};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Widget, Wrap};
use tracing::Level;

use super::theme::{ACCENT, AMBER, CHIP_BG, DIM, FAINT, GAUGE_BG, RED, TEXT, card};
use super::{Hit, SqueezeItApp, Workspace};

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
        Paragraph::new(Line::from(vec![Span::styled(
            " SQUEEZEIT ",
            Style::new().fg(Color::Black).bg(ACCENT).bold(),
        )]))
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
                Span::styled("  ·  ", Style::new().fg(FAINT)),
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
                Span::styled("nothing selected", Style::new().fg(FAINT).italic()),
                Span::styled("  —  press ", Style::new().fg(FAINT)),
                Span::styled("o", Style::new().fg(ACCENT).bold()),
                Span::styled(" for a folder or ", Style::new().fg(FAINT)),
                Span::styled("f", Style::new().fg(ACCENT).bold()),
                Span::styled(" for files", Style::new().fg(FAINT)),
            ]),
        };
        Paragraph::new(content)
            .block(card("▪ WORKSPACE"))
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
                    "{:.0}%  ·  {} / {} files",
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
        statuses.extend(stat("√", s.optimized, "optimized", ACCENT));
        statuses.extend(stat("Ø", s.locked, "locked", AMBER));
        statuses.extend(stat("–", s.skipped, "skipped", DIM));
        statuses.extend(stat("×", s.failed, "failed", RED));
        Paragraph::new(Line::from(statuses)).render(rows[0], buf);
        Paragraph::new(Line::from(Span::styled(
            format!(
                "▼ {} ({:.1}%)",
                format_size(s.bytes_saved(), DECIMAL),
                s.percent_saved()
            ),
            Style::new().fg(ACCENT).bold(),
        )))
        .alignment(Alignment::Right)
        .render(rows[0], buf);

        let row = |label: &str, value: String| {
            Line::from(vec![
                Span::styled(format!("{label:<12}"), Style::new().fg(DIM)),
                Span::styled(value, Style::new().fg(TEXT)),
            ])
        };
        let cols = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(rows[1].union(rows[2]));
        Paragraph::new(vec![
            row(
                "Work rate",
                format!(
                    "{:.1}/s real · {:.1}/s scan ({})",
                    s.recent_work_per_sec,
                    s.recent_files_per_sec,
                    if running { "now" } else { "avg" },
                ),
            ),
            row("Elapsed", format!("{:.1}s", s.elapsed_secs())),
        ])
        .render(cols[0], buf);
        Paragraph::new(vec![
            row("Input", format_size(s.bytes_before, DECIMAL)),
            row("Output", format_size(s.bytes_after, DECIMAL)),
        ])
        .render(cols[1], buf);
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

    pub(super) fn footer(&mut self, area: Rect, buf: &mut Buffer) {
        let running = self.running.load(Ordering::SeqCst);
        let mut spans = Vec::new();
        let mut x = area.x;
        let mut chip =
            |k: &str, label: &str, code: Option<KeyCode>, hits: &mut Vec<(Rect, Hit)>| {
                let width = (k.chars().count() + label.chars().count() + 5) as u16;
                if let Some(code) = code {
                    let rect = Rect {
                        x,
                        width,
                        height: 1,
                        ..area
                    };
                    hits.push((rect, Hit::Key(code)));
                }
                x += width;
                spans.push(Span::styled(
                    format!(" {k} "),
                    Style::new().fg(TEXT).bg(CHIP_BG).bold(),
                ));
                spans.push(Span::styled(format!(" {label}  "), Style::new().fg(DIM)));
            };
        if running {
            chip("c", "cancel", Some(KeyCode::Char('c')), &mut self.hits);
        } else {
            chip(
                "o",
                "select folder",
                Some(KeyCode::Char('o')),
                &mut self.hits,
            );
            chip(
                "f",
                "select files",
                Some(KeyCode::Char('f')),
                &mut self.hits,
            );
            chip("s", "start", Some(KeyCode::Char('s')), &mut self.hits);
            let has_backups = self
                .workspace
                .as_ref()
                .is_some_and(|w| squeezeit::BackupVault::has_backups(&w.root(), None));
            if has_backups {
                chip("r", "restore", Some(KeyCode::Char('r')), &mut self.hits);
            }
        }
        chip("a", "settings", Some(KeyCode::Char('a')), &mut self.hits);
        if self.advanced_open {
            chip("↑↓", "move", None, &mut self.hits);
            chip("←→", "set", None, &mut self.hits);
        }
        chip("q", "quit", Some(KeyCode::Char('q')), &mut self.hits);
        if self.quit_requested {
            spans.push(Span::styled(
                "cancelling — exiting when the batch stops…",
                Style::new().fg(AMBER).italic(),
            ));
        }
        Paragraph::new(Line::from(spans)).render(area, buf);
    }
}
