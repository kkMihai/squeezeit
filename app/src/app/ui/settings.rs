use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use squeezeit::{ProcessingBackend, Quality, SqueezeSettings};

use super::theme::{
    ACCENT, AMBER, CARD_BORDER, DIM, FAINT, LABEL_WIDTH, SELECT_BG, TEXT, TIP_LINES, card,
};
use super::{Hit, SqueezeItApp};

const RESOLUTIONS: [u32; 5] = [256, 512, 1024, 2048, 4096];
const QUALITIES: [Quality; 3] = [Quality::Fast, Quality::Normal, Quality::Slow];

pub(super) const SETTINGS: [SettingId; 10] = [
    SettingId::Resolution,
    SettingId::Quality,
    SettingId::Overdrive,
    SettingId::GenerateMipmaps,
    SettingId::ForceConvert,
    SettingId::Backup,
    SettingId::DryRun,
    SettingId::Gpu,
    SettingId::Headless,
    SettingId::GtaPath,
];

pub(super) const SETTINGS_ROWS: usize = SETTINGS.len();

#[derive(Clone, Copy)]
pub(super) enum SettingId {
    Resolution,
    Quality,
    Overdrive,
    GenerateMipmaps,
    ForceConvert,
    Backup,
    DryRun,
    Gpu,
    Headless,
    GtaPath,
}

impl SettingId {
    fn is_toggle(self) -> bool {
        !matches!(self, Self::Resolution | Self::Quality | Self::GtaPath)
    }

    fn needs_picker(self) -> bool {
        matches!(self, Self::GtaPath)
    }

    fn is_active(self, s: &SqueezeSettings) -> bool {
        match self {
            Self::Resolution | Self::Quality => true,
            Self::Overdrive => s.overdrive,
            Self::GenerateMipmaps => s.generate_mipmaps,
            Self::ForceConvert => s.force_convert,
            Self::Backup => s.make_backup,
            Self::DryRun => s.dry_run,
            Self::Gpu => s.backend == ProcessingBackend::GPU,
            Self::Headless => s.disable_ui_when_processing,
            Self::GtaPath => s.gta_install_path.is_some(),
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Resolution => "Resolution Limit",
            Self::Quality => "Encoding Quality",
            Self::Overdrive => "Overdrive",
            Self::GenerateMipmaps => "Generate Mipmaps",
            Self::ForceConvert => "Force DDS Convert",
            Self::Backup => "Backup Vault",
            Self::DryRun => "Dry Run",
            Self::Gpu => "GPU Acceleration",
            Self::Headless => "Headless Mode",
            Self::GtaPath => "GTA V Path",
        }
    }

    fn label(self, s: &SqueezeSettings) -> String {
        if self.is_toggle() {
            let glyph = if self.is_active(s) { "●" } else { "○" };
            format!("{glyph} {}", self.title())
        } else {
            self.title().to_owned()
        }
    }

    fn value(self, s: &SqueezeSettings, gpu_available: bool) -> String {
        match self {
            Self::Resolution => format!("‹ {0}×{0} ›", s.max_dimension),
            Self::Quality => format!("‹ {:?} ›", s.quality),
            Self::Overdrive => "compress normal & specular channels".into(),
            Self::GenerateMipmaps => "build full mip chains".into(),
            Self::ForceConvert => "transcode to DDS even if larger".into(),
            Self::Backup => "supports rollback via r".into(),
            Self::DryRun => "evaluate without disk writes".into(),
            Self::Gpu if gpu_available => "resize & block-compress in VRAM".into(),
            Self::Gpu => "no GPU device available".into(),
            Self::Headless => "freeze UI while processing for raw speed".into(),
            Self::GtaPath => match &s.gta_install_path {
                Some(path) => {
                    let display = path.display().to_string();
                    if display.len() > 40 {
                        format!("…{}", &display[display.len() - 37..])
                    } else {
                        display
                    }
                }
                None => "not set".into(),
            },
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Self::Resolution => {
                "Textures larger than this are downscaled to fit it. Smaller textures are never \
                 upscaled. 1024×1024 is a good balance."
            }
            Self::Quality => {
                "Effort spent on block compression. Fast is near-instant with slightly rougher \
                 gradients; Slow squeezes the best image quality out of the same file size. Does \
                 not change the output size, only fidelity and processing time."
            }
            Self::Overdrive => {
                "Also downscales normal and specular maps, to half the size of their paired \
                 diffuse texture. Biggest extra savings; slight loss of surface detail up close."
            }
            Self::GenerateMipmaps => {
                "Builds the full mipmap chain for textures that ship without one. Reduces shimmer \
                 at a distance and lets the game stream smaller mip levels."
            }
            Self::ForceConvert => {
                "Converts every PNG/JPG/TGA/BMP to DDS even when the DDS comes out larger — small \
                 textures below the size cap no longer get skipped. Turns SqueezeIt into a DDS \
                 converter."
            }
            Self::Backup => {
                "Copies each original file into a vault folder before overwriting it, so restore \
                 (r) can undo the whole batch. Costs extra disk space."
            }
            Self::DryRun => {
                "Runs the full pipeline and reports the savings, but writes nothing to disk. Use \
                 it to preview results before committing."
            }
            Self::Gpu => {
                "Resizes and block-compresses textures on the graphics card instead of the CPU — \
                 much faster on large batches. Textures the GPU can't take (unsupported format, \
                 busy) fall back to the CPU automatically."
            }
            Self::Headless => {
                "Stops redrawing this window while a batch runs, freeing every core for \
                 processing. Progress and logs appear once the batch finishes."
            }
            Self::GtaPath => {
                "Path to GTA5.exe. SqueezeIt extracts encryption keys from the executable to \
                 process encrypted RPF archives. Required for GTA V mod folders."
            }
        }
    }

