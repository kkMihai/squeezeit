use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeLimit {
    Max(u32),
    Keep,
}

impl SizeLimit {
    pub const ALL: [SizeLimit; 6] = [
        SizeLimit::Max(256),
        SizeLimit::Max(512),
        SizeLimit::Max(1024),
        SizeLimit::Max(2048),
        SizeLimit::Max(4096),
        SizeLimit::Keep,
    ];

    pub fn cap(self) -> u32 {
        match self {
            SizeLimit::Max(px) => px,
            SizeLimit::Keep => u32::MAX,
        }
    }

    pub fn resizes(self) -> bool {
        matches!(self, SizeLimit::Max(_))
    }

    pub fn label(self) -> String {
        match self {
            SizeLimit::Max(px) => format!("{px} px"),
            SizeLimit::Keep => "no resize".to_owned(),
        }
    }
}

impl Default for SizeLimit {
    fn default() -> Self {
        SizeLimit::Max(2048)
    }
}

impl fmt::Display for SizeLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SizeLimit::Max(px) => write!(f, "{px}"),
            SizeLimit::Keep => f.write_str("keep"),
        }
    }
}

impl FromStr for SizeLimit {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.eq_ignore_ascii_case("keep") {
            return Ok(SizeLimit::Keep);
        }
        text.parse()
            .map(SizeLimit::Max)
            .map_err(|_| format!("expected a pixel count or `keep`, got `{text}`"))
    }
}

impl Serialize for SizeLimit {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        match self {
            SizeLimit::Max(px) => out.serialize_u32(*px),
            SizeLimit::Keep => out.serialize_str("keep"),
        }
    }
}

impl<'de> Deserialize<'de> for SizeLimit {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        struct Scalar;

        impl Visitor<'_> for Scalar {
            type Value = SizeLimit;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a pixel count or `keep`")
            }

            fn visit_u64<E: de::Error>(self, px: u64) -> Result<SizeLimit, E> {
                Ok(SizeLimit::Max(px.min(u64::from(u32::MAX)) as u32))
            }

            fn visit_i64<E: de::Error>(self, px: i64) -> Result<SizeLimit, E> {
                Ok(SizeLimit::Max(px.clamp(0, i64::from(u32::MAX)) as u32))
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<SizeLimit, E> {
                text.parse().map_err(E::custom)
            }
        }

        input.deserialize_any(Scalar)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FormatMode {
    #[default]
    Auto,
    Keep,
    ForceDds,
}

impl FormatMode {
    pub const ALL: [FormatMode; 3] = [FormatMode::Auto, FormatMode::Keep, FormatMode::ForceDds];

    pub fn label(self) -> &'static str {
        match self {
            FormatMode::Auto => "Auto",
            FormatMode::Keep => "Keep same",
            FormatMode::ForceDds => "Always DDS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Safety {
    #[default]
    Protected,
    Relaxed,
}

impl Safety {
    pub const ALL: [Safety; 2] = [Safety::Protected, Safety::Relaxed];

    pub fn is_relaxed(self) -> bool {
        matches!(self, Safety::Relaxed)
    }

