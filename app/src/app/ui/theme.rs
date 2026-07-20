use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding};

pub(super) const BG: Color = Color::Rgb(17, 16, 12);
pub(super) const CARD_BG: Color = Color::Rgb(24, 23, 17);
pub(super) const CARD_BORDER: Color = Color::Rgb(56, 53, 38);
pub(super) const ACCENT: Color = Color::Rgb(250, 224, 60);
pub(super) const TEXT: Color = Color::Rgb(232, 229, 218);
pub(super) const DIM: Color = Color::Rgb(142, 137, 118);
pub(super) const FAINT: Color = Color::Rgb(95, 91, 74);
pub(super) const SELECT_BG: Color = Color::Rgb(52, 47, 20);
pub(super) const AMBER: Color = Color::Rgb(255, 150, 40);
pub(super) const RED: Color = Color::Rgb(240, 80, 80);
pub(super) const CHIP_BG: Color = Color::Rgb(48, 46, 34);
pub(super) const GAUGE_BG: Color = Color::Rgb(40, 38, 26);

pub(super) const MAX_WIDTH: u16 = 132;
pub(super) const TIP_LINES: u16 = 3;
pub(super) const LABEL_WIDTH: usize = 30;

pub(super) const TWO_COL_MIN_WIDTH: u16 = 100;
pub(super) const COLUMN_GAP: u16 = 2;

pub const WINDOW_SIZE: (u16, u16) = (134, 44);

pub(super) fn card(title: &str) -> Block<'_> {
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
