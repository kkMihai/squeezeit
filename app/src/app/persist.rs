use serde::{Deserialize, Serialize};
use squeezeit::{
    Backend, Exclusions, FormatMode, Liveries, Preset, Quality, Safety, ScriptRt, SizeLimit,
    SqueezeSettings,
};
use std::path::PathBuf;

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
    #[serde(alias = "advanced_open")]
    settings_open: bool,
    #[serde(alias = "headless")]
    quiet: bool,
    settings: Knobs,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Knobs {
    preset: Preset,
    max_res: SizeLimit,
    quality: Quality,
    format: FormatMode,
    #[serde(alias = "generate_mipmaps")]
    mipmaps: bool,
    overdrive: bool,
    safety: Safety,
    liveries: Liveries,
    script_rt: ScriptRt,
    exclusions: Exclusions,
    backend: Backend,
    #[serde(alias = "make_backup")]
    backup: bool,
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
            preset: s.preset,
            max_res: s.size_limit,
            quality: s.quality,
            format: s.format,
            mipmaps: s.mipmaps,
            overdrive: s.overdrive,
            safety: s.safety,
            liveries: s.liveries,
            script_rt: s.script_rt,
            exclusions: s.exclusions,
            backend: s.backend,
            backup: s.backup,
            dry_run: s.dry_run,
        }
    }
}

impl From<Knobs> for SqueezeSettings {
    fn from(k: Knobs) -> Self {
        Self {
            preset: k.preset,
            size_limit: k.max_res,
            quality: k.quality,
            format: k.format,
            mipmaps: k.mipmaps,
            overdrive: k.overdrive,
            safety: k.safety,
            liveries: k.liveries,
            script_rt: k.script_rt,
            exclusions: k.exclusions,
            backend: k.backend,
            backup: k.backup,
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
            settings_open: self.settings_open,
            quiet: self.quiet,
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
        self.settings_open = config.settings_open;
        self.quiet = config.quiet;
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
        assert_eq!(knobs.max_res, SqueezeSettings::default().size_limit);
    }

    #[test]
    fn a_config_from_an_older_build_still_loads() {
        let old = "
last_folder: null
gta_exe: null
advanced_open: true
headless: true
settings:
  max_res: 1024
  preset: VehiclesProps
  quality: Slow
  backend: Gpu
  overdrive: true
  generate_mipmaps: false
  keep_source_format: false
  force_convert: false
  cloth_mips: true
  cloth_gpu: false
  vehicle_mips: false
  make_backup: true
  dry_run: false
";
        let config: ConfigFile = serde_yaml::from_str(old).expect("old config parses");
        assert!(config.settings_open);
        assert!(config.quiet);

        let settings: SqueezeSettings = config.settings.into();
        assert_eq!(settings.preset, Preset::Vehicles);
        assert_eq!(settings.size_limit, SizeLimit::Max(1024));
        assert_eq!(settings.quality, Quality::Slow);
        assert_eq!(settings.backend, Backend::Gpu);
        assert!(settings.overdrive);
        assert!(!settings.mipmaps);
        assert!(settings.backup);

        assert_eq!(settings.safety, Safety::Protected);
        assert_eq!(settings.format, FormatMode::Auto);
    }

    #[test]
    fn a_no_resize_limit_round_trips_through_the_file() {
        let knobs = Knobs {
            max_res: SizeLimit::Keep,
            ..Default::default()
        };
        let text = serde_yaml::to_string(&knobs).unwrap();
        let back: Knobs = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back.max_res, SizeLimit::Keep, "wrote: {text}");
    }
}
