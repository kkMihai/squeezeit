use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use squeezeit::{Backend, Preset, Quality, SqueezeSettings};

use super::{SqueezeItApp, Workspace};

const HEADER: &str = "# SqueezeIt configuration\n";

pub(super) fn settings_path() -> Option<PathBuf> {
    Some(
        std::env::current_exe()
            .ok()?
            .parent()?
            .join("settings.yaml"),
    )
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct ConfigFile {
    last_folder: Option<PathBuf>,
    gta_exe: Option<PathBuf>,
    advanced_open: bool,
    headless: bool,
    settings: Knobs,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Knobs {
    max_res: u32,
    preset: Preset,
    quality: Quality,
    backend: Backend,
    overdrive: bool,
    generate_mipmaps: bool,
    keep_source_format: bool,
    force_convert: bool,
    cloth_mips: bool,
    cloth_gpu: bool,
    vehicle_mips: bool,
    make_backup: bool,
    dry_run: bool,
}

impl Default for Knobs {
    fn default() -> Self {
        SqueezeSettings::default().into()
    }
}

impl From<SqueezeSettings> for Knobs {
    fn from(s: SqueezeSettings) -> Self {
        Self {
            max_res: s.max_dimension,
            preset: s.preset,
            quality: s.quality,
            backend: s.backend,
            overdrive: s.overdrive,
            generate_mipmaps: s.generate_mipmaps,
            keep_source_format: s.keep_source_format,
            force_convert: s.force_convert,
            cloth_mips: s.cloth_mips,
            cloth_gpu: s.cloth_gpu,
            vehicle_mips: s.vehicle_mips,
            make_backup: s.make_backup,
            dry_run: s.dry_run,
        }
    }
}

impl From<Knobs> for SqueezeSettings {
    fn from(k: Knobs) -> Self {
        Self {
            max_dimension: k.max_res,
            preset: k.preset,
            quality: k.quality,
            backend: k.backend,
            overdrive: k.overdrive,
            generate_mipmaps: k.generate_mipmaps,
            keep_source_format: k.keep_source_format,
            force_convert: k.force_convert,
            cloth_mips: k.cloth_mips,
            cloth_gpu: k.cloth_gpu,
            vehicle_mips: k.vehicle_mips,
            make_backup: k.make_backup,
            dry_run: k.dry_run,
        }
    }
}

impl SqueezeItApp {
    pub(super) fn save_if_changed(&mut self) {
        let encoded = self.encode_settings();
        if encoded == self.last_saved {
            return;
        }
        if let Some(path) = settings_path()
            && std::fs::write(&path, &encoded).is_ok()
        {
            self.last_saved = encoded;
        }
    }

    pub(super) fn encode_settings(&self) -> String {
        let config = ConfigFile {
            last_folder: match &self.workspace {
                Some(Workspace::Folder(folder)) => Some(folder.clone()),
                _ => None,
            },
            gta_exe: self.gta_exe.clone(),
            advanced_open: self.advanced_open,
            headless: self.headless,
            settings: self.settings.clone().into(),
        };
        format!(
            "{HEADER}{}",
            serde_yaml::to_string(&config).unwrap_or_default()
        )
    }

    pub(super) fn decode_settings(&mut self, encoded: &str) {
        let Ok(config) = serde_yaml::from_str::<ConfigFile>(encoded) else {
            return;
        };
        self.settings = config.settings.into();
        self.gta_exe = config.gta_exe;
        self.advanced_open = config.advanced_open;
        self.headless = config.headless;
        if let Some(folder) = config.last_folder
            && folder.is_dir()
        {
            self.workspace = Some(Workspace::Folder(folder));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_through_yaml() {
        for preset in Preset::ALL {
            let knobs = Knobs {
                preset,
                ..Default::default()
            };
            let text = serde_yaml::to_string(&knobs).unwrap();
            let back: Knobs = serde_yaml::from_str(&text).unwrap();
            assert_eq!(
                back.preset, preset,
                "{preset:?} came back as {:?}",
                back.preset
            );
        }
    }

    #[test]
    fn a_missing_field_falls_back_to_the_default() {
        let knobs: Knobs = serde_yaml::from_str("preset: Clothing").unwrap();
        assert_eq!(knobs.preset, Preset::Clothing);
        assert_eq!(knobs.max_res, SqueezeSettings::default().max_dimension);
    }
}
