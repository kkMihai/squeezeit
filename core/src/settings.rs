#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Quality {
    Fast,
    #[default]
    Normal,
    Slow,
}

impl From<Quality> for image_dds::Quality {
    fn from(q: Quality) -> Self {
        match q {
            Quality::Fast => image_dds::Quality::Fast,
            Quality::Normal => image_dds::Quality::Normal,
            Quality::Slow => image_dds::Quality::Slow,
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ProcessingBackend {
    #[default]
    CPU,
    GPU,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqueezeSettings {
    pub max_dimension: u32,

    pub overdrive: bool,

    pub generate_mipmaps: bool,

    pub keep_source_format: bool,

    pub force_convert: bool,

    pub quality: Quality,

    pub make_backup: bool,

    pub dry_run: bool,

    pub backend: ProcessingBackend,

    pub disable_ui_when_processing: bool,

    pub gta_install_path: Option<std::path::PathBuf>,
}

impl Default for SqueezeSettings {
    fn default() -> Self {
        Self {
            max_dimension: 2048,
            overdrive: false,

            generate_mipmaps: true,
            keep_source_format: false,
            force_convert: false,
            quality: Quality::Normal,
            make_backup: false,
            dry_run: false,
            backend: ProcessingBackend::default(),
            disable_ui_when_processing: false,
            gta_install_path: None,
        }
    }
}
