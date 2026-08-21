use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use squeezeit::{
    Backend, FormatMode, Knob, Liveries, Preset, Quality, Safety, ScriptRt, SizeLimit,
    SqueezeSettings,
};

use super::theme::{
    ACCENT, AMBER, CHIP_BG, CONTROL_WIDTH, DIM, FAINT, LABEL_WIDTH, RULE, SELECT_BG, TEXT,
    TIP_LINES, card, wrap,
};
use super::{Hit, SqueezeItApp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Size,
    Protect,
    Run,
    Machine,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::Size => "SIZE AND FORMAT",
            Group::Protect => "WHAT TO PROTECT",
            Group::Run => "THIS RUN",
            Group::Machine => "MACHINE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    Preset,
    Size,
    Quality,
    Format,
    Mipmaps,
    Overdrive,
    Safety,
    Liveries,
    ScriptRt,
    Exclusions,
    MatchCase,
    Backup,
    DryRun,
    Gpu,
    Quiet,
}

enum Entry {
    Heading(Group),
    Setting(usize),
}

impl Row {
    const ALL: [Row; 15] = [
        Row::Preset,
        Row::Size,
        Row::Quality,
        Row::Format,
        Row::Mipmaps,
        Row::Overdrive,
        Row::Safety,
        Row::Liveries,
        Row::ScriptRt,
        Row::Exclusions,
        Row::MatchCase,
        Row::Backup,
        Row::DryRun,
        Row::Gpu,
        Row::Quiet,
    ];

    fn group(self) -> Group {
        match self {
            Row::Preset | Row::Size | Row::Quality | Row::Format => Group::Size,
            Row::Mipmaps
            | Row::Overdrive
            | Row::Safety
            | Row::Liveries
            | Row::ScriptRt
            | Row::Exclusions
            | Row::MatchCase => Group::Protect,
            Row::Backup | Row::DryRun => Group::Run,
            Row::Gpu | Row::Quiet => Group::Machine,
        }
    }

    fn knob(self) -> Option<Knob> {
        match self {
            Row::Mipmaps => Some(Knob::Mipmaps),
            Row::Overdrive => Some(Knob::Overdrive),
            Row::Format => Some(Knob::Format),
            Row::Gpu => Some(Knob::Gpu),
            Row::Safety => Some(Knob::Safety),
            _ => None,
        }
    }

    fn lock(self, s: &SqueezeSettings) -> Option<String> {
        let knob = self.knob()?;
        (!s.preset.allows(knob)).then(|| format!("set by {}", s.preset.label()))
    }

    fn is_locked(self, s: &SqueezeSettings) -> bool {
        self.lock(s).is_some()
    }

