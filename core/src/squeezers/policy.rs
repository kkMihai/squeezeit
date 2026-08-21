use crate::gta5::AssetFamily;
use crate::settings::{FormatMode, Liveries, Preset, Quality, SqueezeSettings};
use crate::texture::TextureRole;
pub const HAIR_MIN_SIDE: u32 = 512;
pub const SECONDARY_CAP: u32 = 1024;
const CLOTH_DIFFUSE_CAP: u32 = 1024;
const CLOTH_SECONDARY_CAP: u32 = 512;
const HAIR_CAP: u32 = 1024;
const CLOTH_MIP_MIN_SIDE: u32 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Dictionary,
    Drawable { rebuilt: bool },
}

impl Container {
    pub fn may_grow(self) -> bool {
        match self {
            Container::Dictionary => true,
            Container::Drawable { rebuilt } => rebuilt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipRule {
    Preserve,
    TrimTail,
    GenerateFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatRule {
    Locked,
    Conservative,
    Aggressive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub diffuse_cap: u32,
    pub secondary_cap: u32,
    pub min_side: u32,
    pub allow_resize: bool,
    pub mips: MipRule,
    pub mip_exception: bool,
    pub format: FormatRule,
    pub overdrive: bool,
    pub gpu: bool,
    pub min_quality: Quality,
    pub liveries: Liveries,
}

impl Policy {
    pub fn resolve(settings: &SqueezeSettings, family: AssetFamily) -> Self {
        let family = match (settings.preset.family_override(), family) {
            (Some(AssetFamily::Generic), AssetFamily::Vehicle) => AssetFamily::Vehicle,
            (Some(forced), _) => forced,
            (None, detected) => detected,
        };

        let base = match settings.preset {
            Preset::Custom => Self::custom(settings),
            _ => match family {
                AssetFamily::PedHair => Self::hair(),
                AssetFamily::PedCloth => Self::cloth(settings),
                AssetFamily::Vehicle => Self::vehicle(settings),
                AssetFamily::Generic => Self::world(),
            },
        };
        Policy {
            liveries: settings.liveries,
            ..base.clamp_to(settings)
        }
    }

    fn hair() -> Self {
        Self {
            diffuse_cap: HAIR_CAP,
            secondary_cap: HAIR_CAP,
            min_side: HAIR_MIN_SIDE,
            allow_resize: true,
            mips: MipRule::Preserve,
            mip_exception: false,
            format: FormatRule::Locked,
            overdrive: false,
            gpu: false,
            min_quality: Quality::Normal,
            liveries: Liveries::Protect,
        }
    }

    fn cloth(settings: &SqueezeSettings) -> Self {
        Self {
            diffuse_cap: CLOTH_DIFFUSE_CAP,
            secondary_cap: CLOTH_SECONDARY_CAP,
            min_side: crate::texture::MIN_DIMENSION,
            allow_resize: true,
            mips: MipRule::TrimTail,
            mip_exception: settings.safety.is_relaxed(),
            format: FormatRule::Conservative,
            overdrive: false,
            gpu: true,
            min_quality: Quality::Normal,
            liveries: Liveries::Protect,
        }
    }

    fn vehicle(settings: &SqueezeSettings) -> Self {
        Self {
            mips: if settings.safety.is_relaxed() {
                MipRule::GenerateFull
            } else {
                MipRule::TrimTail
            },
            ..Self::world()
        }
    }

    fn world() -> Self {
        Self {
            diffuse_cap: u32::MAX,
            secondary_cap: SECONDARY_CAP,
            min_side: crate::texture::MIN_DIMENSION,
            allow_resize: true,
            mips: MipRule::GenerateFull,
            mip_exception: false,
            format: FormatRule::Aggressive,
            overdrive: true,
            gpu: true,
            min_quality: Quality::Fast,
            liveries: Liveries::Protect,
        }
    }

    fn custom(settings: &SqueezeSettings) -> Self {
        Self {
            mips: if settings.mipmaps {
                MipRule::GenerateFull
            } else {
                MipRule::TrimTail
            },
            ..Self::world()
        }
    }

    fn clamp_to(self, settings: &SqueezeSettings) -> Self {
        let cap = settings.size_limit.cap().max(crate::texture::MIN_DIMENSION);
        Self {
            diffuse_cap: self.diffuse_cap.min(cap),
            secondary_cap: self.secondary_cap.min(cap).min(SECONDARY_CAP),
            allow_resize: self.allow_resize && settings.size_limit.resizes(),
            overdrive: self.overdrive && settings.overdrive,
            mips: match self.mips {
                MipRule::GenerateFull if !settings.mipmaps => MipRule::TrimTail,
                rule => rule,
            },
            mip_exception: self.mip_exception && settings.mipmaps,
            format: if settings.format == FormatMode::Keep {
                FormatRule::Locked
            } else {
                self.format
            },
            ..self
        }
    }

    pub fn tighten_for_hair(self) -> Self {
        Self {
            min_side: self.min_side.max(HAIR_MIN_SIDE),
            mips: MipRule::Preserve,
            mip_exception: false,
            format: FormatRule::Locked,
            overdrive: false,
            gpu: false,
            min_quality: self.min_quality.at_least(Quality::Normal),
            ..self
        }
    }

    pub fn cap_for(&self, role: TextureRole) -> u32 {
        match role {
            TextureRole::Normal | TextureRole::Specular => self.secondary_cap,
            _ => self.diffuse_cap,
        }
    }

    pub fn quality(&self, settings: &SqueezeSettings) -> Quality {
        settings.quality.at_least(self.min_quality)
    }

    pub fn mip_exception_applies(
        &self,
        opaque: bool,
        format_unchanged: bool,
        source_levels: u32,
        width: u32,
        height: u32,
        container: Container,
    ) -> bool {
        self.mip_exception
            && opaque
            && format_unchanged
            && source_levels <= 1
            && container == Container::Dictionary
            && width == height
            && width.is_power_of_two()
            && width >= CLOTH_MIP_MIN_SIDE
    }
}

impl Preset {
    fn family_override(self) -> Option<AssetFamily> {
        match self {
            Preset::Auto | Preset::Custom => None,
            Preset::Clothing => Some(AssetFamily::PedCloth),
            Preset::Hair => Some(AssetFamily::PedHair),
            Preset::Vehicles => Some(AssetFamily::Generic),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Knob, Safety, SizeLimit};

    fn settings() -> SqueezeSettings {
        SqueezeSettings::default()
    }

    fn policy(family: AssetFamily) -> Policy {
        Policy::resolve(&settings(), family)
    }

    #[test]
    fn hair_is_locked_down() {
        let p = policy(AssetFamily::PedHair);
        assert_eq!(p.mips, MipRule::Preserve);
        assert_eq!(p.format, FormatRule::Locked);
        assert!(!p.gpu);
        assert!(!p.overdrive);
        assert_eq!(p.min_side, HAIR_MIN_SIDE);
        assert!(p.allow_resize);
    }

    #[test]
    fn a_keep_size_limit_stops_every_family_resizing() {
        let s = SqueezeSettings {
            size_limit: SizeLimit::Keep,
            ..settings()
        };
        for family in [
            AssetFamily::Generic,
            AssetFamily::Vehicle,
            AssetFamily::PedCloth,
            AssetFamily::PedHair,
        ] {
            assert!(
                !Policy::resolve(&s, family).allow_resize,
                "{family:?} resized under a `keep` limit"
            );
        }
    }

    #[test]
    fn cloth_resizes_but_keeps_its_mips_and_formats() {
        let p = policy(AssetFamily::PedCloth);
        assert!(p.allow_resize);
        assert_eq!(p.diffuse_cap, CLOTH_DIFFUSE_CAP);
        assert_eq!(p.secondary_cap, CLOTH_SECONDARY_CAP);
        assert_eq!(p.mips, MipRule::TrimTail);
        assert_eq!(p.format, FormatRule::Conservative);
        assert!(
            p.gpu,
            "clothing compresses on the card like everything else"
        );
    }

    #[test]
    fn mipmap_switch_never_reaches_cloth_or_hair() {
        let s = SqueezeSettings {
            mipmaps: true,
            ..settings()
        };
        assert_eq!(
            Policy::resolve(&s, AssetFamily::PedCloth).mips,
            MipRule::TrimTail
        );
        assert_eq!(
            Policy::resolve(&s, AssetFamily::PedHair).mips,
            MipRule::Preserve
        );
        assert_eq!(
            Policy::resolve(&s, AssetFamily::Generic).mips,
            MipRule::GenerateFull
        );
    }

    #[test]
    fn overdrive_switch_never_reaches_cloth_or_hair() {
        let s = SqueezeSettings {
            overdrive: true,
            ..settings()
        };
        assert!(!Policy::resolve(&s, AssetFamily::PedCloth).overdrive);
        assert!(!Policy::resolve(&s, AssetFamily::PedHair).overdrive);
        assert!(Policy::resolve(&s, AssetFamily::Generic).overdrive);
    }

    #[test]
    fn relaxing_safety_only_unlocks_mipmaps() {
        assert_eq!(policy(AssetFamily::Vehicle).mips, MipRule::TrimTail);
        assert!(!policy(AssetFamily::PedCloth).mip_exception);

        let s = SqueezeSettings {
            safety: Safety::Relaxed,
            ..settings()
        };
        assert_eq!(
            Policy::resolve(&s, AssetFamily::Vehicle).mips,
            MipRule::GenerateFull
        );
        assert!(Policy::resolve(&s, AssetFamily::PedCloth).mip_exception);
    }

    #[test]
    fn clothing_uses_the_card_whatever_safety_says() {
        for safety in Safety::ALL {
            let s = SqueezeSettings {
                safety,
                ..settings()
            };
            assert!(Policy::resolve(&s, AssetFamily::PedCloth).gpu, "{safety:?}");
        }
    }

    #[test]
    fn hair_stays_on_the_processor() {
        for safety in Safety::ALL {
            let s = SqueezeSettings {
                safety,
                ..settings()
            };
            let hair = Policy::resolve(&s, AssetFamily::PedHair);
            assert!(!hair.gpu, "{safety:?}");
            assert_eq!(hair.mips, MipRule::Preserve);
            assert!(!hair.mip_exception);
        }
        assert!(!policy(AssetFamily::Generic).tighten_for_hair().gpu);
    }

    #[test]
    fn user_cap_lowers_a_preset_but_never_raises_it() {
        let s = SqueezeSettings {
            size_limit: SizeLimit::Max(512),
            ..settings()
        };
        assert_eq!(Policy::resolve(&s, AssetFamily::PedCloth).diffuse_cap, 512);

        let s = SqueezeSettings {
            size_limit: SizeLimit::Max(4096),
            ..settings()
        };
        assert_eq!(
            Policy::resolve(&s, AssetFamily::PedCloth).diffuse_cap,
            CLOTH_DIFFUSE_CAP
        );
        assert_eq!(Policy::resolve(&s, AssetFamily::Generic).diffuse_cap, 4096);
        assert_eq!(
            Policy::resolve(&s, AssetFamily::Generic).secondary_cap,
            SECONDARY_CAP
        );
    }

    #[test]
    fn forcing_vehicles_and_props_keeps_the_vehicle_rules() {
        let s = SqueezeSettings {
            preset: Preset::Vehicles,
            ..settings()
        };
        assert_eq!(
            Policy::resolve(&s, AssetFamily::Vehicle).mips,
            MipRule::TrimTail
        );
        assert_eq!(
            Policy::resolve(&s, AssetFamily::Generic).mips,
            MipRule::GenerateFull
        );
    }

    #[test]
    fn keeping_the_source_format_locks_every_family() {
        let s = SqueezeSettings {
            format: FormatMode::Keep,
            ..settings()
        };
        for family in [
            AssetFamily::Generic,
            AssetFamily::Vehicle,
            AssetFamily::PedCloth,
        ] {
            assert_eq!(Policy::resolve(&s, family).format, FormatRule::Locked);
        }
    }

    #[test]
    fn forced_presets_ignore_detection() {
        let s = SqueezeSettings {
            preset: Preset::Hair,
            ..settings()
        };
        assert_eq!(
            Policy::resolve(&s, AssetFamily::Vehicle).mips,
            MipRule::Preserve
        );

        let s = SqueezeSettings {
            preset: Preset::Vehicles,
            ..settings()
        };
        assert_eq!(
            Policy::resolve(&s, AssetFamily::PedHair).format,
            FormatRule::Aggressive
        );
    }

    #[test]
    fn custom_preset_keeps_the_old_ungated_behaviour() {
        let s = SqueezeSettings {
            preset: Preset::Custom,
            overdrive: true,
            ..settings()
        };
        let p = Policy::resolve(&s, AssetFamily::PedCloth);
        assert_eq!(p.mips, MipRule::GenerateFull);
        assert_eq!(p.format, FormatRule::Aggressive);
        assert!(p.overdrive);
        assert!(p.gpu);
    }

    #[test]
    fn hair_texture_tightens_a_world_policy() {
        let p = policy(AssetFamily::Generic).tighten_for_hair();
        assert_eq!(p.mips, MipRule::Preserve);
        assert_eq!(p.format, FormatRule::Locked);
        assert!(!p.gpu);
        assert_eq!(p.min_side, HAIR_MIN_SIDE);
    }

    #[test]
    fn cloth_mip_exception_needs_every_condition() {
        let s = SqueezeSettings {
            safety: Safety::Relaxed,
            ..settings()
        };
        let dict = Container::Dictionary;
        let p = Policy::resolve(&s, AssetFamily::PedCloth);
        assert!(p.mip_exception_applies(true, true, 1, 512, 512, dict));

        assert!(!p.mip_exception_applies(false, true, 1, 512, 512, dict));
        assert!(!p.mip_exception_applies(true, false, 1, 512, 512, dict));
        assert!(!p.mip_exception_applies(true, true, 4, 512, 512, dict));
        assert!(!p.mip_exception_applies(true, true, 1, 512, 256, dict));
        assert!(!p.mip_exception_applies(true, true, 1, 64, 64, dict));

        assert!(
            !policy(AssetFamily::PedCloth).mip_exception_applies(true, true, 1, 512, 512, dict)
        );
    }

    #[test]
    fn no_drawable_clothing_texture_ever_grows_a_mip_chain() {
        for safety in Safety::ALL {
            for rebuilt in [false, true] {
                let s = SqueezeSettings {
                    safety,
                    mipmaps: true,
                    ..settings()
                };
                let p = Policy::resolve(&s, AssetFamily::PedCloth);
                assert!(
                    !p.mip_exception_applies(
                        true,
                        true,
                        1,
                        512,
                        512,
                        Container::Drawable { rebuilt }
                    ),
                    "cloth gained mipmaps in a drawable (safety {safety:?}, rebuilt {rebuilt})"
                );
            }
        }
    }

    #[test]
    fn ped_families_never_resolve_to_generating_mipmaps() {
        for preset in [Preset::Auto, Preset::Clothing, Preset::Hair] {
            for safety in Safety::ALL {
                let s = SqueezeSettings {
                    preset,
                    safety,
                    mipmaps: true,
                    overdrive: true,
                    ..settings()
                };
                for family in [AssetFamily::PedCloth, AssetFamily::PedHair] {
                    assert_ne!(
                        Policy::resolve(&s, family).mips,
                        MipRule::GenerateFull,
                        "{preset:?}/{safety:?} would generate mipmaps for {family:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_forced_ped_preset_generates_no_mipmaps_for_anything() {
        for preset in [Preset::Clothing, Preset::Hair] {
            for safety in Safety::ALL {
                let s = SqueezeSettings {
                    preset,
                    safety,
                    mipmaps: true,
                    ..settings()
                };
                for &family in &FAMILIES {
                    assert_ne!(
                        Policy::resolve(&s, family).mips,
                        MipRule::GenerateFull,
                        "{preset:?}/{safety:?} generated mipmaps for a {family:?} file"
                    );
                }
            }
        }
    }

    #[test]
    fn only_a_rebuilt_container_has_room_to_grow() {
        assert!(Container::Dictionary.may_grow());
        assert!(Container::Drawable { rebuilt: true }.may_grow());
        assert!(!Container::Drawable { rebuilt: false }.may_grow());
    }

    const FAMILIES: [AssetFamily; 4] = [
        AssetFamily::Generic,
        AssetFamily::Vehicle,
        AssetFamily::PedCloth,
        AssetFamily::PedHair,
    ];

    #[test]
    fn vetoed_knobs_match_the_resolved_policy() {
        for preset in Preset::ALL {
            let s = SqueezeSettings {
                preset,
                overdrive: true,
                mipmaps: true,
                ..settings()
            };
            let reaches =
                |f: fn(&Policy) -> bool| FAMILIES.iter().any(|&fam| f(&Policy::resolve(&s, fam)));

            assert_eq!(
                preset.allows(Knob::Overdrive),
                reaches(|p| p.overdrive),
                "{preset:?} overdrive"
            );
            assert_eq!(
                preset.allows(Knob::Gpu),
                reaches(|p| p.gpu),
                "{preset:?} gpu"
            );
            assert_eq!(
                preset.allows(Knob::Mipmaps),
                reaches(|p| p.mips == MipRule::GenerateFull),
                "{preset:?} mipmaps"
            );
            assert_eq!(
                preset.allows(Knob::Format),
                reaches(|p| p.format != FormatRule::Locked),
                "{preset:?} format"
            );
        }
    }

    #[test]
    fn a_vetoed_safety_knob_really_changes_nothing() {
        for preset in Preset::ALL {
            let of = |safety| {
                let s = SqueezeSettings {
                    preset,
                    safety,
                    ..settings()
                };
                FAMILIES
                    .iter()
                    .map(|&fam| Policy::resolve(&s, fam))
                    .collect::<Vec<_>>()
            };
            let changes_something = of(Safety::Protected) != of(Safety::Relaxed);
            assert_eq!(
                preset.allows(Knob::Safety),
                changes_something,
                "{preset:?} safety"
            );
        }
    }
}
