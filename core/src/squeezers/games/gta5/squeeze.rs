use std::borrow::Cow;
use std::collections::{HashSet, hash_map::Entry};
use std::io::{Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use super::dictionary::{
    TexRecord, drawable_offsets, find_embedded_dictionaries, looks_like_vehicle_dict,
    parse_dictionary,
};
use super::format::{PixelLayout, chain_len, pixel_layout};
use super::optimize::{TexPatch, optimize_texture};
use super::raw::{
    DATA_ALIGN, GRAPHICS_BASE, RSC7_MAGIC, YTD_VERSION, flags_from_size, put_u16, put_u32, put_u64,
    size_from_flags, u32_at,
};
use super::shaders::{AssetFamily, classify_drawables};
use crate::error::{Result, SqueezeError};
use crate::gpu::GpuContext;
use crate::settings::SqueezeSettings;
use crate::squeezers::{SqueezeOutcome, TextureJob};
use crate::texture::{MIN_DIMENSION, TextureRole, classify_name, pair_base, target_dimensions};

#[derive(Debug, Clone, Copy)]
pub(crate) enum DrawableKind {
    Drawable,

    Dictionary,

    Fragment,
}

struct SqueezedTex<'a> {
    data: Cow<'a, [u8]>,

    patch: Option<TexPatch>,
}

enum TexKind {
    Optimized,
    Locked,
    Kept,
}

enum RecOutcome<'a> {
    Tex(SqueezedTex<'a>, TexKind),
}

const DICT_INSPECT_BYTES: usize = 16 * 1024 * 1024;

fn verify_declaration(path: &Path, rec: &TexRecord, tex: &SqueezedTex<'_>) -> Result<()> {
    let (width, height, levels, format) = match &tex.patch {
        Some(p) => (p.width as u32, p.height as u32, p.levels as u32, p.format),
        None => (rec.width, rec.height, rec.levels, rec.format),
    };

    let fail = |detail: String| SqueezeError::Rsc7 {
        path: path.to_path_buf(),
        detail,
    };

    let Some(layout) = pixel_layout(format) else {
        return Err(fail(format!(
            "texture `{}` would be written with unsupported format {format:#x}",
            rec.name
        )));
    };

    let needed = chain_len(width, height, levels, layout);
    if tex.data.len() < needed {
        return Err(fail(format!(
            "texture `{}` declares {width}x{height} levels={levels} format={format:#x} \
             ({needed} bytes) but only {} bytes were produced — refusing to emit a \
             container that would read past its graphics segment",
            rec.name,
            tex.data.len(),
        )));
    }
    Ok(())
}

fn warn_if_large_dict(path: &Path, records: &[TexRecord]) {
    let total: usize = records
        .iter()
        .filter_map(|r| {
            Some(chain_len(
                r.width,
                r.height,
                r.levels,
                pixel_layout(r.format)?,
            ))
        })
        .sum();
    if total >= DICT_INSPECT_BYTES {
        tracing::warn!(
            path = %path.display(),
            mib = total / (1024 * 1024),
            "large texture dictionary, inspect for oversized textures (fivem ~16 mib streaming)"
        );
    }
}

fn diffuse_pair_targets(
    records: &[TexRecord],
    settings: &SqueezeSettings,
) -> FxHashMap<String, u32> {
    let mut targets = FxHashMap::default();
    for rec in records {
        let path = Path::new(&rec.name);
        if matches!(classify_name(path), None | Some(TextureRole::Diffuse)) {
            let (tw, th) = target_dimensions(rec.width, rec.height, TextureRole::Diffuse, settings);
            let side = targets.entry(pair_base(path)).or_insert(0);
            *side = (*side).max(tw.max(th));
        }
    }
    targets
}

