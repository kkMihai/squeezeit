use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use squeezeit::{Backend, Knob, Preset, Quality, SqueezeSettings};

use super::theme::{
    ACCENT, AMBER, CARD_BORDER, CONTROL_WIDTH, DIM, FAINT, LABEL_WIDTH, SELECT_BG, TEXT, TIP_LINES,
    card,
};
use super::{Hit, SqueezeItApp};

const HINT_WIDTH: usize = 23;
const RESOLUTIONS: [u32; 5] = [256, 512, 1024, 2048, 4096];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    Preset,
    Resolution,
    Quality,
    Overdrive,
    Mipmaps,
    KeepFormat,
    ForceConvert,
    ClothMips,
    ClothGpu,
    VehicleMips,
    Backup,
    DryRun,
    Gpu,
    Headless,
    GtaPath,
}

impl Row {
    const ALL: [Row; 15] = [
        Row::Preset,
        Row::Resolution,
        Row::Quality,
        Row::Overdrive,
        Row::Mipmaps,
        Row::KeepFormat,
        Row::ForceConvert,
        Row::ClothMips,
        Row::ClothGpu,
        Row::VehicleMips,
        Row::Backup,
        Row::DryRun,
        Row::Gpu,
        Row::Headless,
        Row::GtaPath,
    ];

    fn visible(self, s: &SqueezeSettings) -> bool {
        match self {
            Row::ClothMips | Row::ClothGpu => s.preset.touches_cloth(),
            Row::VehicleMips => s.preset.touches_vehicles(),
            _ => true,
        }
    }

    fn lock(self, s: &SqueezeSettings) -> Option<String> {
        let knob = match self {
            Row::Resolution => Knob::Resize,
            Row::Mipmaps => Knob::Mipmaps,
            Row::Overdrive => Knob::Overdrive,
            Row::Gpu => Knob::Gpu,
            Row::KeepFormat => Knob::Format,
            Row::ClothMips if !s.generate_mipmaps => {
                return Some("needs Generate Mipmaps".into());
            }
            _ => return None,
        };
        (!s.preset.allows(knob)).then(|| format!("locked by {}", s.preset.label()))
    }

    fn is_locked(self, s: &SqueezeSettings) -> bool {
        self.lock(s).is_some()
    }

