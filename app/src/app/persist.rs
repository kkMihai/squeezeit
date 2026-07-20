use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use squeezeit::{ProcessingBackend, Quality, SqueezeSettings};

use super::SqueezeItApp;

pub(super) fn settings_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("settings.yaml"))
}

#[derive(Serialize, Deserialize)]
pub(super) struct ConfigFile {
    #[serde(default)]
    last_folder: Option<PathBuf>,
    #[serde(default)]
    advanced_open: bool,
    #[serde(default)]
    gta_install_path: Option<PathBuf>,
    #[serde(default)]
    settings: ConfigSettings,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct ConfigSettings {
    max_res: u32,
    quality: Quality,
    overdrive: bool,
    generate_mipmaps: bool,
    keep_source_format: bool,
    force_convert: bool,
    make_backup: bool,
    dry_run: bool,
    backend: ProcessingBackend,
    disable_ui_when_processing: bool,
}

impl Default for ConfigSettings {
    fn default() -> Self {
        SqueezeSettings::default().into()
    }
}

impl From<SqueezeSettings> for ConfigSettings {
    fn from(s: SqueezeSettings) -> Self {
        Self {
            max_res: s.max_dimension,
            quality: s.quality,
            overdrive: s.overdrive,
            generate_mipmaps: s.generate_mipmaps,
            keep_source_format: s.keep_source_format,
            force_convert: s.force_convert,
            make_backup: s.make_backup,
            dry_run: s.dry_run,
            backend: s.backend,
            disable_ui_when_processing: s.disable_ui_when_processing,
        }
    }
}

impl From<ConfigSettings> for SqueezeSettings {
    fn from(c: ConfigSettings) -> Self {
        Self {
            max_dimension: c.max_res,
            quality: c.quality,
            overdrive: c.overdrive,
            generate_mipmaps: c.generate_mipmaps,
            keep_source_format: c.keep_source_format,
            force_convert: c.force_convert,
            make_backup: c.make_backup,
            dry_run: c.dry_run,
            backend: c.backend,
            disable_ui_when_processing: c.disable_ui_when_processing,
            gta_install_path: None,
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
                Some(super::Workspace::Folder(folder)) => Some(folder.clone()),
                _ => None,
            },
            advanced_open: self.advanced_open,
            gta_install_path: self.settings.gta_install_path.clone(),
            settings: self.settings.clone().into(),
        };
        let body = serde_yaml::to_string(&config).unwrap_or_default();
        format!("# SqueezeIt Configuration\n{body}")
    }

    pub(super) fn decode_settings(&mut self, encoded: &str) {
        let Ok(config) = serde_yaml::from_str::<ConfigFile>(encoded) else {
            return;
        };
        let mut settings: SqueezeSettings = config.settings.into();
        settings.gta_install_path = config.gta_install_path;
        self.settings = settings;
        self.advanced_open = config.advanced_open;
        if let Some(folder) = config.last_folder
            && folder.is_dir()
        {
            self.workspace = Some(super::Workspace::Folder(folder));
        }
    }
}
