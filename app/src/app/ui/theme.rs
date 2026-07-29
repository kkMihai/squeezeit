use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding};

pub(crate) const BG: Color = Color::Rgb(17, 16, 12);
pub(crate) const CARD_BG: Color = Color::Rgb(24, 23, 17);
pub(crate) const CARD_BORDER: Color = Color::Rgb(56, 53, 38);
pub(crate) const ACCENT: Color = Color::Rgb(250, 224, 60);
pub(crate) const TEXT: Color = Color::Rgb(232, 229, 218);
pub(crate) const DIM: Color = Color::Rgb(142, 137, 118);
pub(crate) const FAINT: Color = Color::Rgb(95, 91, 74);
pub(crate) const SELECT_BG: Color = Color::Rgb(52, 47, 20);
pub(crate) const AMBER: Color = Color::Rgb(255, 150, 40);
pub(crate) const RED: Color = Color::Rgb(240, 80, 80);
pub(crate) const CHIP_BG: Color = Color::Rgb(48, 46, 34);
pub(crate) const GAUGE_BG: Color = Color::Rgb(40, 38, 26);

pub(crate) const MAX_WIDTH: u16 = 132;
pub(crate) const TWO_COL_MIN_WIDTH: u16 = 100;
pub(crate) const COLUMN_GAP: u16 = 2;

pub(crate) const TIP_LINES: u16 = 6;
pub(crate) const LABEL_WIDTH: usize = 26;
pub(crate) const CONTROL_WIDTH: usize = 22;

pub const WINDOW_SIZE: (u16, u16) = (134, 44);

pub(crate) fn card(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(CARD_BORDER))
        .style(Style::new().bg(CARD_BG))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled("─ ", Style::new().fg(CARD_BORDER)),
            Span::styled(title.to_owned(), Style::new().fg(DIM).bold()),
            Span::styled(" ", Style::new()),
        ]))
}