    fn title(self) -> &'static str {
        match self {
            Row::Preset => "Preset",
            Row::Resolution => "Resolution Limit",
            Row::Quality => "Encoding Quality",
            Row::Overdrive => "Overdrive",
            Row::Mipmaps => "Generate Mipmaps",
            Row::KeepFormat => "Keep Source Format",
            Row::ForceConvert => "Force DDS Convert",
            Row::ClothMips => "Cloth Mip Chains",
            Row::ClothGpu => "Cloth On GPU",
            Row::VehicleMips => "Vehicle Mip Chains",
            Row::Backup => "Backup Vault",
            Row::DryRun => "Dry Run",
            Row::Gpu => "GPU Acceleration",
            Row::Headless => "Headless Mode",
            Row::GtaPath => "GTA V Path",
        }
    }

    fn is_on(self, app: &SqueezeItApp) -> bool {
        let s = &app.settings;
        match self {
            Row::Preset | Row::Resolution | Row::Quality => true,
            Row::Overdrive => s.overdrive,
            Row::Mipmaps => s.generate_mipmaps,
            Row::KeepFormat => s.keep_source_format,
            Row::ForceConvert => s.force_convert,
            Row::ClothMips => s.cloth_mips,
            Row::ClothGpu => s.cloth_gpu,
            Row::VehicleMips => s.vehicle_mips,
            Row::Backup => s.make_backup,
            Row::DryRun => s.dry_run,
            Row::Gpu => s.backend == Backend::Gpu,
            Row::Headless => app.headless,
            Row::GtaPath => app.gta_exe.is_some(),
        }
    }

    fn control(self, app: &SqueezeItApp) -> String {
        match self {
            Row::Preset => app.settings.preset.label().to_owned(),
            Row::Resolution => format!("{0}×{0}", app.settings.max_dimension),
            Row::Quality => app.settings.quality.label().to_owned(),
            Row::GtaPath if app.gta_exe.is_some() => "clear · set".into(),
            Row::GtaPath => "browse…".into(),
            _ if self.is_on(app) => "on".into(),
            _ => "off".into(),
        }
    }

    fn hint(self, app: &SqueezeItApp) -> String {
        if let Some(reason) = self.lock(&app.settings) {
            return reason;
        }
        match self {
            Row::Preset => "rules per file".into(),
            Row::Resolution => "biggest side allowed".into(),
            Row::Quality => "encoder effort".into(),
            Row::Overdrive => "half-size normal & spec".into(),
            Row::Mipmaps => "build full mip chains".into(),
            Row::KeepFormat => "resize, no re-encode".into(),
            Row::ForceConvert => "always write DDS".into(),
            Row::ClothMips | Row::ClothGpu => "clothing opt-in".into(),
            Row::VehicleMips => "vehicle opt-in".into(),
            Row::Backup => "press r to roll back".into(),
            Row::DryRun => "writes nothing".into(),
            Row::Gpu if app.gpu.is_some() => "compress in VRAM".into(),
            Row::Gpu => "no GPU found".into(),
            Row::Headless => "freeze UI for speed".into(),
            Row::GtaPath => match &app.gta_exe {
                Some(path) => tail(&path.display().to_string(), HINT_WIDTH),
                None => "for encrypted .rpf".into(),
            },
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Row::Preset => {
                "Picks the rules for each file from its shaders and its name. Auto is right for \
                 mixed folders. Clothing shrinks, hair is left alone, vehicles and props get \
                 everything."
            }
            Row::Resolution => {
                "Halves any texture bigger than this until it fits. Never makes one bigger. \
                 Liveries, weapon skins and script_rt textures ignore it."
            }
            Row::Quality => {
                "How hard the block compressor works. Fast is instant and a bit rough. Slow looks \
                 best. The output size is the same either way, only the time changes."
            }
            Row::Overdrive => {
                "Shrinks normal and specular maps to half the size of their diffuse, and spots \
                 unlabelled maps by reading the pixels. The biggest extra saving. Surfaces get \
                 softer up close."
            }
            Row::Mipmaps => {
                "Builds the full RAGE mip chain for .ytd textures that ship without one. Less \
                 shimmer at distance and smaller levels to stream. Drawables cannot grow, so they \
                 never get new mips."
            }
            Row::KeepFormat => {
                "Keeps the pixel format the file already uses. Textures still resize, nothing is \
                 re-compressed. Force DDS Convert wins if you turn on both."
            }
            Row::ForceConvert => {
                "Turns every PNG, JPG, TGA and BMP into DDS, even when the DDS ends up bigger. \
                 Small textures stop being skipped."
            }
            Row::ClothMips => {
                "Lets one clothing texture grow a mip chain, but only if it has no alpha, keeps \
                 its format, shipped without a chain, and is a square power of two of 128 or more. \
                 Everything else is left alone."
            }
            Row::ClothGpu => {
                "Lets clothing textures be block-compressed on the graphics card. Off by default \
                 because that is the path that upset AMD drivers."
            }
            Row::VehicleMips => {
                "Lets vehicle textures grow full mip chains. Off by default because vehicles have \
                 been excluded since day one and nobody wrote down why."
            }
            Row::Backup => {
                "Copies every original into a vault folder before overwriting it. Restore puts the \
                 whole batch back. Costs disk space."
            }
            Row::DryRun => {
                "Runs the whole pipeline and reports the savings without touching a single file. \
                 Use it to preview a run."
            }
            Row::Gpu => {
                "Resizes and block-compresses on the graphics card instead of the CPU. Much faster \
                 on big batches. Anything the GPU cannot take falls back to the CPU on its own."
            }
            Row::Headless => {
                "Stops drawing this window while a batch runs so every core goes to the work. \
                 Progress and logs come back when it finishes."
            }
            Row::GtaPath => {
                "Path to GTA5.exe. SqueezeIt pulls the archive keys out of it so it can open \
                 encrypted .rpf files."
            }
        }
    }

    fn warning(self) -> Option<&'static str> {
        match self {
            Row::Preset => Some(
                "Forcing a preset turns detection off. Vehicles & props over a clothing pack is \
                 what crashed FiveM on AMD.",
            ),
            Row::Resolution => Some(
                "Presets cap clothing and hair lower than this. Hair never goes under 512, or the \
                 strands break up.",
            ),
            Row::Overdrive => Some(
                "Never runs on ped cloth or hair. Half-size maps against a full-size diffuse \
                 shifted shoe colours and broke hair's second channel.",
            ),
            Row::Mipmaps => Some(
                "On ped hair and cloth, generated mips broke strands and shifted shoe colours. \
                 Paired with a format change they lined up with AMD load crashes.",
            ),
            Row::ClothMips => Some(
                "This is the combination that upset AMD drivers. Test one pack before you trust \
                 it on a whole server.",
            ),
            Row::ClothGpu => Some(
                "GPU block compression on cloth caused driver resets and corrupt drawables on AMD \
                 cards.",
            ),
            Row::Gpu => Some("Ped cloth and hair always run on the CPU, whatever this says."),
            _ => None,
        }
    }

    fn scope(self) -> Option<&'static str> {
        match self {
            Row::Preset => {
                Some("Auto · Clothing · Hair & alpha · Hair (strict) · Vehicles & props")
            }
            Row::Resolution => Some("Clothing 1024 · Hair 1024 (floor 512) · Props your limit"),
            Row::Overdrive | Row::Gpu => Some("Safe: vehicles & props. Off: clothing, hair."),
            Row::Mipmaps => Some("Safe: vehicles & props. Off: clothing. Never: hair."),
            _ => None,
        }
    }
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .chain(['…'])
        .collect()
}

