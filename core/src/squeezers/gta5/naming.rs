use std::path::Path;

use super::shaders::AssetFamily;

const HAIR_COMPONENTS: &[&str] = &["hair", "berd"];

const CLOTH_COMPONENTS: &[&str] = &[
    "head", "uppr", "lowr", "hand", "feet", "teef", "accs", "task", "decl", "jbib", "p_head",
    "p_eyes", "p_ears", "p_mouth", "p_lhand", "p_rhand", "p_lwrist", "p_rwrist", "p_hip",
    "p_lfoot", "p_rfoot",
];

pub fn family_from_filename(path: &Path) -> Option<AssetFamily> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())?
        .to_ascii_lowercase();

    if let Some(component) = ped_component(&stem) {
        return Some(if HAIR_COMPONENTS.contains(&component) {
            AssetFamily::PedHair
        } else {
            AssetFamily::PedCloth
        });
    }

    let tail = stem.rsplit('^').next().unwrap_or(&stem);
    let loose_hair =
        tail.starts_with("hair_") || tail.ends_with("_hair") || tail.contains("_hair_");
    loose_hair.then_some(AssetFamily::PedHair)
}

fn ped_component(stem: &str) -> Option<&str> {
    let tail = stem.rsplit('^').next().unwrap_or(stem);
    let known = |c: &&str| HAIR_COMPONENTS.contains(c) || CLOTH_COMPONENTS.contains(c);

    [
        strip_index(strip_variant(tail)),
        strip_index(tail),
        Some(tail),
    ]
    .into_iter()
    .flatten()
    .find(|c| known(c))
}

fn strip_variant(s: &str) -> &str {
    match s.rsplit_once('_') {
        Some((head, tail))
            if (1..=2).contains(&tail.len()) && tail.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            head
        }
        _ => s,
    }
}

fn strip_index(s: &str) -> Option<&str> {
    let (head, tail) = s.rsplit_once('_')?;
    (!tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit())).then_some(head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn family(name: &str) -> Option<AssetFamily> {
        family_from_filename(&PathBuf::from(name))
    }

    #[test]
    fn detects_hair_from_a_real_pack_name() {
        assert_eq!(
            family("mp_f_freemode_01_artix_clothes_civil_03^hair_006_u.ydd"),
            Some(AssetFamily::PedHair)
        );
        assert_eq!(
            family("MP_F_Freemode_01^HAIR_000_U.YDD"),
            Some(AssetFamily::PedHair)
        );
        assert_eq!(
            family("mp_m_freemode_01^berd_012_r.ydd"),
            Some(AssetFamily::PedHair)
        );
    }

    #[test]
    fn detects_clothing_components() {
        for name in [
            "mp_f_freemode_01^uppr_012_r.ydd",
            "mp_m_freemode_01^lowr_003_u.ytd",
            "mp_f_freemode_01^feet_045_u.ydd",
            "mp_f_freemode_01^p_head_001_u.ydd",
            "mp_m_freemode_01^jbib_010_u.ydd",
        ] {
            assert_eq!(family(name), Some(AssetFamily::PedCloth), "{name}");
        }
    }

    #[test]
    fn tolerates_missing_index_and_variant() {
        assert_eq!(family("hair_006.ydd"), Some(AssetFamily::PedHair));
        assert_eq!(family("hair.ydd"), Some(AssetFamily::PedHair));
        assert_eq!(family("ped^uppr.ydd"), Some(AssetFamily::PedCloth));
        assert_eq!(family("ped^hair_diff_000.ydd"), Some(AssetFamily::PedHair));
    }

    #[test]
    fn leaves_everything_else_undetected() {
        for name in [
            "adder.ytd",
            "adder_hi.yft",
            "prop_bench_01.ydr",
            "v_res_fh_hairbrsh.ydr",
            "w_ar_assaultrifle.ydr",
            "vehicles.rpf",
        ] {
            assert_eq!(family(name), None, "{name}");
        }
    }
}
