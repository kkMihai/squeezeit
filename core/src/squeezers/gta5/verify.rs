use super::DrawableKind;
use super::dictionary::{
    TexRecord, drawable_offsets, find_embedded_dictionaries, looks_like_vehicle_dict,
    parse_dictionary,
};
use super::format::{chain_len, pixel_layout, rage_mip_levels};
use super::raw::{RSC7_MAGIC, size_from_flags, u32_at};
use super::shaders::{AssetFamily, classify_drawables};
use crate::error::{Result, SqueezeError};
use crate::rsc7::pages::PageLayout;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Shape {
    Dictionary,
    Drawable(DrawableKind),
}

impl Shape {
    fn of(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("ydr") => Shape::Drawable(DrawableKind::Drawable),
            Some("ydd") => Shape::Drawable(DrawableKind::Dictionary),
            Some("yft") => Shape::Drawable(DrawableKind::Fragment),
            _ => Shape::Dictionary,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub levels: u32,
    pub offset: usize,
    pub declared_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInfo {
    pub graphics_bytes: u64,
    pub pages: usize,
    pub family: AssetFamily,
    pub textures: Vec<TextureInfo>,
}

pub fn inspect_bytes(path: &Path, bytes: &[u8]) -> Result<ContainerInfo> {
    let shape = Shape::of(path);
    let (layout, records, family) = read_back(path, bytes, shape, &|detail| detail)?;
    Ok(ContainerInfo {
        graphics_bytes: layout.total_size(),
        pages: layout.page_count(),
        family,
        textures: records
            .iter()
            .map(|rec| TextureInfo {
                name: rec.name.clone(),
                width: rec.width,
                height: rec.height,
                format: rec.format,
                levels: rec.levels,
                offset: rec.data,
                declared_bytes: pixel_layout(rec.format)
                    .map(|l| chain_len(rec.width, rec.height, rec.levels, l) as u64),
            })
            .collect(),
    })
}

pub fn verify_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    verify_container(path, bytes, Shape::of(path))
}

pub(crate) fn verify_container(path: &Path, bytes: &[u8], shape: Shape) -> Result<()> {
    let preface =
        |detail: String| format!("refusing to write a container that would not load: {detail}");
    let fail = |detail: String| SqueezeError::Rsc7 {
        path: path.to_path_buf(),
        detail: preface(detail),
    };

    let (layout, records, _) = read_back(path, bytes, shape, &preface)?;

    let mut checked = 0usize;
    for rec in &records {
        let Some(pixels) = pixel_layout(rec.format) else {
            if layout.page_of(rec.data as u64).is_none() {
                return Err(fail(format!(
                    "texture `{}` in unreadable format {:#x} sits at {:#x}, which is inside no \
                     page at all",
                    rec.name, rec.format, rec.data,
                )));
            }
            continue;
        };

        let declared = chain_len(rec.width, rec.height, rec.levels, pixels);
        let sampled = chain_len(
            rec.width,
            rec.height,
            rec.levels
                .min(rage_mip_levels(rec.width, rec.height))
                .max(1),
            pixels,
        );
        let total = layout.total_size() as usize;

        if rec.data + sampled > total {
            return Err(fail(format!(
                "texture `{}` declares {}x{} levels={} ({sampled} sampled bytes) at {:#x}, which \
                 runs {} bytes past a {total} byte graphics segment",
                rec.name,
                rec.width,
                rec.height,
                rec.levels,
                rec.data,
                rec.data + sampled - total,
            )));
        }

        if !layout.holds_block(rec.data as u64, sampled as u64) {
            return Err(fail(format!(
                "texture `{}` spans {:#x}..{:#x}, crossing a page boundary in a segment of {} \
                 pages",
                rec.name,
                rec.data,
                rec.data + sampled,
                layout.page_count(),
            )));
        }

        if declared > sampled && !layout.holds_block(rec.data as u64, declared as u64) {
            tracing::debug!(
                path = %path.display(),
                texture = %rec.name,
                levels = rec.levels,
                "record over-declares its mip tail past the end of its page; the engine never \
                 samples that far, so it is left alone"
            );
        }

        checked += 1;
    }

    tracing::trace!(path = %path.display(), textures = checked, "container verified");
    Ok(())
}

fn read_back(
    path: &Path,
    bytes: &[u8],
    shape: Shape,
    preface: &dyn Fn(String) -> String,
) -> Result<(PageLayout, Vec<TexRecord>, AssetFamily)> {
    let fail = |detail: String| SqueezeError::Rsc7 {
        path: path.to_path_buf(),
        detail: preface(detail),
    };

    if bytes.get(..4) != Some(&RSC7_MAGIC) {
        return Err(fail("not an RSC7 container".into()));
    }
    let sys_flags = u32_at(bytes, 8).ok_or_else(|| fail("truncated header".into()))?;
    let gfx_flags = u32_at(bytes, 12).ok_or_else(|| fail("truncated header".into()))?;

    let sys_size = size_from_flags(sys_flags) as usize;
    let layout = PageLayout::from_flags(gfx_flags);
    let gfx_size = layout.total_size() as usize;

    let mut segments = Vec::with_capacity(sys_size + gfx_size);
    {
        use std::io::Read;
        flate2::read::DeflateDecoder::new(&bytes[16..])
            .read_to_end(&mut segments)
            .map_err(|e| fail(format!("the payload does not inflate: {e}")))?;
    }
    if segments.len() < sys_size + gfx_size {
        return Err(fail(format!(
            "header claims {sys_size} system and {gfx_size} graphics bytes, the payload inflates \
             to {}",
            segments.len()
        )));
    }
    let (sys, gfx) = segments.split_at(sys_size);
    let gfx = &gfx[..gfx_size];

    let dictionaries: Vec<usize> = match shape {
        Shape::Dictionary => vec![0],
        Shape::Drawable(kind) => find_embedded_dictionaries(sys, kind),
    };
    let mut family = match shape {
        Shape::Dictionary => AssetFamily::Generic,
        Shape::Drawable(kind) => classify_drawables(sys, &drawable_offsets(sys, kind)),
    };

    let mut records = Vec::new();
    let mut seen = rustc_hash::FxHashSet::default();
    for offset in dictionaries {
        if let Ok(parsed) = parse_dictionary(sys, gfx, offset) {
            records.extend(parsed.into_iter().filter(|r| seen.insert(r.record)));
        }
    }

    if family == AssetFamily::Generic && looks_like_vehicle_dict(&records) {
        family = AssetFamily::Vehicle;
    }
    Ok((layout, records, family))
}
