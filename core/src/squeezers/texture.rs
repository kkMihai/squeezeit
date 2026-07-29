use std::path::Path;

use crate::policy::Policy;

pub const MIN_DIMENSION: u32 = 4;

const MAX_SAMPLES: u32 = 1024;

const NORMAL_BLUE_MIN: u8 = 180;
const NORMAL_XY_MID: i16 = 128;
const NORMAL_XY_TOLERANCE: i16 = 38;
const NORMAL_MATCH_RATIO: f32 = 0.85;

const GRAY_MAX_VARIANCE: u8 = 15;
const GRAY_MATCH_RATIO: f32 = 0.90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureRole {
    Diffuse,
    Normal,
    Specular,
    Hair,
    ScriptRenderTarget,
    Livery,
    Weapon,
}

const ROLE_SUFFIXES: &[&str] = &[
    "n",
    "nm",
    "nrm",
    "normal",
    "bump",
    "s",
    "spec",
    "specular",
    "rough",
    "roughness",
    "metallic",
    "metal",
    "ao",
    "d",
    "diff",
    "diffuse",
];

fn is_hair_name(stem: &str) -> bool {
    stem.split(['_', '+', '^', '-', '.'])
        .any(|token| matches!(token, "hair" | "hairs" | "fur"))
}

fn is_weapon_model_prefix(stem: &str) -> bool {
    let b = stem.as_bytes();
    b.len() >= 5
        && b[0] == b'w'
        && b[1] == b'_'
        && b[2].is_ascii_alphabetic()
        && b[3].is_ascii_alphabetic()
        && b[4] == b'_'
}

pub fn classify_name(path: &Path) -> Option<TextureRole> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if stem.starts_with("script_rt") {
        return Some(TextureRole::ScriptRenderTarget);
    }
    if stem.contains("livery")
        || stem.contains("_lvr")
        || stem.contains("_sign_")
        || stem.ends_with("_sign")
    {
        return Some(TextureRole::Livery);
    }
    if stem.contains("weapon") || is_weapon_model_prefix(&stem) {
        return Some(TextureRole::Weapon);
    }
    if is_hair_name(&stem) {
        return Some(TextureRole::Hair);
    }

    let stem = stem.split('+').next().unwrap_or(&stem);
    match stem.rsplit('_').next().unwrap_or_default() {
        "n" | "nm" | "nrm" | "normal" | "bump" => Some(TextureRole::Normal),
        "s" | "spec" | "specular" | "rough" | "roughness" | "metallic" | "metal" | "ao" => {
            Some(TextureRole::Specular)
        }
        _ => None,
    }
}

#[inline(always)]
pub fn classify_pixels(buf: &[u8], width: u32, height: u32) -> Option<TextureRole> {
    if width == 0 || height == 0 {
        return None;
    }

    let grid = |dim: u32| dim.min(MAX_SAMPLES.isqrt());
    let (gx, gy) = (grid(width), grid(height));

    let mut normal_hits = 0u32;
    let mut gray_hits = 0u32;

    let step_x = width / gx;
    let step_y = height / gy;

    for j in 0..gy {
        let y_offset = j * step_y * width;
        for i in 0..gx {
            let idx = ((y_offset + i * step_x) as usize) * 4;
            if idx + 3 >= buf.len() {
                continue;
            }
            let [r, g, b, _] = [buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3]];

            if b >= NORMAL_BLUE_MIN
                && (r as i16 - NORMAL_XY_MID).abs() <= NORMAL_XY_TOLERANCE
                && (g as i16 - NORMAL_XY_MID).abs() <= NORMAL_XY_TOLERANCE
            {
                normal_hits += 1;
            }
            if r.max(g).max(b) - r.min(g).min(b) <= GRAY_MAX_VARIANCE {
                gray_hits += 1;
            }
        }
    }

    let total = (gx * gy) as f32;
    if normal_hits as f32 >= total * NORMAL_MATCH_RATIO {
        Some(TextureRole::Normal)
    } else if gray_hits as f32 >= total * GRAY_MATCH_RATIO {
        Some(TextureRole::Specular)
    } else {
        None
    }
}

