use super::raw::{system_offset, u16_at, u32_at, u64_at};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssetFamily {
    Vehicle,
    PedCloth,
    Generic,
}

impl AssetFamily {
    pub(crate) fn allows_mip_generation(self) -> bool {
        matches!(self, AssetFamily::Generic)
    }
}

const fn joaat(s: &[u8]) -> u32 {
    let mut h: u32 = 0;
    let mut i = 0;
    while i < s.len() {
        h = h.wrapping_add(s[i] as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
        i += 1;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

const VEHICLE_SHADERS: &[u32] = &[
    joaat(b"vehicle_paint1"),
    joaat(b"vehicle_paint1_enveff"),
    joaat(b"vehicle_paint2"),
    joaat(b"vehicle_paint2_enveff"),
    joaat(b"vehicle_paint3"),
    joaat(b"vehicle_paint3_enveff"),
    joaat(b"vehicle_paint3_lvr"),
    joaat(b"vehicle_paint4"),
    joaat(b"vehicle_paint4_enveff"),
    joaat(b"vehicle_paint4_emissive"),
    joaat(b"vehicle_paint5_enveff"),
    joaat(b"vehicle_paint6"),
    joaat(b"vehicle_paint6_enveff"),
    joaat(b"vehicle_paint7"),
    joaat(b"vehicle_paint7_enveff"),
    joaat(b"vehicle_paint8"),
    joaat(b"vehicle_paint9"),
    joaat(b"vehicle_mesh"),
    joaat(b"vehicle_mesh_enveff"),
    joaat(b"vehicle_mesh2_enveff"),
    joaat(b"vehicle_detail"),
    joaat(b"vehicle_detail2"),
    joaat(b"vehicle_badges"),
    joaat(b"vehicle_decal"),
    joaat(b"vehicle_decal2"),
    joaat(b"vehicle_generic"),
    joaat(b"vehicle_interior"),
    joaat(b"vehicle_interior2"),
    joaat(b"vehicle_basic"),
    joaat(b"vehicle_blurredrotor"),
    joaat(b"vehicle_blurredrotor_emissive"),
    joaat(b"vehicle_chrome"),
    joaat(b"vehicle_cloth"),
    joaat(b"vehicle_cloth2"),
    joaat(b"vehicle_cutout"),
    joaat(b"vehicle_disc"),
    joaat(b"vehicle_emissive_opaque"),
    joaat(b"vehicle_emissive_alpha"),
    joaat(b"vehicle_licenseplate"),
    joaat(b"vehicle_lights"),
    joaat(b"vehicle_lightsemissive"),
    joaat(b"vehicle_dash_emissive"),
    joaat(b"vehicle_dash_emissive_opaque"),
    joaat(b"vehicle_shts_emissive"),
    joaat(b"vehicle_shts_emissive_opaque"),
    joaat(b"vehicle_vehglass"),
    joaat(b"vehicle_vehglass_inner"),
    joaat(b"vehicle_track"),
    joaat(b"vehicle_track2"),
    joaat(b"vehicle_track_ammo"),
    joaat(b"vehicle_track_emissive"),
    joaat(b"vehicle_tire"),
    joaat(b"vehicle_tire_emissive"),
];

const PED_CLOTH_SHADERS: &[u32] = &[
    joaat(b"ped"),
    joaat(b"ped_alpha"),
    joaat(b"ped_cloth"),
    joaat(b"ped_cloth_enveff"),
    joaat(b"ped_decal"),
    joaat(b"ped_decal_decoration"),
    joaat(b"ped_decal_nodiff"),
    joaat(b"ped_default"),
    joaat(b"ped_default_cloth"),
    joaat(b"ped_default_enveff"),
    joaat(b"ped_default_mp"),
    joaat(b"ped_default_palette"),
    joaat(b"ped_emissive"),
    joaat(b"ped_enveff"),
    joaat(b"ped_fur"),
    joaat(b"ped_hair_cutout_alpha"),
    joaat(b"ped_hair_spiked"),
    joaat(b"ped_nopeddamagedecals"),
    joaat(b"ped_palette"),
    joaat(b"ped_wrinkle"),
    joaat(b"ped_wrinkle_cloth"),
    joaat(b"ped_wrinkle_cloth_enveff"),
    joaat(b"ped_wrinkle_enveff"),
    joaat(b"cloth_default"),
    joaat(b"cloth_normal_spec"),
    joaat(b"cloth_normal_spec_alpha"),
    joaat(b"cloth_normal_spec_cutout"),
    joaat(b"cloth_spec_alpha"),
];

fn family_of_shader(name_hash: u32) -> Option<AssetFamily> {
    if VEHICLE_SHADERS.contains(&name_hash) {
        Some(AssetFamily::Vehicle)
    } else if PED_CLOTH_SHADERS.contains(&name_hash) {
        Some(AssetFamily::PedCloth)
    } else {
        None
    }
}

fn shader_name_hashes(sys: &[u8], draw_off: usize) -> Vec<u32> {
    let group = match u64_at(sys, draw_off + 0x10).and_then(|p| system_offset(p, sys.len())) {
        Some(g) => g,
        None => return Vec::new(),
    };
    let Some(list) = u64_at(sys, group + 0x10).and_then(|p| system_offset(p, sys.len())) else {
        return Vec::new();
    };
    let count = u16_at(sys, group + 0x18).unwrap_or(0) as usize;

    (0..count.min(1024))
        .filter_map(|i| {
            let shader = u64_at(sys, list + i * 8).and_then(|p| system_offset(p, sys.len()))?;
            u32_at(sys, shader + 0x08)
        })
        .collect()
}

pub(crate) fn classify_drawables(sys: &[u8], draw_offsets: &[usize]) -> AssetFamily {
    let mut ped_cloth = false;
    for &draw in draw_offsets {
        for hash in shader_name_hashes(sys, draw) {
            match family_of_shader(hash) {
                Some(AssetFamily::Vehicle) => return AssetFamily::Vehicle,
                Some(AssetFamily::PedCloth) => ped_cloth = true,
                _ => {}
            }
        }
    }
    if ped_cloth {
        AssetFamily::PedCloth
    } else {
        AssetFamily::Generic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joaat_matches_known_hashes() {
        assert_eq!(joaat(b"vehicle_paint1"), 0xf9fb_7331);
        assert_eq!(joaat(b"vehicle_mesh"), 0xd963_b58b);
        assert_eq!(joaat(b"vehicle_tire"), 0x1d5f_09ce);
        assert_eq!(joaat(b"vehicle_vehglass"), 0x7c98_d207);
    }

    #[test]
    fn families_resolve() {
        assert_eq!(
            family_of_shader(joaat(b"vehicle_mesh")),
            Some(AssetFamily::Vehicle)
        );
        assert_eq!(
            family_of_shader(joaat(b"ped_default")),
            Some(AssetFamily::PedCloth)
        );
        assert_eq!(
            family_of_shader(joaat(b"cloth_normal_spec")),
            Some(AssetFamily::PedCloth)
        );
        assert_eq!(family_of_shader(joaat(b"default")), None);
    }

    #[test]
    fn generic_allows_mips_others_dont() {
        assert!(AssetFamily::Generic.allows_mip_generation());
        assert!(!AssetFamily::Vehicle.allows_mip_generation());
        assert!(!AssetFamily::PedCloth.allows_mip_generation());
    }
}