    fn adjust(self, s: &mut SqueezeSettings, forward: bool) -> bool {
        match self {
            Self::Resolution => {
                s.max_dimension = cycle(&RESOLUTIONS, s.max_dimension, forward, 2);
            }
            Self::Quality => s.quality = cycle(&QUALITIES, s.quality, forward, 1),
            Self::Overdrive => s.overdrive = !s.overdrive,
            Self::GenerateMipmaps => s.generate_mipmaps = !s.generate_mipmaps,
            Self::ForceConvert => s.force_convert = !s.force_convert,
            Self::Backup => s.make_backup = !s.make_backup,
            Self::DryRun => s.dry_run = !s.dry_run,
            Self::Gpu => {
                let turn_on = s.backend != ProcessingBackend::GPU;
                s.backend = if turn_on {
                    ProcessingBackend::GPU
                } else {
                    ProcessingBackend::CPU
                };
                return turn_on;
            }
            Self::Headless => s.disable_ui_when_processing = !s.disable_ui_when_processing,
            Self::GtaPath => {
                if !forward {
                    s.gta_install_path = None;
                }
            }
        }
        false
    }
}

fn cycle<T: Copy + PartialEq>(items: &[T], current: T, forward: bool, default: usize) -> T {
    let n = items.len();
    let i = items.iter().position(|&x| x == current).unwrap_or(default);
    items[(i + if forward { 1 } else { n - 1 }) % n]
}

impl SqueezeItApp {
    pub(super) fn adjust_setting(&mut self, forward: bool) {
        let Some(setting) = SETTINGS.get(self.selected) else {
            return;
        };
        if setting.needs_picker() && forward {
            let filter_name = "GTA5.exe";
            let filter_ext = &["exe"];
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(filter_name, filter_ext)
                .set_title("Select GTA5.exe")
                .pick_file()
            {
                self.set_gta_install_path(Some(path));
            }
            return;
        }
        if setting.adjust(&mut self.settings, forward)
            && let Some(gpu) = &self.gpu
        {
            gpu.warm_up();
        }
    }

    pub(super) fn settings_card(&mut self, area: Rect, buf: &mut Buffer) {
        let block = card("≡ SETTINGS");
        let inner = block.inner(area);
        block.render(area, buf);

        let gpu_available = self.gpu.is_some();
        let list_height = (inner.height.saturating_sub(1 + TIP_LINES)).max(1) as usize;
        let offset = self.selected.saturating_sub(list_height - 1);
        let end = (offset + list_height).min(SETTINGS_ROWS);

        for (j, i) in (offset..end).enumerate() {
            let row_rect = Rect {
                y: inner.y + j as u16,
                height: 1,
                ..inner
            };
            self.hits.push((row_rect, Hit::Row(i)));
        }

        let lines: Vec<Line> = SETTINGS
            .iter()
            .enumerate()
            .skip(offset)
            .take(end - offset)
            .map(|(i, setting)| {
                let selected = i == self.selected;
                let on = setting.is_active(&self.settings);
                let label = setting.label(&self.settings);
                let value = setting.value(&self.settings, gpu_available);
                let base = if selected {
                    Style::new().bg(SELECT_BG)
                } else {
                    Style::new()
                };
                let label_style = base.fg(if selected {
                    ACCENT
                } else if on {
                    TEXT
                } else {
                    DIM
                });
                let pad =
                    (inner.width as usize).saturating_sub(2 + LABEL_WIDTH + value.chars().count());
                Line::from(vec![
                    Span::styled(if selected { "► " } else { "  " }, base.fg(ACCENT)),
                    Span::styled(
                        format!("{label:<width$}", width = LABEL_WIDTH),
                        label_style.bold(),
                    ),
                    Span::styled(value, base.fg(if selected { TEXT } else { FAINT })),
                    Span::styled(" ".repeat(pad), base),
                ])
            })
            .collect();

        let parts = Layout::vertical([
            Constraint::Length(list_height as u16),
            Constraint::Length(1),
            Constraint::Length(TIP_LINES),
        ])
        .split(inner);
        Paragraph::new(lines).render(parts[0], buf);

        let marker = |n: usize, arrow: &str| {
            Line::from(Span::styled(
                format!("{arrow} {n} more "),
                Style::new().fg(AMBER).bold(),
            ))
        };
        if offset > 0 {
            Paragraph::new(marker(offset, "▲"))
                .alignment(Alignment::Right)
                .render(
                    Rect {
                        height: 1,
                        ..parts[0]
                    },
                    buf,
                );
        }
        if end < SETTINGS_ROWS {
            let last = Rect {
                y: parts[0].y + list_height as u16 - 1,
                height: 1,
                ..parts[0]
            };
            Paragraph::new(marker(SETTINGS_ROWS - end, "▼"))
                .alignment(Alignment::Right)
                .render(last, buf);
        }
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            Style::new().fg(CARD_BORDER),
        )))
        .render(parts[1], buf);
        Paragraph::new(Line::from(vec![
            Span::styled("» ", Style::new().fg(ACCENT)),
            Span::styled(SETTINGS[self.selected].tip(), Style::new().fg(DIM).italic()),
        ]))
        .wrap(Wrap { trim: true })
        .render(parts[2], buf);
    }
}
