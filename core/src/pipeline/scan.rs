use crate::gta5::{self, AssetFamily};
use crate::policy::Policy;
use crate::settings::SqueezeSettings;
use crate::texture::{TextureRole, classify_name, target_dimensions};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TextureScan {
    pub name: String,
    pub role: Option<TextureRole>,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub levels: u32,
    pub resize_to: Option<(u32, u32)>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Resize,
    AlreadyFits,
    Excluded,
    RenderTarget,
    Protected,
}

#[derive(Debug, Clone)]
pub struct FileScan {
    pub path: PathBuf,
    pub family: AssetFamily,
    pub reserved: u64,
    pub textures: Vec<TextureScan>,
}

impl FileScan {
    pub fn resizing(&self) -> usize {
        self.textures
            .iter()
            .filter(|t| t.verdict == Verdict::Resize)
            .count()
    }
}

pub fn scan_bytes(path: &Path, bytes: &[u8], settings: &SqueezeSettings) -> Option<FileScan> {
    let info = gta5::inspect_bytes(path, bytes).ok()?;
    let family = gta5::resolve_family(info.family, gta5::family_from_filename(path));
    let base = Policy::resolve(settings, family);

    let textures = info
        .textures
        .iter()
        .map(|tex| {
            let role = classify_name(Path::new(&tex.name));
            let policy = if role == Some(TextureRole::Hair) {
                base.tighten_for_hair()
            } else {
                base
            };
            let effective = role.unwrap_or(TextureRole::Diffuse);
            let target = target_dimensions(tex.width, tex.height, effective, &policy);

            let verdict = if role == Some(TextureRole::ScriptRenderTarget) {
                Verdict::RenderTarget
            } else if settings.exclusions.excludes(&tex.name) {
                Verdict::Excluded
            } else if matches!(effective, TextureRole::Livery | TextureRole::Weapon)
                && policy.liveries.protects()
            {
                Verdict::Protected
            } else if target == (tex.width, tex.height) {
                Verdict::AlreadyFits
            } else {
                Verdict::Resize
            };

            TextureScan {
                name: tex.name.clone(),
                role,
                width: tex.width,
                height: tex.height,
                format: tex.format,
                levels: tex.levels,
                resize_to: (verdict == Verdict::Resize).then_some(target),
                verdict,
            }
        })
        .collect();

    Some(FileScan {
        path: path.to_path_buf(),
        family,
        reserved: info.graphics_bytes,
        textures,
    })
}

pub fn scan(targets: &[PathBuf], settings: &SqueezeSettings) -> Vec<FileScan> {
    targets
        .iter()
        .filter_map(|path| {
            let bytes = std::fs::read(path).ok()?;
            scan_bytes(path, &bytes, settings)
        })
        .collect()
}
