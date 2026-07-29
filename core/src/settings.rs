use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Quality {
    Fast,
    #[default]
    Normal,
    Slow,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Fast, Quality::Normal, Quality::Slow];

    fn rank(self) -> u8 {
        match self {
            Quality::Fast => 0,
            Quality::Normal => 1,
            Quality::Slow => 2,
        }
    }

    pub fn at_least(self, floor: Quality) -> Quality {
        if self.rank() >= floor.rank() {
            self
        } else {
            floor
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Quality::Fast => "Fast",
            Quality::Normal => "Normal",
            Quality::Slow => "Slow",
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Backend {
    #[default]
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Preset {
    #[default]
    Auto,
    Clothing,
    Hair,
    HairStrict,
    VehiclesProps,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knob {
    Resize,
    Mipmaps,
    Overdrive,
    Gpu,
    Format,
}

impl Preset {
    pub const ALL: [Preset; 6] = [
        Preset::Auto,
        Preset::Clothing,
        Preset::Hair,
        Preset::HairStrict,
        Preset::VehiclesProps,
        Preset::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Auto => "Auto (per asset)",
            Preset::Clothing => "Clothing",
            Preset::Hair => "Hair & alpha",
            Preset::HairStrict => "Hair (strict)",
            Preset::VehiclesProps => "Vehicles & props",
            Preset::Custom => "Custom",
        }
    }

    pub fn allows(self, knob: Knob) -> bool {
        match self {
            Preset::Auto | Preset::VehiclesProps | Preset::Custom => true,
            Preset::Clothing => matches!(knob, Knob::Resize | Knob::Format),
            Preset::Hair => matches!(knob, Knob::Resize),
            Preset::HairStrict => false,
        }
    }

    pub fn touches_cloth(self) -> bool {
        matches!(self, Preset::Auto | Preset::Clothing)
    }

    pub fn touches_vehicles(self) -> bool {
        matches!(self, Preset::Auto | Preset::VehiclesProps)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqueezeSettings {
    pub max_dimension: u32,
    pub preset: Preset,
    pub quality: Quality,
    pub backend: Backend,

    pub overdrive: bool,
    pub generate_mipmaps: bool,
    pub keep_source_format: bool,
    pub force_convert: bool,

    pub cloth_mips: bool,

    pub cloth_gpu: bool,
    pub vehicle_mips: bool,

    pub make_backup: bool,
    pub dry_run: bool,
}

impl Default for SqueezeSettings {
    fn default() -> Self {
        Self {
            max_dimension: 2048,
            preset: Preset::default(),
            quality: Quality::default(),
            backend: Backend::default(),
            overdrive: false,
            generate_mipmaps: true,
            keep_source_format: false,
            force_convert: false,
            cloth_mips: false,
            cloth_gpu: false,
            vehicle_mips: false,
            make_backup: false,
            dry_run: false,
        }
    }
}