pub fn pair_base(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stem = stem.split('+').next().unwrap_or(&stem);

    match stem.rsplit_once('_') {
        Some((base, suffix)) if ROLE_SUFFIXES.contains(&suffix) => base.to_owned(),
        _ => stem.to_owned(),
    }
}

pub fn cap_dimensions(mut w: u32, mut h: u32, cap: u32) -> (u32, u32) {
    let cap = cap.max(MIN_DIMENSION);
    while w.max(h) > cap {
        w = (w / 2).max(MIN_DIMENSION);
        h = (h / 2).max(MIN_DIMENSION);
    }
    (w, h)
}

pub fn nearest_power_of_two(n: u32) -> u32 {
    if n <= MIN_DIMENSION {
        return MIN_DIMENSION;
    }
    if n.is_power_of_two() {
        return n;
    }
    let hi = n.next_power_of_two();
    let lo = hi >> 1;
    if n - lo < hi - n { lo } else { hi }
}

pub fn target_dimensions(
    width: u32,
    height: u32,
    role: TextureRole,
    policy: &Policy,
) -> (u32, u32) {
    if !policy.allow_resize {
        if width.is_multiple_of(4) && height.is_multiple_of(4) {
            return (width, height);
        }
        return (nearest_power_of_two(width), nearest_power_of_two(height));
    }

    let w = nearest_power_of_two(width);
    let h = nearest_power_of_two(height);

    let mut cap = policy.cap_for(role);
    if policy.overdrive && matches!(role, TextureRole::Normal | TextureRole::Specular) {
        cap /= 2;
    }
    cap = cap.max(policy.min_side).max(MIN_DIMENSION);

    if matches!(role, TextureRole::Livery | TextureRole::Weapon) {
        cap = cap.max(w.max(h));
    }

    cap_dimensions(w, h, cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gta5::AssetFamily;
    use crate::settings::SqueezeSettings;
    use image::RgbaImage;
    use std::path::PathBuf;

    fn role(name: &str) -> TextureRole {
        classify_name(&PathBuf::from(name)).unwrap_or(TextureRole::Diffuse)
    }

    fn policy(settings: &SqueezeSettings, family: AssetFamily) -> Policy {
        Policy::resolve(settings, family)
    }

    #[test]
    fn classifies_script_render_targets() {
        assert_eq!(
            role("script_rt_dashboard.dds"),
            TextureRole::ScriptRenderTarget
        );
        assert_eq!(
            role("SCRIPT_RT_tvscreen.png"),
            TextureRole::ScriptRenderTarget
        );
    }

    #[test]
    fn classifies_liveries_and_signs() {
        assert_eq!(role("police_livery_01.png"), TextureRole::Livery);
        assert_eq!(role("police_lvr_01.png"), TextureRole::Livery);
        assert_eq!(role("shop_sign_neon.dds"), TextureRole::Livery);
        assert_eq!(role("billboard_sign_3.tga"), TextureRole::Livery);
    }

    #[test]
    fn classifies_weapon_textures() {
        assert_eq!(role("weapon_pistol_diff.png"), TextureRole::Weapon);
        assert_eq!(role("custom_weapon_skin.dds"), TextureRole::Weapon);

        for name in [
            "w_me_pistol1.dds",
            "w_ar_assaultrifle.dds",
            "w_pi_combatpistol.dds",
            "w_sg_pumpshotgun.dds",
            "w_sm_microsmg.dds",
            "w_sr_sniperrifle.dds",
            "w_mg_minigun.dds",
            "w_lr_rpg.dds",
            "w_ex_grenade.dds",
            "w_sb_switchblade.dds",
        ] {
            assert_eq!(role(name), TextureRole::Weapon, "{name}");
        }

        assert_ne!(role("w_texture.dds"), TextureRole::Weapon);
        assert_ne!(role("wall_brick.dds"), TextureRole::Weapon);
    }

    #[test]
    fn classifies_hair_and_fur() {
        assert_eq!(role("hair_diff_000_a_uni.dds"), TextureRole::Hair);
        assert_eq!(role("HAIR_NORM.dds"), TextureRole::Hair);
        assert_eq!(role("mp_m_hair_012_spec.dds"), TextureRole::Hair);
        assert_eq!(role("ped_fur_01.dds"), TextureRole::Hair);

        assert_eq!(role("furniture_wood.dds"), TextureRole::Diffuse);
        assert_eq!(role("hairbrush.dds"), TextureRole::Diffuse);
    }

    #[test]
    fn classifies_secondary_maps() {
        assert_eq!(role("vehicle_generic_n.dds"), TextureRole::Normal);
        assert_eq!(role("brick_wall_normal.png"), TextureRole::Normal);
        assert_eq!(role("ped_torso_s.dds"), TextureRole::Specular);
        assert_eq!(role("car_paint_spec.png"), TextureRole::Specular);
        assert_eq!(role("prop_crate_n+hidef.dds"), TextureRole::Normal);
    }

    #[test]
    fn defaults_to_diffuse() {
        assert_eq!(role("building_facade.png"), TextureRole::Diffuse);
        assert_eq!(role("nightclub.dds"), TextureRole::Diffuse);
    }

    #[test]
    fn name_classifier_reports_ambiguity() {
        assert_eq!(classify_name(&PathBuf::from("unnamed_texture_5.png")), None);
        assert_eq!(
            classify_name(&PathBuf::from("wall_n.dds")),
            Some(TextureRole::Normal)
        );
        assert_eq!(
            classify_name(&PathBuf::from("wall_roughness.png")),
            Some(TextureRole::Specular)
        );
    }

    fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(px))
    }

    #[test]
    fn pixels_detect_flat_normal_map() {
        let img = solid(256, 256, [128, 128, 255, 255]);
        assert_eq!(
            classify_pixels(img.as_raw(), img.width(), img.height()),
            Some(TextureRole::Normal)
        );
    }

    #[test]
    fn pixels_detect_bumpy_normal_map() {
        let mut img = solid(64, 64, [128, 128, 255, 255]);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let wobble = ((x + y) % 60) as u8;
            p.0 = [98 + wobble, 166 - wobble, 200 + wobble / 2, 255];
        }
        assert_eq!(
            classify_pixels(img.as_raw(), img.width(), img.height()),
            Some(TextureRole::Normal)
        );
    }

    #[test]
    fn pixels_detect_grayscale_mask() {
        let mut img = solid(128, 128, [0, 0, 0, 255]);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let v = ((x * 2 + y) % 256) as u8;
            p.0 = [v, v.saturating_add(5), v.saturating_sub(5), 255];
        }
        assert_eq!(
            classify_pixels(img.as_raw(), img.width(), img.height()),
            Some(TextureRole::Specular)
        );
    }

    #[test]
    fn pixels_leave_colored_diffuse_alone() {
        let mut img = solid(64, 64, [0, 0, 0, 255]);
        for (x, y, p) in img.enumerate_pixels_mut() {
            p.0 = [(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255];
        }
        assert_eq!(
            classify_pixels(img.as_raw(), img.width(), img.height()),
            None
        );
    }

    #[test]
    fn pixels_reject_blue_but_not_normal() {
        let img = solid(64, 64, [40, 60, 230, 255]);
        assert_eq!(
            classify_pixels(img.as_raw(), img.width(), img.height()),
            None
        );
    }

    #[test]
    fn pixels_handle_tiny_textures() {
        let img = solid(1, 1, [128, 128, 255, 255]);
        assert_eq!(
            classify_pixels(img.as_raw(), img.width(), img.height()),
            Some(TextureRole::Normal)
        );
    }

    #[test]
    fn snaps_to_nearest_power_of_two() {
        assert_eq!(nearest_power_of_two(1000), 1024);
        assert_eq!(nearest_power_of_two(1500), 1024);
        assert_eq!(nearest_power_of_two(1600), 2048);
        assert_eq!(nearest_power_of_two(640), 512);
        assert_eq!(nearest_power_of_two(2048), 2048);
        assert_eq!(nearest_power_of_two(1), MIN_DIMENSION);
    }

    #[test]
    fn caps_preserve_aspect_ratio() {
        let s = SqueezeSettings {
            max_dimension: 1024,
            ..Default::default()
        };
        assert_eq!(
            target_dimensions(
                4096,
                2048,
                TextureRole::Diffuse,
                &policy(&s, AssetFamily::Generic)
            ),
            (1024, 512)
        );
    }

    #[test]
    fn overdrive_halves_secondary_maps_only() {
        let s = SqueezeSettings {
            max_dimension: 2048,
            overdrive: true,
            ..Default::default()
        };
        let p = policy(&s, AssetFamily::Generic);
        assert_eq!(
            target_dimensions(2048, 2048, TextureRole::Normal, &p),
            (512, 512)
        );
        assert_eq!(
            target_dimensions(2048, 2048, TextureRole::Diffuse, &p),
            (2048, 2048)
        );
    }

    #[test]
    fn clothing_is_resized_within_its_own_cap() {
        let s = SqueezeSettings::default();
        let p = policy(&s, AssetFamily::PedCloth);
        assert_eq!(
            target_dimensions(2048, 2048, TextureRole::Diffuse, &p),
            (1024, 1024)
        );
        assert_eq!(
            target_dimensions(2048, 2048, TextureRole::Normal, &p),
            (512, 512)
        );
        assert_eq!(
            target_dimensions(512, 512, TextureRole::Diffuse, &p),
            (512, 512)
        );
    }

    #[test]
    fn hair_never_goes_below_its_floor() {
        let s = SqueezeSettings {
            max_dimension: 256,
            ..Default::default()
        };
        let p = policy(&s, AssetFamily::PedHair);
        assert_eq!(
            target_dimensions(2048, 2048, TextureRole::Diffuse, &p),
            (512, 512)
        );
        assert_eq!(
            target_dimensions(256, 256, TextureRole::Diffuse, &p),
            (256, 256)
        );
    }

    #[test]
    fn strict_hair_keeps_source_dimensions() {
        let s = SqueezeSettings {
            preset: crate::settings::Preset::HairStrict,
            max_dimension: 256,
            ..Default::default()
        };
        assert_eq!(
            target_dimensions(
                2048,
                2048,
                TextureRole::Diffuse,
                &policy(&s, AssetFamily::PedHair)
            ),
            (2048, 2048)
        );
    }

    #[test]
    fn liveries_and_weapons_never_downscale() {
        let s = SqueezeSettings {
            max_dimension: 512,
            ..Default::default()
        };
        let p = policy(&s, AssetFamily::Generic);
        assert_eq!(
            target_dimensions(4096, 4096, TextureRole::Livery, &p),
            (4096, 4096)
        );
        assert_eq!(
            target_dimensions(4096, 2048, TextureRole::Weapon, &p),
            (4096, 2048)
        );
    }

    #[test]
    fn pair_base_strips_role_suffixes() {
        let base = |name: &str| pair_base(&PathBuf::from(name));
        assert_eq!(base("car_paint_d.dds"), "car_paint");
        assert_eq!(base("car_paint_n.dds"), "car_paint");
        assert_eq!(base("car_paint_s.dds"), "car_paint");
        assert_eq!(base("CAR_PAINT.dds"), "car_paint");
        assert_eq!(base("prop_crate_n+hidef.dds"), "prop_crate");
        assert_eq!(base("brick_wall_normal.png"), "brick_wall");
        assert_eq!(base("no_suffix_here.png"), "no_suffix_here");
    }

    #[test]
    fn cap_dimensions_halves_preserving_aspect() {
        assert_eq!(cap_dimensions(2048, 1024, 1024), (1024, 512));
        assert_eq!(cap_dimensions(512, 512, 1024), (512, 512));
        assert_eq!(
            cap_dimensions(4096, 4096, 1),
            (MIN_DIMENSION, MIN_DIMENSION)
        );
    }
}
