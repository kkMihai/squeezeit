use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding};
pub(crate) const BG: Color = Color::Rgb(14, 14, 17);
pub(crate) const CARD_BG: Color = Color::Rgb(21, 21, 25);
pub(crate) const CARD_BORDER: Color = Color::Rgb(45, 45, 53);
pub(crate) const RULE: Color = Color::Rgb(38, 38, 45);
pub(crate) const ACCENT: Color = Color::Rgb(247, 202, 74);
pub(crate) const TEXT: Color = Color::Rgb(226, 226, 232);
pub(crate) const DIM: Color = Color::Rgb(136, 136, 148);
pub(crate) const FAINT: Color = Color::Rgb(86, 86, 98);
pub(crate) const SELECT_BG: Color = Color::Rgb(38, 35, 24);
pub(crate) const GOOD: Color = Color::Rgb(112, 205, 140);
pub(crate) const AMBER: Color = Color::Rgb(240, 160, 66);
pub(crate) const RED: Color = Color::Rgb(232, 96, 96);
pub(crate) const CHIP_BG: Color = Color::Rgb(38, 38, 45);
pub(crate) const GAUGE_BG: Color = Color::Rgb(32, 32, 38);
pub(crate) const MAX_WIDTH: u16 = 132;
pub(crate) const TWO_COL_MIN_WIDTH: u16 = 100;
pub(crate) const COLUMN_GAP: u16 = 2;
pub(crate) const TIP_LINES: u16 = 5;
pub(crate) const LABEL_WIDTH: usize = 24;
pub(crate) const CONTROL_WIDTH: usize = 20;
pub const WINDOW_SIZE: (u16, u16) = (134, 44);

pub(crate) fn card(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(CARD_BORDER))
        .style(Style::new().bg(CARD_BG))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(" ", Style::new()),
            Span::styled(title.to_owned(), Style::new().fg(DIM).bold()),
            Span::styled(" ", Style::new()),
        ]))
}

pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