    fn title(self) -> &'static str {
        match self {
            Row::Preset => "Preset",
            Row::Size => "Max Size",
            Row::Quality => "Quality",
            Row::Format => "File Format",
            Row::Mipmaps => "Mipmaps",
            Row::Overdrive => "Shrink Detail Maps",
            Row::Safety => "Safety",
            Row::Liveries => "Liveries",
            Row::ScriptRt => "Screens & Dials",
            Row::Exclusions => "Skip By Name",
            Row::MatchCase => "Match Case",
            Row::Backup => "Backup",
            Row::DryRun => "Preview Only",
            Row::Gpu => "Use GPU",
            Row::Quiet => "Hide UI While Busy",
        }
    }

    fn is_toggle(self) -> bool {
        matches!(
            self,
            Row::Mipmaps
                | Row::Overdrive
                | Row::MatchCase
                | Row::Backup
                | Row::DryRun
                | Row::Gpu
                | Row::Quiet
        )
    }

    fn is_on(self, app: &SqueezeItApp) -> bool {
        let s = &app.settings;
        match self {
            Row::Preset
            | Row::Size
            | Row::Quality
            | Row::Format
            | Row::Safety
            | Row::Liveries
            | Row::ScriptRt => true,
            Row::Exclusions => !s.exclusions.is_empty(),
            Row::MatchCase => !s.exclusions.ignore_case,
            Row::Mipmaps => s.mipmaps,
            Row::Overdrive => s.overdrive,
            Row::Backup => s.backup,
            Row::DryRun => s.dry_run,
            Row::Gpu => s.backend == Backend::Gpu,
            Row::Quiet => app.quiet,
        }
    }

    fn control(self, app: &SqueezeItApp) -> String {
        let s = &app.settings;
        match self {
            Row::Preset => s.preset.label().to_owned(),
            Row::Size => s.size_limit.label(),
            Row::Quality => s.quality.label().to_owned(),
            Row::Format => s.format.label().to_owned(),
            Row::Safety => s.safety.label().to_owned(),
            Row::Liveries => s.liveries.label().to_owned(),
            Row::ScriptRt => s.script_rt.label().to_owned(),
            Row::Exclusions if s.exclusions.is_empty() => "none".into(),
            Row::Exclusions => format!("{} listed", s.exclusions.names.len()),
            _ if self.is_on(app) => "ON".into(),
            _ => "off".into(),
        }
    }

    fn hint(self, app: &SqueezeItApp) -> String {
        if let Some(reason) = self.lock(&app.settings) {
            return reason;
        }
        match self {
            Row::Preset => "how files are handled".into(),
            Row::Size => "biggest side allowed".into(),
            Row::Quality => "how long encoding takes".into(),
            Row::Format => "what gets written".into(),
            Row::Mipmaps => "smaller distant copies".into(),
            Row::Overdrive => "extra saving, softer look".into(),
            Row::Safety => "how careful to be".into(),
            Row::Liveries => "logos, signs and skins".into(),
            Row::ScriptRt => "what the game draws on".into(),
            Row::Exclusions if app.settings.exclusions.is_empty() => "right to load a list".into(),
            Row::Exclusions => "left to clear the list".into(),
            Row::MatchCase => "for the skip list".into(),
            Row::Backup => "press r to undo a run".into(),
            Row::DryRun => "changes nothing".into(),
            Row::Gpu if app.gpu.is_some() => "much faster on big jobs".into(),
            Row::Gpu => "no graphics card found".into(),
            Row::Quiet => "gives the work every core".into(),
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Row::Preset => {
                "Works out what each file is and treats it accordingly. Auto is right for a mixed \
                 folder. The named ones treat everything as that one thing, for a folder you \
                 already know is all clothing, all hair or all vehicles."
            }
            Row::Size => {
                "Any texture wider or taller than this is halved until it fits. Nothing is ever \
                 made bigger. Clothing and hair have their own lower caps that this cannot raise. \
                 Pick no resize to leave every size alone."
            }
            Row::Quality => {
                "How long the compressor spends on each texture. Fast is instant and a little \
                 rough, slow looks best. The file comes out the same size either way, so this \
                 only costs you time."
            }
            Row::Format => {
                "Auto swaps the format when that saves space and leaves it alone when it does \
                 not. Keep same only resizes, apart from a loose .dds, which is always \
                 re-compressed. Always DDS converts every PNG, JPG, TGA and BMP even when \
                 the DDS ends up bigger."
            }
            Row::Mipmaps => {
                "Builds the smaller copies the game uses for things in the distance. Less \
                 shimmer, and less to stream. Only .ytd files can gain them, since the others \
                 cannot grow."
            }
            Row::Overdrive => {
                "Halves the bumpiness and shininess maps, which nobody looks at directly, while \
                 the colour stays full size. The biggest extra saving available. Surfaces get a \
                 little softer up close."
            }
            Row::Liveries => {
                "Liveries, signs and weapon skins are never resized, because people read them \
                 close up and at full zoom, which is where a downscale shows. Turn this off when \
                 you know yours are oversized."
            }
            Row::ScriptRt => {
                "Screens, dials and anything else the game draws on at runtime have to be \
                 uncompressed with a single level. Packs are full of ones that are neither, \
                 which makes FiveM warn and can crash clients. Fix format repairs them as it \
                 goes. Fix only these changes nothing else at all, for a pack you have already \
                 sized."
            }
            Row::Exclusions => {
                "Textures to leave exactly as they are, listed by name. Load a plain text file \
                 with one name per line. Liveries and screens are already protected and do not \
                 need listing. Names match whole, there are no wildcards."
            }
            Row::MatchCase => {
                "Whether the skip list cares about capitals. Off is usually what you want, \
                 since texture names inside a dictionary are capitalised inconsistently."
            }
            Row::Safety => {
                "Safe leaves clothing and vehicles with the mipmaps they came with. Risky lets \
                 clothing gain a set when it passes every check, and gives vehicles full ones. \
                 Hair is left alone either way."
            }
            Row::Backup => {
                "Moves every original into a folder of its own before overwriting it, so r puts \
                 the whole run back. Costs as much disk space as the folder itself."
            }
            Row::DryRun => {
                "Does the entire job and tells you what it would have saved, without touching a \
                 single file. Worth running first on anything you care about."
            }
            Row::Gpu => {
                "Does the resizing and compressing on your graphics card instead of the \
                 processor. Much faster on big folders, and safe to leave on: anything the card \
                 cannot handle goes back to the processor on its own."
            }
            Row::Quiet => {
                "Stops drawing this window while a job runs, so every core goes to the work. \
                 Progress and the log come back when it finishes."
            }
        }
    }

    fn warning(self) -> Option<&'static str> {
        match self {
            Row::Preset => Some(
                "A named preset turns detection off, so a folder that is not all one thing gets \
                 the wrong rules.",
            ),
            Row::Overdrive => Some("Never applied to clothing or hair, whatever this says."),
            Row::Mipmaps => Some(
                "Only reaches vehicles and props. Clothing and hair keep the mipmaps they came \
                 with.",
            ),
            Row::Safety => Some(
                "Risky is off by default for a reason. Try it on one pack and load it in game \
                 before trusting it on a whole server.",
            ),
            Row::Gpu => Some("Hair always uses the processor, whatever this says."),
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
        .chain(['~'])
        .collect()
}