fn tail(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let skip = text.chars().count() - width + 1;
    format!("…{}", text.chars().skip(skip).collect::<String>())
}

fn cycle<T: Copy + PartialEq>(items: &[T], current: T, forward: bool) -> T {
    let n = items.len();
    let i = items.iter().position(|&x| x == current).unwrap_or(0);
    items[(i + if forward { 1 } else { n - 1 }) % n]
}

impl SqueezeItApp {
    pub(crate) fn rows(&self) -> Vec<Row> {
        Row::ALL
            .into_iter()
            .filter(|r| r.visible(&self.settings))
            .collect()
    }

    pub(super) fn row_count(&self) -> usize {
        self.rows().len()
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let n = self.row_count() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
    }

    pub(crate) fn adjust_setting(&mut self, forward: bool) {
        let rows = self.rows();
        let Some(&row) = rows.get(self.selected) else {
            return;
        };
        if row.is_locked(&self.settings) {
            return;
        }

        let warm_gpu = self.apply(row, forward);

        let rows = self.rows();
        self.selected = rows
            .iter()
            .position(|&r| r == row)
            .unwrap_or_else(|| self.selected.min(rows.len().saturating_sub(1)));

        if warm_gpu && let Some(gpu) = &self.gpu {
            gpu.warm_up();
        }
    }