fn pair_cap_for(
    role: Option<TextureRole>,
    name: &str,
    targets: &FxHashMap<String, u32>,
) -> Option<u32> {
    if !matches!(role, Some(TextureRole::Normal | TextureRole::Specular)) {
        return None;
    }
    targets
        .get(&pair_base(Path::new(name)))
        .map(|&side| (side / 2).max(MIN_DIMENSION))
}

fn map_records<'a, T, F>(records: &'a [TexRecord], parallel: bool, f: F) -> Result<Vec<T>>
where
    T: Send,
    F: Fn(usize, &'a TexRecord) -> Result<T> + Sync + Send,
{
    const PARALLEL_MIN: usize = 4;
    if parallel && records.len() >= PARALLEL_MIN {
        records
            .par_iter()
            .enumerate()
            .map(|(i, r)| f(i, r))
            .collect()
    } else {
        records.iter().enumerate().map(|(i, r)| f(i, r)).collect()
    }
}

pub(crate) fn squeeze_ytd(
    job: &TextureJob<'_>,
    settings: &SqueezeSettings,
    gpu: Option<&GpuContext>,
) -> Result<SqueezeOutcome> {
    let rsc7 = |detail: String| SqueezeError::Rsc7 {
        path: job.path.to_path_buf(),
        detail,
    };

    let version = u32_at(job.bytes, 4).ok_or_else(|| rsc7("truncated header".into()))?;
    if version != YTD_VERSION {
        return Ok(SqueezeOutcome::Skipped {
            reason: format!(
                "RSC7 version {version} (expected {YTD_VERSION} for .ytd) — left untouched"
            ),
        });
    }
    let sys_flags = u32_at(job.bytes, 8).unwrap_or(0);
    let gfx_flags = u32_at(job.bytes, 12).unwrap_or(0);
    let sys_size = size_from_flags(sys_flags) as usize;
    let gfx_size = size_from_flags(gfx_flags) as usize;

    let mut segments = Vec::with_capacity(sys_size + gfx_size);
    DeflateDecoder::new(&job.bytes[16..])
        .read_to_end(&mut segments)
        .map_err(|e| rsc7(format!("deflate payload corrupt: {e}")))?;

    segments.resize(sys_size + gfx_size, 0);

    let gfx = segments.split_off(sys_size);
    let mut sys = segments;

    let records = parse_dictionary(&sys, &gfx, 0).map_err(rsc7)?;
    warn_if_large_dict(job.path, &records);

    let family = if looks_like_vehicle_dict(&records) {
        AssetFamily::Vehicle
    } else {
        job.asset_hint.unwrap_or(AssetFamily::Generic)
    };

    let pair_targets = diffuse_pair_targets(&records, settings);

    let outcomes = map_records(
        &records,
        !job.pool_saturated,
        |_, rec| -> Result<RecOutcome<'_>> {
            let role = classify_name(Path::new(&rec.name));

            let verbatim = |kind| {
                // Calculate source bytes even when pixel_layout is unknown so we
                // can pass the raw data through verbatim. If we can't determine
                // the length from the format, fall back to reading until end-of-gfx.
                let source = if let Some(layout) = pixel_layout(rec.format) {
                    let source_len = chain_len(rec.width, rec.height, rec.levels, layout);
                    gfx.get(rec.data..rec.data + source_len)
                        .unwrap_or(&gfx[rec.data..])
                } else {
                    &gfx[rec.data..]
                };
                RecOutcome::Tex(
                    SqueezedTex {
                        data: Cow::Borrowed(source),
                        patch: None,
                    },
                    kind,
                )
            };

            // Unknown/unsupported pixel format: keep the texture verbatim and
            // continue processing the rest of the dictionary. Previously this
            // bailed the entire .ytd with Skipped, unlike squeeze_drawable which
            // just kept the individual texture.
            let Some(layout) = pixel_layout(rec.format) else {
                tracing::debug!(
                    path = %job.path.display(),
                    name = %rec.name,
                    format = rec.format,
                    "unsupported pixel format — keeping texture verbatim"
                );
                return Ok(verbatim(TexKind::Kept));
            };

            let source_len = chain_len(rec.width, rec.height, rec.levels, layout);
            let source = gfx.get(rec.data..rec.data + source_len).ok_or_else(|| {
                rsc7(format!("texture `{}` mip chain overruns segment", rec.name))
            })?;

            let verbatim_from_source = |kind| {
                RecOutcome::Tex(
                    SqueezedTex {
                        data: Cow::Borrowed(source),
                        patch: None,
                    },
                    kind,
                )
            };

            if role == Some(TextureRole::ScriptRenderTarget) {
                return Ok(verbatim_from_source(TexKind::Locked));
            }
            if !matches!(layout, PixelLayout::Block { .. }) {
                return Ok(verbatim_from_source(TexKind::Kept));
            }

            let pair_cap = pair_cap_for(role, &rec.name, &pair_targets);
            Ok(
                match optimize_texture(
                    rec, source, layout, role, pair_cap, settings, None, gpu, family,
                ) {
                    Some((data, patch)) => RecOutcome::Tex(
                        SqueezedTex {
                            data: Cow::Owned(data),
                            patch: Some(patch),
                        },
                        TexKind::Optimized,
                    ),
                    None => verbatim_from_source(TexKind::Kept),
                },
            )
        },
    )?;

    let mut squeezed: Vec<SqueezedTex<'_>> = Vec::with_capacity(records.len());
    let (mut optimized, mut locked, mut kept) = (0usize, 0usize, 0usize);
    for outcome in outcomes {
        match outcome {
            RecOutcome::Tex(tex, kind) => {
                match kind {
                    TexKind::Optimized => optimized += 1,
                    TexKind::Locked => locked += 1,
                    TexKind::Kept => kept += 1,
                }
                squeezed.push(tex);
            }
        }
    }

    let mut unique = HashSet::new();
    let duplicates = squeezed
        .iter()
        .filter(|t| !unique.insert(t.data.as_ref()))
        .count();

    if optimized == 0 && duplicates == 0 {
        return Ok(SqueezeOutcome::Skipped {
            reason: format!(
                "already optimal ({} textures: {locked} locked, {kept} kept)",
                records.len()
            ),
        });
    }

    let mut new_gfx: Vec<u8> = Vec::with_capacity(gfx.len());
    let mut dedup: FxHashMap<&[u8], u64> = FxHashMap::default();
    dedup.reserve(squeezed.len());
    for (rec, tex) in records.iter().zip(&squeezed) {
        verify_declaration(job.path, rec, tex)?;

        let offset = match dedup.entry(tex.data.as_ref()) {
            Entry::Occupied(slot) => *slot.get(),
            Entry::Vacant(slot) => {
                let offset = new_gfx.len().next_multiple_of(DATA_ALIGN);
                new_gfx.resize(offset, 0);
                new_gfx.extend_from_slice(&tex.data);
                *slot.insert(offset as u64)
            }
        };

        put_u64(&mut sys, rec.record + 0x70, GRAPHICS_BASE | offset);
        if let Some(p) = &tex.patch {
            put_u16(&mut sys, rec.record + 0x50, p.width);
            put_u16(&mut sys, rec.record + 0x52, p.height);
            put_u16(&mut sys, rec.record + 0x56, p.stride);
            put_u32(&mut sys, rec.record + 0x58, p.format);
            sys[rec.record + 0x5D] = p.levels;
        }
    }

    let (new_gfx_flags, padded_gfx) = flags_from_size(new_gfx.len() as u64, gfx_flags >> 28)
        .ok_or_else(|| rsc7("graphics segment too large to encode".into()))?;
    new_gfx.resize(padded_gfx as usize, 0);

    let mut out = Vec::with_capacity(job.bytes.len());
    out.extend_from_slice(&RSC7_MAGIC);
    out.extend_from_slice(&YTD_VERSION.to_le_bytes());
    out.extend_from_slice(&sys_flags.to_le_bytes());
    out.extend_from_slice(&new_gfx_flags.to_le_bytes());
    let mut encoder = DeflateEncoder::new(&mut out, Compression::default());
    encoder
        .write_all(&sys)
        .and_then(|_| encoder.write_all(&new_gfx))
        .and_then(|_| encoder.finish().map(|_| ()))
        .map_err(|e| rsc7(format!("recompression failed: {e}")))?;

    if out.len() >= job.bytes.len() {
        if gpu.is_some() && out.len() < job.bytes.len() + job.bytes.len() / 16 {
            tracing::debug!(path = %job.path.display(), "near break-even with GPU mix — retrying CPU-only");
            return squeeze_ytd(job, settings, None);
        }
        return Ok(SqueezeOutcome::Skipped {
            reason: format!(
                "no size win after rebuild ({} in, {} out)",
                job.bytes.len(),
                out.len()
            ),
        });
    }

    Ok(SqueezeOutcome::Optimized {
        bytes: out,
        extension: "ytd",
    })
}