fn cycle<T: Copy + PartialEq>(items: &[T], current: T, forward: bool) -> T {
    let n = items.len();
    let i = items.iter().position(|&x| x == current).unwrap_or(0);
    items[(i + if forward { 1 } else { n - 1 }) % n]
}

fn entries() -> Vec<Entry> {
    let mut out = Vec::with_capacity(Row::ALL.len() + 3);
    let mut current = None;
    for (i, row) in Row::ALL.iter().enumerate() {
        if current != Some(row.group()) {
            current = Some(row.group());
            out.push(Entry::Heading(row.group()));
        }
        out.push(Entry::Setting(i));
    }
    out
}

impl SqueezeItApp {
    pub(super) fn settings_lines(&self) -> u16 {
        (Row::ALL.len() + 3) as u16
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let n = Row::ALL.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
    }

    pub(crate) fn adjust_setting(&mut self, forward: bool) {
        let row = Row::ALL[self.selected.min(Row::ALL.len() - 1)];
        if row.is_locked(&self.settings) {
            return;
        }
        if self.apply(row, forward)
            && let Some(gpu) = &self.gpu
        {
            gpu.begin_warm_up();
        }
    }

    fn apply(&mut self, row: Row, forward: bool) -> bool {
        let s = &mut self.settings;
        match row {
            Row::Preset => s.preset = cycle(&Preset::ALL, s.preset, forward),
            Row::Size => s.size_limit = cycle(&SizeLimit::ALL, s.size_limit, forward),
            Row::Quality => s.quality = cycle(&Quality::ALL, s.quality, forward),
            Row::Format => s.format = cycle(&FormatMode::ALL, s.format, forward),
            Row::Safety => s.safety = cycle(&Safety::ALL, s.safety, forward),
            Row::Liveries => s.liveries = cycle(&Liveries::ALL, s.liveries, forward),
            Row::ScriptRt => s.script_rt = cycle(&ScriptRt::ALL, s.script_rt, forward),
            Row::MatchCase => s.exclusions.ignore_case = !s.exclusions.ignore_case,
            Row::Exclusions if forward => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("text", &["txt", "list"])
                    .set_title("Load a list of texture names to skip")
                    .pick_file()
                {
                    self.load_exclusions(&path);
                }
            }
            Row::Exclusions => s.exclusions.names.clear(),
            Row::Mipmaps => s.mipmaps = !s.mipmaps,
            Row::Overdrive => s.overdrive = !s.overdrive,
            Row::Backup => s.backup = !s.backup,
            Row::DryRun => s.dry_run = !s.dry_run,
            Row::Quiet => self.quiet = !self.quiet,
            Row::Gpu => {
                let turn_on = s.backend != Backend::Gpu;
                s.backend = if turn_on { Backend::Gpu } else { Backend::Cpu };
                return turn_on;
            }
        }
        false
    }

    pub(super) fn settings_card(&mut self, area: Rect, buf: &mut Buffer) {
        let block = card("SETTINGS");
        let inner = block.inner(area);
        block.render(area, buf);

        let entries = entries();
        let list_height = (inner.height.saturating_sub(1 + TIP_LINES)).max(1) as usize;
        let cursor = entries
            .iter()
            .position(|e| matches!(e, Entry::Setting(i) if *i == self.selected))
            .unwrap_or(0);
        let offset = cursor
            .saturating_sub(list_height.saturating_sub(1))
            .min(entries.len().saturating_sub(list_height));
        let end = (offset + list_height).min(entries.len());

        let parts = Layout::vertical([
            Constraint::Length(list_height as u16),
            Constraint::Length(1),
            Constraint::Length(TIP_LINES),
        ])
        .split(inner);

        let left = inner.x + 2 + LABEL_WIDTH as u16;
        let right = left + CONTROL_WIDTH as u16 - 1;

        let mut lines = Vec::with_capacity(end - offset);
        for (screen_row, entry) in entries[offset..end].iter().enumerate() {
            let y = inner.y + screen_row as u16;
            let &Entry::Setting(i) = entry else {
                let Entry::Heading(group) = entry else {
                    unreachable!()
                };
                let title = group.title();
                let rule = (inner.width as usize).saturating_sub(title.len() + 3);
                lines.push(Line::from(vec![
                    Span::styled(title.to_owned(), Style::new().fg(DIM).bold()),
                    Span::styled(format!("  {}", "─".repeat(rule)), Style::new().fg(RULE)),
                ]));
                continue;
            };

            let row = Row::ALL[i];
            if !row.is_locked(&self.settings) {
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
        self.scroll_markers(parts[0], buf, &entries, offset, end);

        Paragraph::new(Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            Style::new().fg(RULE),
        )))
        .render(parts[1], buf);

        self.tip_block(
            Row::ALL[self.selected.min(Row::ALL.len() - 1)],
            parts[2],
            buf,
        );
    }

    fn tip_block(&self, row: Row, area: Rect, buf: &mut Buffer) {
        let body = (area.width as usize).saturating_sub(2);
        let mut lines: Vec<Line> = wrap(row.tip(), body)
            .into_iter()
            .map(|text| {
                Line::from(Span::styled(
                    format!("  {text}"),
                    Style::new().fg(DIM).italic(),
                ))
            })
            .collect();

        if let Some(warning) = row.warning() {
            for (i, text) in wrap(warning, body).into_iter().enumerate() {
                let marker = if i == 0 { "! " } else { "  " };
                lines.push(Line::from(vec![
                    Span::styled(marker, Style::new().fg(AMBER).bold()),
                    Span::styled(text, Style::new().fg(AMBER)),
                ]));
            }
        }
        lines.truncate(area.height as usize);
        Paragraph::new(lines).render(area, buf);
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
            _ => TEXT,
        };
        let value_colour = match () {
            _ if locked => FAINT,
            _ if row.is_toggle() && !on => FAINT,
            _ if row.is_toggle() => ACCENT,
            _ => TEXT,
        };

        let inner_width = CONTROL_WIDTH - 4;
        let control = clip(&row.control(self), inner_width);
        let hint = row.hint(self);
        let used = 2 + LABEL_WIDTH + CONTROL_WIDTH + 2 + hint.chars().count();
        let (open, close) = if selected && !locked {
            ("‹ ", " ›")
        } else {
            ("  ", "  ")
        };

        Line::from(vec![
            Span::styled(if selected { "▌ " } else { "  " }, base.fg(ACCENT)),
            Span::styled(
                format!("{:<w$}", row.title(), w = LABEL_WIDTH),
                base.fg(label_colour),
            ),
            Span::styled(open, base.fg(ACCENT)),
            Span::styled(
                format!("{control:^inner_width$}"),
                base.fg(value_colour).bold(),
            ),
            Span::styled(close, base.fg(ACCENT)),
            Span::styled(
                format!("  {hint}"),
                base.fg(if locked { FAINT } else { DIM }),
            ),
            Span::styled(" ".repeat(width.saturating_sub(used)), base),
        ])
    }

    #[cfg(test)]
    pub(crate) fn focused_row(&self) -> Row {
        Row::ALL[self.selected]
    }

    #[cfg(test)]
    pub(crate) fn focus(&mut self, row: Row) {
        self.selected = Row::ALL
            .iter()
            .position(|&r| r == row)
            .expect("every row is always present");
    }

    fn scroll_markers(
        &self,
        area: Rect,
        buf: &mut Buffer,
        entries: &[Entry],
        offset: usize,
        end: usize,
    ) {
        let settings_in = |slice: &[Entry]| {
            slice
                .iter()
                .filter(|e| matches!(e, Entry::Setting(_)))
                .count()
        };
        let marker = |n: usize, arrow: &str| {
            Line::from(Span::styled(
                format!(" {arrow} {n} more "),
                Style::new().fg(DIM).bg(CHIP_BG),
            ))
        };

        let above = settings_in(&entries[..offset]);
        if above > 0 {
            Paragraph::new(marker(above, "▲"))
                .alignment(Alignment::Right)
                .render(Rect { height: 1, ..area }, buf);
        }
        let below = settings_in(&entries[end..]);
        if below > 0 {
            Paragraph::new(marker(below, "▼"))
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
    use crate::app::Phase;
    use std::sync::atomic::Ordering;

    const HINT_WIDTH: usize = 25;

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
    fn forward_and_back_land_on_the_same_value() {
        let mut app = app();
        for row in [
            Row::Preset,
            Row::Size,
            Row::Quality,
            Row::Format,
            Row::Safety,
        ] {
            app.focus(row);
            for _ in 0..Preset::ALL.len() + SizeLimit::ALL.len() {
                let before = app.settings.clone();
                app.adjust_setting(true);
                app.adjust_setting(false);
                assert_eq!(app.settings, before, "{} did not come back", row.title());
            }
        }
    }

    #[test]
    fn the_row_list_never_changes_shape() {
        let mut app = app();
        let count = || {
            entries()
                .iter()
                .filter(|e| matches!(e, Entry::Setting(_)))
                .count()
        };
        assert_eq!(count(), Row::ALL.len());
        for preset in Preset::ALL {
            set_preset(&mut app, preset);
            assert_eq!(count(), Row::ALL.len(), "{preset:?} added or removed rows");
        }
    }

    #[test]
    fn a_preset_greys_out_the_rows_it_overrules() {
        let mut app = app();

        set_preset(&mut app, Preset::Hair);
        for row in [
            Row::Overdrive,
            Row::Mipmaps,
            Row::Gpu,
            Row::Format,
            Row::Safety,
        ] {
            assert!(
                row.is_locked(&app.settings),
                "{} should be locked under Hair",
                row.title()
            );
        }
        assert!(
            !Row::Size.is_locked(&app.settings),
            "the size limit is the user's call under every preset"
        );

        set_preset(&mut app, Preset::Clothing);
        assert!(!Row::Format.is_locked(&app.settings));
        assert!(!Row::Safety.is_locked(&app.settings));
        assert!(Row::Mipmaps.is_locked(&app.settings));

        set_preset(&mut app, Preset::Custom);
        assert!(
            Row::Safety.is_locked(&app.settings),
            "Custom drops the family rules, so there is nothing to relax"
        );

        set_preset(&mut app, Preset::Vehicles);
        for row in Row::ALL {
            assert!(
                !row.is_locked(&app.settings),
                "{} should be free under Vehicles & props",
                row.title()
            );
        }
    }

    #[test]
    fn a_locked_row_refuses_to_move() {
        let mut app = app();
        set_preset(&mut app, Preset::Hair);
        let before = app.settings.overdrive;
        app.focus(Row::Overdrive);
        app.adjust_setting(true);
        assert_eq!(app.settings.overdrive, before, "locked row changed anyway");
    }

    #[test]
    fn every_key_is_drawn_inside_the_footer() {
        for width in [60u16, 79, 100, 132] {
            for phase in [Phase::Setup, Phase::Running, Phase::Done] {
                let mut app = app();
                match phase {
                    Phase::Setup => {}
                    Phase::Running => app.running.store(true, Ordering::SeqCst),
                    Phase::Done => app.report.begin(10),
                }
                assert_eq!(app.phase(), phase);

                let chips = app.footer_chips();
                let height = app.footer_height(&chips, width);
                let area = Rect::new(0, 0, width, height);
                let mut buf = Buffer::empty(area);
                app.hits.clear();
                app.footer(area, &mut buf, &chips);

                for (rect, _) in &app.hits {
                    assert!(
                        rect.x + rect.width <= width,
                        "{phase:?} at {width} cols: a key runs to {} past the {width}-col edge",
                        rect.x + rect.width,
                    );
                    assert!(
                        rect.y < height,
                        "{phase:?} at {width} cols: a key is off-screen"
                    );
                }
                assert_eq!(
                    app.hits.len(),
                    chips.iter().filter(|c| c.is_clickable()).count(),
                    "{phase:?} at {width} cols: not every key got drawn"
                );
            }
        }
    }

    #[test]
    fn every_preset_draws_without_panicking() {
        let mut app = app();
        for preset in Preset::ALL {
            set_preset(&mut app, preset);
            for selected in 0..Row::ALL.len() {
                app.selected = selected;
                for height in [10, 24, 40] {
                    let area = Rect::new(0, 0, 79, height);
                    let mut buf = Buffer::empty(area);
                    app.settings_card(area, &mut buf);
                }
            }
        }
    }

    #[test]
    fn the_selected_row_is_always_on_screen() {
        let mut app = app();
        for selected in 0..Row::ALL.len() {
            app.selected = selected;
            let area = Rect::new(0, 0, 79, 14);
            let mut buf = Buffer::empty(area);
            app.hits.clear();
            app.settings_card(area, &mut buf);
            assert!(
                app.hits
                    .iter()
                    .any(|(_, hit)| matches!(hit, Hit::Focus(i) if *i == selected)),
                "{} scrolled out of view",
                Row::ALL[selected].title()
            );
        }
    }

    #[test]
    fn every_label_fits_the_narrow_layout() {
        let mut app = app();
        for preset in Preset::ALL {
            set_preset(&mut app, preset);
            for row in Row::ALL {
                for limit in SizeLimit::ALL {
                    app.settings.size_limit = limit;
                    let control = row.control(&app);
                    assert!(
                        control.chars().count() <= CONTROL_WIDTH - 4,
                        "{} renders `{control}`, which overflows the arrows",
                        row.title()
                    );
                }

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