    fn apply(&mut self, row: Row, forward: bool) -> bool {
        let s = &mut self.settings;
        match row {
            Row::Preset => s.preset = cycle(&Preset::ALL, s.preset, forward),
            Row::Resolution => s.max_dimension = cycle(&RESOLUTIONS, s.max_dimension, forward),
            Row::Quality => s.quality = cycle(&Quality::ALL, s.quality, forward),
            Row::Overdrive => s.overdrive = !s.overdrive,
            Row::Mipmaps => s.generate_mipmaps = !s.generate_mipmaps,
            Row::KeepFormat => s.keep_source_format = !s.keep_source_format,
            Row::ForceConvert => s.force_convert = !s.force_convert,
            Row::ClothMips => s.cloth_mips = !s.cloth_mips,
            Row::ClothGpu => s.cloth_gpu = !s.cloth_gpu,
            Row::VehicleMips => s.vehicle_mips = !s.vehicle_mips,
            Row::Backup => s.make_backup = !s.make_backup,
            Row::DryRun => s.dry_run = !s.dry_run,
            Row::Headless => self.headless = !self.headless,
            Row::Gpu => {
                let turn_on = s.backend != Backend::Gpu;
                s.backend = if turn_on { Backend::Gpu } else { Backend::Cpu };
                return turn_on;
            }
            Row::GtaPath if forward => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("GTA5.exe", &["exe"])
                    .set_title("Select GTA5.exe")
                    .pick_file()
                {
                    self.set_gta_exe(Some(path));
                }
            }
            Row::GtaPath => self.set_gta_exe(None),
        }
        false
    }

    pub(super) fn settings_card(&mut self, area: Rect, buf: &mut Buffer) {
        let block = card("≡ SETTINGS");
        let inner = block.inner(area);
        block.render(area, buf);

        let rows = self.rows();
        let total = rows.len();
        let list_height = (inner.height.saturating_sub(1 + TIP_LINES)).max(1) as usize;
        let offset = self.selected.saturating_sub(list_height - 1);
        let end = (offset + list_height).min(total);

        let parts = Layout::vertical([
            Constraint::Length(list_height as u16),
            Constraint::Length(1),
            Constraint::Length(TIP_LINES),
        ])
        .split(inner);

        let left = inner.x + 2 + LABEL_WIDTH as u16;
        let right = left + CONTROL_WIDTH as u16 - 1;

        let mut lines = Vec::with_capacity(end - offset);
        for (screen_row, i) in (offset..end).enumerate() {
            let y = inner.y + screen_row as u16;
            let row = rows[i];
            let locked = row.is_locked(&self.settings);

            if !locked {
                for (x, forward) in [(left, false), (right.saturating_sub(1), true)] {
                    let hit = Rect {
                        x,
                        y,
                        width: 2,
                        height: 1,
                    };
                    self.hits.push((hit, Hit::Bump(i, forward)));
                }
            }
            self.hits.push((
                Rect {
                    y,
                    height: 1,
                    ..inner
                },
                Hit::Focus(i),
            ));

            lines.push(self.settings_line(row, i == self.selected, inner.width as usize));
        }

        Paragraph::new(lines).render(parts[0], buf);
        self.scroll_markers(parts[0], buf, offset, end, total);

        Paragraph::new(Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            Style::new().fg(CARD_BORDER),
        )))
        .render(parts[1], buf);

        let row = rows[self.selected.min(total - 1)];
        let mut tip = vec![Line::from(vec![
            Span::styled("» ", Style::new().fg(ACCENT)),
            Span::styled(row.tip(), Style::new().fg(DIM).italic()),
        ])];
        if let Some(warning) = row.warning() {
            tip.push(Line::from(vec![
                Span::styled("⚠ ", Style::new().fg(AMBER).bold()),
                Span::styled(warning, Style::new().fg(AMBER)),
            ]));
        }
        if let Some(scope) = row.scope() {
            tip.push(Line::from(Span::styled(
                format!("  {scope}"),
                Style::new().fg(FAINT),
            )));
        }
        Paragraph::new(tip)
            .wrap(Wrap { trim: true })
            .render(parts[2], buf);
    }

    fn settings_line(&self, row: Row, selected: bool, width: usize) -> Line<'static> {
        let locked = row.is_locked(&self.settings);
        let on = row.is_on(self);
        let base = if selected {
            Style::new().bg(SELECT_BG)
        } else {
            Style::new()
        };

        let label_colour = match () {
            _ if locked => FAINT,
            _ if selected => ACCENT,
            _ if on => TEXT,
            _ => DIM,
        };
        let (arrow_colour, value_colour) = if locked {
            (FAINT, FAINT)
        } else if selected {
            (ACCENT, TEXT)
        } else {
            (FAINT, DIM)
        };

        let inner_width = CONTROL_WIDTH - 4;
        let control = clip(&row.control(self), inner_width);
        let hint = row.hint(self);
        let used = 2 + LABEL_WIDTH + CONTROL_WIDTH + 2 + hint.chars().count();

        Line::from(vec![
            Span::styled(if selected { "► " } else { "  " }, base.fg(ACCENT)),
            Span::styled(
                format!("{:<w$}", row.title(), w = LABEL_WIDTH),
                base.fg(label_colour).bold(),
            ),
            Span::styled("‹ ", base.fg(arrow_colour)),
            Span::styled(
                format!("{control:^inner_width$}"),
                base.fg(value_colour).bold(),
            ),
            Span::styled(" ›", base.fg(arrow_colour)),
            Span::styled(
                format!("  {hint}"),
                base.fg(if locked { FAINT } else { DIM }),
            ),
            Span::styled(" ".repeat(width.saturating_sub(used)), base),
        ])
    }

    #[cfg(test)]
    pub(crate) fn focused_row(&self) -> Row {
        self.rows()[self.selected]
    }

    #[cfg(test)]
    pub(crate) fn focus(&mut self, row: Row) {
        self.selected = self
            .rows()
            .iter()
            .position(|&r| r == row)
            .expect("row is visible");
    }

    fn scroll_markers(
        &self,
        area: Rect,
        buf: &mut Buffer,
        offset: usize,
        end: usize,
        total: usize,
    ) {
        let marker = |n: usize, arrow: &str| {
            Line::from(Span::styled(
                format!("{arrow} {n} more "),
                Style::new().fg(AMBER).bold(),
            ))
        };
        if offset > 0 {
            Paragraph::new(marker(offset, "▲"))
                .alignment(Alignment::Right)
                .render(Rect { height: 1, ..area }, buf);
        }
        if end < total {
            Paragraph::new(marker(total - end, "▼"))
                .alignment(Alignment::Right)
                .render(
                    Rect {
                        y: area.y + area.height.saturating_sub(1),
                        height: 1,
                        ..area
                    },
                    buf,
                );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> SqueezeItApp {
        SqueezeItApp::for_test()
    }

    fn set_preset(app: &mut SqueezeItApp, preset: Preset) {
        app.focus(Row::Preset);
        for _ in 0..Preset::ALL.len() {
            if app.settings.preset == preset {
                return;
            }
            app.adjust_setting(true);
        }
        panic!("could not reach {preset:?}");
    }

    #[test]
    fn cycling_the_preset_never_moves_the_cursor() {
        let mut app = app();
        app.focus(Row::Preset);
        for _ in 0..Preset::ALL.len() * 2 {
            app.adjust_setting(true);
            assert_eq!(app.focused_row(), Row::Preset);
        }
        for _ in 0..Preset::ALL.len() * 2 {
            app.adjust_setting(false);
            assert_eq!(app.focused_row(), Row::Preset);
        }
    }

    #[test]
    fn forward_and_back_land_on_the_same_preset() {
        let mut app = app();
        app.focus(Row::Preset);
        for _ in 0..Preset::ALL.len() {
            let before = app.settings.preset;
            app.adjust_setting(true);
            app.adjust_setting(false);
            assert_eq!(app.settings.preset, before);
        }
    }

    #[test]
    fn preset_decides_which_opt_ins_exist() {
        let mut app = app();

        set_preset(&mut app, Preset::Auto);
        for row in [Row::ClothMips, Row::ClothGpu, Row::VehicleMips] {
            assert!(app.rows().contains(&row), "auto should show every opt-in");
        }

        set_preset(&mut app, Preset::Clothing);
        assert!(app.rows().contains(&Row::ClothMips));
        assert!(!app.rows().contains(&Row::VehicleMips));

        set_preset(&mut app, Preset::VehiclesProps);
        assert!(app.rows().contains(&Row::VehicleMips));
        assert!(!app.rows().contains(&Row::ClothGpu));

        set_preset(&mut app, Preset::Hair);
        assert!(!app.rows().contains(&Row::ClothMips));
        assert!(!app.rows().contains(&Row::VehicleMips));
    }

    #[test]
    fn a_preset_greys_out_the_switches_it_overrides() {
        let mut app = app();

        set_preset(&mut app, Preset::Hair);
        for row in [Row::Overdrive, Row::Mipmaps, Row::Gpu, Row::KeepFormat] {
            assert!(
                row.is_locked(&app.settings),
                "{} should be locked under Hair",
                row.title()
            );
        }
        assert!(!Row::Resolution.is_locked(&app.settings));

        set_preset(&mut app, Preset::HairStrict);
        assert!(Row::Resolution.is_locked(&app.settings));

        set_preset(&mut app, Preset::VehiclesProps);
        for row in Row::ALL {
            assert!(
                !row.is_locked(&app.settings),
                "{} should be free under Vehicles & props",
                row.title()
            );
        }
    }

    #[test]
    fn a_locked_switch_refuses_to_move() {
        let mut app = app();
        set_preset(&mut app, Preset::Hair);
        let before = app.settings.overdrive;
        app.focus(Row::Overdrive);
        app.adjust_setting(true);
        assert_eq!(app.settings.overdrive, before, "locked row changed anyway");
    }

    #[test]
    fn the_cloth_mip_opt_in_follows_the_mipmap_switch() {
        let mut app = app();
        set_preset(&mut app, Preset::Auto);
        assert!(!Row::ClothMips.is_locked(&app.settings));

        app.focus(Row::Mipmaps);
        app.adjust_setting(true);
        assert!(!app.settings.generate_mipmaps);
        assert!(Row::ClothMips.is_locked(&app.settings));
    }

    #[test]
    fn every_preset_draws_without_panicking() {
        let mut app = app();
        for preset in Preset::ALL {
            set_preset(&mut app, preset);
            for selected in 0..app.rows().len() {
                app.selected = selected;
                let area = Rect::new(0, 0, 79, 24);
                let mut buf = Buffer::empty(area);
                app.settings_card(area, &mut buf);
            }
        }
    }

    #[test]
    fn every_label_fits_the_narrow_layout() {
        let mut app = app();
        for preset in Preset::ALL {
            set_preset(&mut app, preset);
            for row in app.rows() {
                let control = row.control(&app);
                assert!(
                    control.chars().count() <= CONTROL_WIDTH - 4,
                    "{} renders `{control}`, which overflows the arrows",
                    row.title()
                );

                let hint = row.hint(&app);
                assert!(
                    hint.chars().count() <= HINT_WIDTH,
                    "{} hints `{hint}` ({} chars, max {HINT_WIDTH})",
                    row.title(),
                    hint.chars().count(),
                );

                assert!(row.title().chars().count() < LABEL_WIDTH, "{}", row.title());
            }
        }
    }
}