    pub fn label(self) -> &'static str {
        match self {
            Safety::Protected => "Safe",
            Safety::Relaxed => "Risky",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exclusions {
    pub names: Vec<String>,
    pub ignore_case: bool,
}

impl Default for Exclusions {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            ignore_case: true,
        }
    }
}

impl Exclusions {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn excludes(&self, name: &str) -> bool {
        let name = name.trim();
        self.names.iter().any(|listed| {
            let listed = listed.trim();
            if self.ignore_case {
                listed.eq_ignore_ascii_case(name)
            } else {
                listed == name
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScriptRt {
    Lock,
    #[default]
    Repair,
    Only,
}

impl ScriptRt {
    pub const ALL: [ScriptRt; 3] = [ScriptRt::Lock, ScriptRt::Repair, ScriptRt::Only];

    pub fn repairs(self) -> bool {
        matches!(self, ScriptRt::Repair | ScriptRt::Only)
    }

    pub fn label(self) -> &'static str {
        match self {
            ScriptRt::Lock => "Leave alone",
            ScriptRt::Repair => "Fix format",
            ScriptRt::Only => "Fix only these",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Liveries {
    #[default]
    Protect,
    Include,
}

impl Liveries {
    pub const ALL: [Liveries; 2] = [Liveries::Protect, Liveries::Include];

    pub fn protects(self) -> bool {
        matches!(self, Liveries::Protect)
    }

    pub fn label(self) -> &'static str {
        match self {
            Liveries::Protect => "Never resize",
            Liveries::Include => "Resize too",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Preset {
    #[default]
    Auto,
    Clothing,
    #[serde(alias = "HairStrict")]
    Hair,
    #[serde(alias = "VehiclesProps")]
    Vehicles,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knob {
    Mipmaps,
    Overdrive,
    Gpu,
    Format,
    Safety,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::Auto,
        Preset::Clothing,
        Preset::Hair,
        Preset::Vehicles,
        Preset::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Auto => "Auto",
            Preset::Clothing => "Clothing",
            Preset::Hair => "Hair",
            Preset::Vehicles => "Vehicles",
            Preset::Custom => "No rules",
        }
    }

    pub fn allows(self, knob: Knob) -> bool {
        match self {
            Preset::Auto | Preset::Vehicles => true,
            Preset::Custom => knob != Knob::Safety,
            Preset::Clothing => matches!(knob, Knob::Format | Knob::Safety | Knob::Gpu),
            Preset::Hair => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqueezeSettings {
    pub preset: Preset,
    pub size_limit: SizeLimit,
    pub quality: Quality,
    pub format: FormatMode,
    pub mipmaps: bool,
    pub overdrive: bool,
    pub safety: Safety,
    pub liveries: Liveries,
    pub script_rt: ScriptRt,
    pub exclusions: Exclusions,
    pub backend: Backend,
    pub backup: bool,
    pub dry_run: bool,
}

impl Default for SqueezeSettings {
    fn default() -> Self {
        Self {
            preset: Preset::default(),
            size_limit: SizeLimit::default(),
            quality: Quality::default(),
            format: FormatMode::default(),
            mipmaps: true,
            overdrive: false,
            safety: Safety::default(),
            liveries: Liveries::default(),
            script_rt: ScriptRt::default(),
            exclusions: Exclusions::default(),
            backend: Backend::default(),
            backup: false,
            dry_run: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_limit_reads_back_as_it_was_written() {
        for limit in SizeLimit::ALL {
            let text = serde_yaml::to_string(&limit).unwrap();
            let back: SizeLimit = serde_yaml::from_str(&text).unwrap();
            assert_eq!(back, limit, "{limit:?} came back as {back:?} from {text:?}");
        }
    }

    #[test]
    fn a_size_limit_written_by_an_older_build_still_loads() {
        assert_eq!(
            serde_yaml::from_str::<SizeLimit>("2048").unwrap(),
            SizeLimit::Max(2048)
        );
    }

    #[test]
    fn keep_is_spelled_out_rather_than_stored_as_a_number() {
        assert_eq!(
            serde_yaml::to_string(&SizeLimit::Keep).unwrap().trim(),
            "keep"
        );
        assert_eq!("keep".parse::<SizeLimit>().unwrap(), SizeLimit::Keep);
        assert_eq!("1024".parse::<SizeLimit>().unwrap(), SizeLimit::Max(1024));
        assert!("wide".parse::<SizeLimit>().is_err());
    }

    #[test]
    fn a_retired_preset_name_maps_onto_its_replacement() {
        assert_eq!(
            serde_yaml::from_str::<Preset>("HairStrict").unwrap(),
            Preset::Hair
        );
        assert_eq!(
            serde_yaml::from_str::<Preset>("VehiclesProps").unwrap(),
            Preset::Vehicles
        );
    }

    #[test]
    fn an_exclusion_matches_a_whole_name_and_nothing_else() {
        let list = Exclusions {
            names: vec!["  Logo_Main ".into(), "sign_02".into()],
            ignore_case: true,
        };
        assert!(list.excludes("logo_main"));
        assert!(list.excludes("LOGO_MAIN"));
        assert!(list.excludes("sign_02"));

        assert!(!list.excludes("logo"));
        assert!(!list.excludes("logo_main_n"));
        assert!(!list.excludes("my_logo_main"));
        assert!(!Exclusions::default().excludes("anything"));
    }

    #[test]
    fn case_sensitivity_can_be_turned_back_on() {
        let list = Exclusions {
            names: vec!["Logo".into()],
            ignore_case: false,
        };
        assert!(list.excludes("Logo"));
        assert!(!list.excludes("logo"));
    }

    #[test]
    fn only_the_repairing_script_rt_modes_say_so() {
        assert!(!ScriptRt::Lock.repairs());
        assert!(ScriptRt::Repair.repairs());
        assert!(ScriptRt::Only.repairs());
    }

    #[test]
    fn a_default_run_repairs_render_targets() {
        assert!(SqueezeSettings::default().script_rt.repairs());
    }

    #[test]
    fn keep_is_the_only_limit_that_does_not_resize() {
        for limit in SizeLimit::ALL {
            assert_eq!(limit.resizes(), limit != SizeLimit::Keep);
        }
        assert_eq!(SizeLimit::Keep.cap(), u32::MAX);
    }
}