pub(crate) fn squeeze_drawable(
    job: &TextureJob<'_>,
    settings: &SqueezeSettings,
    kind: DrawableKind,
    extension: &'static str,
    gpu: Option<&GpuContext>,
) -> Result<SqueezeOutcome> {
    let rsc7 = |detail: String| SqueezeError::Rsc7 {
        path: job.path.to_path_buf(),
        detail,
    };

    let version = u32_at(job.bytes, 4).ok_or_else(|| rsc7("truncated header".into()))?;
    let sys_flags = u32_at(job.bytes, 8).unwrap_or(0);
    let gfx_flags = u32_at(job.bytes, 12).unwrap_or(0);
    let sys_size = size_from_flags(sys_flags) as usize;
    let gfx_size = size_from_flags(gfx_flags) as usize;

    let mut segments = Vec::with_capacity(sys_size + gfx_size);
    DeflateDecoder::new(&job.bytes[16..])
        .read_to_end(&mut segments)
        .map_err(|e| rsc7(format!("deflate payload corrupt: {e}")))?;
    segments.resize(sys_size + gfx_size, 0);
    let mut gfx = segments.split_off(sys_size);
    let mut sys = segments;

    let dict_offsets = find_embedded_dictionaries(&sys, kind);
    if dict_offsets.is_empty() {
        return Ok(SqueezeOutcome::Skipped {
            reason: "drawable has no embedded texture dictionary".into(),
        });
    }

    let mut records: Vec<TexRecord> = Vec::new();
    let mut seen = HashSet::new();
    for dict_off in dict_offsets {
        if let Ok(parsed) = parse_dictionary(&sys, &gfx, dict_off) {
            records.extend(parsed.into_iter().filter(|r| seen.insert(r.record)));
        }
    }
    if records.is_empty() {
        return Ok(SqueezeOutcome::Skipped {
            reason: "no parseable embedded texture dictionary".into(),
        });
    }
    warn_if_large_dict(job.path, &records);

    let mut family = classify_drawables(&sys, &drawable_offsets(&sys, kind));
    if family == AssetFamily::Generic
        && let Some(hint) = job.asset_hint
    {
        family = hint;
    }

    let pair_targets = diffuse_pair_targets(&records, settings);

    struct DrawPatch {
        rec: usize,
        data: Vec<u8>,
        patch: TexPatch,
        source_len: usize,
    }
    enum DrawOutcome {
        Optimized(DrawPatch),
        Locked,
        Kept,
    }

    let outcomes = map_records(
        &records,
        !job.pool_saturated,
        |idx, rec| -> Result<DrawOutcome> {
            let role = classify_name(Path::new(&rec.name));
            if role == Some(TextureRole::ScriptRenderTarget) {
                return Ok(DrawOutcome::Locked);
            }

            let Some(layout @ PixelLayout::Block { .. }) = pixel_layout(rec.format) else {
                return Ok(DrawOutcome::Kept);
            };
            let source_len = chain_len(rec.width, rec.height, rec.levels, layout);
            if rec.data + source_len > gfx.len() {
                return Err(rsc7(format!(
                    "texture `{}` mip chain overruns segment",
                    rec.name
                )));
            }

            let source = &gfx[rec.data..rec.data + source_len];
            let pair_cap = pair_cap_for(role, &rec.name, &pair_targets);
            Ok(
                match optimize_texture(
                    rec,
                    source,
                    layout,
                    role,
                    pair_cap,
                    settings,
                    Some(source_len),
                    gpu,
                    family,
                ) {
                    Some((data, patch)) => DrawOutcome::Optimized(DrawPatch {
                        rec: idx,
                        data,
                        patch,
                        source_len,
                    }),
                    None => DrawOutcome::Kept,
                },
            )
        },
    )?;

    let (mut optimized, mut locked, mut kept) = (0usize, 0usize, 0usize);
    for outcome in outcomes {
        match outcome {
            DrawOutcome::Locked => locked += 1,
            DrawOutcome::Kept => kept += 1,
            DrawOutcome::Optimized(p) => {
                let rec = &records[p.rec];

                if let Some(layout) = pixel_layout(p.patch.format) {
                    let needed = chain_len(
                        p.patch.width as u32,
                        p.patch.height as u32,
                        p.patch.levels as u32,
                        layout,
                    );
                    if p.data.len() < needed {
                        return Err(rsc7(format!(
                            "embedded texture `{}` declares {}x{} levels={} ({needed} bytes) \
                             but only {} bytes were produced",
                            rec.name,
                            p.patch.width,
                            p.patch.height,
                            p.patch.levels,
                            p.data.len(),
                        )));
                    }
                }

                let slot = &mut gfx[rec.data..rec.data + p.source_len];
                slot[..p.data.len()].copy_from_slice(&p.data);
                slot[p.data.len()..].fill(0);

                put_u16(&mut sys, rec.record + 0x50, p.patch.width);
                put_u16(&mut sys, rec.record + 0x52, p.patch.height);
                put_u16(&mut sys, rec.record + 0x56, p.patch.stride);
                put_u32(&mut sys, rec.record + 0x58, p.patch.format);
                sys[rec.record + 0x5D] = p.patch.levels;
                optimized += 1;
            }
        }
    }

    if optimized == 0 {
        return Ok(SqueezeOutcome::Skipped {
            reason: format!(
                "already optimal ({} embedded textures: {locked} locked, {kept} kept)",
                records.len()
            ),
        });
    }

    let mut out = Vec::with_capacity(job.bytes.len());
    out.extend_from_slice(&RSC7_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&sys_flags.to_le_bytes());
    out.extend_from_slice(&gfx_flags.to_le_bytes());
    let mut encoder = DeflateEncoder::new(&mut out, Compression::default());
    encoder
        .write_all(&sys)
        .and_then(|_| encoder.write_all(&gfx))
        .and_then(|_| encoder.finish().map(|_| ()))
        .map_err(|e| rsc7(format!("recompression failed: {e}")))?;

    if out.len() >= job.bytes.len() {
        if gpu.is_some() && out.len() < job.bytes.len() + job.bytes.len() / 16 {
            tracing::debug!(path = %job.path.display(), "near break-even with GPU mix — retrying CPU-only");
            return squeeze_drawable(job, settings, kind, extension, None);
        }
        return Ok(SqueezeOutcome::Skipped {
            reason: format!(
                "no size win after rewrite ({} in, {} out)",
                job.bytes.len(),
                out.len()
            ),
        });
    }

    Ok(SqueezeOutcome::Optimized {
        bytes: out,
        extension,
    })
}
