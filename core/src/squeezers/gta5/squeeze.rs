use super::dictionary::{
    TexRecord, drawable_offsets, find_embedded_dictionaries, looks_like_vehicle_dict,
    parse_dictionary,
};
use super::format::{PixelLayout, chain_len, mip_len, pixel_layout, top_stride};
use super::optimize::{Request, TexPatch, optimize_texture};
use super::raw::{
    DATA_ALIGN, GRAPHICS_BASE, RSC7_MAGIC, YTD_VERSION, graphics_offset, put_u16, put_u32, put_u64,
    size_from_flags, u32_at,
};
use super::resolve_family;
use super::shaders::{AssetFamily, classify_drawables};
use super::verify::{self, Shape};
use crate::error::{Result, SqueezeError};
use crate::gpu::GpuContext;
use crate::policy::{Container, Policy};
use crate::rsc7::pages::PageLayout;
use crate::settings::{ScriptRt, SqueezeSettings};
use crate::squeezers::{SqueezeOutcome, TextureBytes, TextureJob, codec};
use crate::texture::{MIN_DIMENSION, TextureRole, classify_name, pair_base, target_dimensions};
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::io::{Read, Write};
use std::path::Path;
const MIB: u64 = 1024 * 1024;
const BLOATED_DICT_BYTES: u64 = 16 * MIB;
const PARALLEL_MIN: usize = 4;

#[derive(Debug, Clone, Copy)]
pub(crate) enum DrawableKind {
    Drawable,
    Dictionary,
    Fragment,
}

#[derive(Default, Debug)]
struct Tally {
    optimized: usize,
    locked: usize,
    kept: usize,
}

impl Tally {
    fn nothing_to_do(&self, total: usize, verb: &str) -> SqueezeOutcome {
        SqueezeOutcome::skipped(format!(
            "already optimal ({total} {verb}: {} locked, {} kept)",
            self.locked, self.kept
        ))
    }
}

struct SqueezedTex<'a> {
    data: Cow<'a, [u8]>,
    patch: Option<TexPatch>,
}

enum TexKind {
    Optimized,
    Repaired,
    Locked,
    Kept,
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
        return Ok(SqueezeOutcome::skipped(format!(
            "RSC7 version {version} (expected {YTD_VERSION} for .ytd), left untouched"
        )));
    }

    let sys_flags = u32_at(job.bytes, 8).unwrap_or(0);
    let gfx_flags = u32_at(job.bytes, 12).unwrap_or(0);
    let (mut sys, gfx) = inflate_segments(job, sys_flags, gfx_flags).map_err(&rsc7)?;

    let records = parse_dictionary(&sys, &gfx, 0).map_err(&rsc7)?;
    let held_before = resident_bytes(&records);
    warn_if_bloated(job.path, held_before);

    let source_layout = PageLayout::from_flags(gfx_flags);
    let reserved_before = source_layout.total_size();
    warn_if_not_power_of_two(job.path, &records);

    let family = if looks_like_vehicle_dict(&records) {
        AssetFamily::Vehicle
    } else {
        job.asset_hint.unwrap_or(AssetFamily::Generic)
    };
    let policy = Policy::resolve(settings, family);
    let pair_targets = diffuse_pair_targets(&records, &policy);
    let extents = payload_extents(&records, &source_layout, gfx.len());

    let outcomes = map_records(&records, !job.pool_saturated, |_, rec| {
        let role = classify_name(Path::new(&rec.name));

        let Some(layout) = pixel_layout(rec.format) else {
            tracing::debug!(
                path = %job.path.display(),
                name = %rec.name,
                format = rec.format,
                "unsupported pixel format, keeping texture verbatim"
            );
            return Ok(keep(carried_payload(&gfx, rec, &extents), TexKind::Kept));
        };

        let source = stored_payload(&gfx, rec, &extents);

        if role == Some(TextureRole::ScriptRenderTarget) {
            return Ok(match repair_render_target(settings, rec, source, layout) {
                Some((data, patch)) => (
                    SqueezedTex {
                        data: Cow::Owned(data),
                        patch: Some(patch),
                    },
                    TexKind::Repaired,
                ),
                None => keep(carried_payload(&gfx, rec, &extents), TexKind::Locked),
            });
        }
        if settings.script_rt == ScriptRt::Only {
            return Ok(keep(carried_payload(&gfx, rec, &extents), TexKind::Kept));
        }
        if settings.exclusions.excludes(&rec.name) {
            return Ok(keep(carried_payload(&gfx, rec, &extents), TexKind::Locked));
        }
        if !matches!(layout, PixelLayout::Block { .. }) {
            return Ok(keep(carried_payload(&gfx, rec, &extents), TexKind::Kept));
        }

        let request = Request {
            rec,
            source,
            layout,
            name_role: role,
            pair_cap: pair_cap_for(role, &rec.name, &pair_targets),
            max_len: None,
            container: Container::Dictionary,
            gpu,
            policy: &policy,
        };
        Ok(match optimize_texture(request, settings) {
            Some((data, patch)) => (
                SqueezedTex {
                    data: Cow::Owned(data),
                    patch: Some(patch),
                },
                TexKind::Optimized,
            ),
            None => keep(carried_payload(&gfx, rec, &extents), TexKind::Kept),
        })
    })?;

    let mut tally = Tally::default();
    let mut repaired = 0usize;
    let squeezed: Vec<SqueezedTex<'_>> = outcomes
        .into_iter()
        .map(|(tex, kind)| {
            match kind {
                TexKind::Optimized => tally.optimized += 1,
                TexKind::Repaired => {
                    tally.optimized += 1;
                    repaired += 1;
                }
                TexKind::Locked => tally.locked += 1,
                TexKind::Kept => tally.kept += 1,
            }
            tex
        })
        .collect();

    let repacked = repack_graphics(job.path, &mut sys, &records, &squeezed)?;

    if tally.optimized == 0 && repacked.duplicates == 0 {
        return Ok(tally
            .nothing_to_do(records.len(), "textures")
            .with_textures(TextureBytes::unchanged(reserved_before)));
    }
    if !repacked.worth_taking(reserved_before, held_before) {
        return Ok(SqueezeOutcome::skipped(format!(
            "rebuilding would reserve {} bytes of graphics memory against the {reserved_before} \
             it already uses, for {} bytes of textures against {held_before}",
            repacked.layout.total_size(),
            repacked.held,
        ))
        .with_textures(TextureBytes::unchanged(reserved_before)));
    }

    let textures = TextureBytes {
        before: reserved_before,
        after: repacked.layout.total_size(),
    };

    let out = deflate_container(
        job,
        YTD_VERSION,
        sys_flags,
        repacked.layout.to_flags(gfx_flags >> 28),
        &sys,
        &repacked.gfx,
    )
    .map_err(&rsc7)?;

    match settle(job, out, gpu, repaired > 0) {
        Settled::Win(bytes) => {
            verify::verify_container(job.path, &bytes, Shape::Dictionary)?;
            Ok(SqueezeOutcome::optimized(bytes, "ytd").with_textures(textures))
        }

        Settled::RetryOnCpu => squeeze_ytd(job, settings, None),
        Settled::Loss(reason) => {
            Ok(SqueezeOutcome::skipped(reason).with_textures(textures.discarded()))
        }
    }
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
    let (mut sys, mut gfx) = inflate_segments(job, sys_flags, gfx_flags).map_err(&rsc7)?;

    let dict_offsets = find_embedded_dictionaries(&sys, kind);
    if dict_offsets.is_empty() {
        return Ok(SqueezeOutcome::skipped(
            "drawable has no embedded texture dictionary",
        ));
    }

    let mut records: Vec<TexRecord> = Vec::new();
    let mut seen = FxHashSet::default();
    for dict_off in dict_offsets {
        if let Ok(parsed) = parse_dictionary(&sys, &gfx, dict_off) {
            records.extend(parsed.into_iter().filter(|r| seen.insert(r.record)));
        }
    }
    if records.is_empty() {
        return Ok(SqueezeOutcome::skipped(
            "no parseable embedded texture dictionary",
        ));
    }
    let held_before = resident_bytes(&records);
    warn_if_bloated(job.path, held_before);
    warn_if_not_power_of_two(job.path, &records);

    let family = resolve_family(
        classify_drawables(&sys, &drawable_offsets(&sys, kind)),
        job.asset_hint,
    );
    let policy = Policy::resolve(settings, family);
    let pair_targets = diffuse_pair_targets(&records, &policy);
    let source_layout = PageLayout::from_flags(gfx_flags);
    let extents = payload_extents(&records, &source_layout, gfx.len());

    let rebuilt = graphics_holds_only_textures(&sys, &records, gfx.len());
    let container = Container::Drawable { rebuilt };
    let reserved_before = source_layout.total_size();

    struct Patched {
        rec: usize,
        data: Vec<u8>,
        patch: TexPatch,
        source_len: usize,
    }

    let outcomes = map_records(&records, !job.pool_saturated, |idx, rec| {
        let role = classify_name(Path::new(&rec.name));
        if role == Some(TextureRole::ScriptRenderTarget) {
            let repaired = rebuilt
                .then(|| pixel_layout(rec.format))
                .flatten()
                .and_then(|layout| {
                    let source = stored_payload(&gfx, rec, &extents);
                    repair_render_target(settings, rec, source, layout)
                        .map(|(data, patch)| (data, patch, source.len()))
                });
            return Ok(match repaired {
                Some((data, patch, source_len)) => (
                    Some(Patched {
                        rec: idx,
                        data,
                        patch,
                        source_len,
                    }),
                    TexKind::Repaired,
                ),
                None => (None, TexKind::Locked),
            });
        }
        if settings.script_rt == ScriptRt::Only || settings.exclusions.excludes(&rec.name) {
            return Ok((None, TexKind::Kept));
        }

        let Some(layout @ PixelLayout::Block { .. }) = pixel_layout(rec.format) else {
            return Ok((None, TexKind::Kept));
        };
        let source = stored_payload(&gfx, rec, &extents);
        if source.len() < mip_len(rec.width, rec.height, layout) {
            tracing::debug!(
                path = %job.path.display(),
                name = %rec.name,
                "record claims more than the segment stores, leaving it alone"
            );
            return Ok((None, TexKind::Kept));
        }

        let request = Request {
            rec,
            source,
            layout,
            name_role: role,
            pair_cap: pair_cap_for(role, &rec.name, &pair_targets),
            max_len: (!rebuilt).then_some(source.len()),
            container,
            gpu,
            policy: &policy,
        };
        Ok(match optimize_texture(request, settings) {
            Some((data, patch)) => (
                Some(Patched {
                    rec: idx,
                    data,
                    patch,
                    source_len: source.len(),
                }),
                TexKind::Optimized,
            ),
            None => (None, TexKind::Kept),
        })
    })?;

    let mut tally = Tally::default();
    let mut repaired = 0usize;
    let mut patched: Vec<Option<Patched>> = Vec::with_capacity(outcomes.len());
    for (p, kind) in outcomes {
        match kind {
            TexKind::Locked => tally.locked += 1,
            TexKind::Kept => tally.kept += 1,
            TexKind::Optimized => tally.optimized += 1,
            TexKind::Repaired => {
                tally.optimized += 1;
                repaired += 1;
            }
        }
        if let Some(p) = &p
            && let Some(layout) = pixel_layout(p.patch.format)
        {
            let rec = &records[p.rec];
            let needed = chain_len(
                p.patch.width as u32,
                p.patch.height as u32,
                p.patch.levels as u32,
                layout,
            );
            if p.data.len() < needed {
                return Err(rsc7(format!(
                    "embedded texture `{}` declares {}x{} levels={} ({needed} bytes) but only {} \
                     bytes were produced",
                    rec.name,
                    p.patch.width,
                    p.patch.height,
                    p.patch.levels,
                    p.data.len(),
                )));
            }
        }
        patched.push(p);
    }

    let (new_gfx_flags, textures) = if rebuilt {
        let squeezed: Vec<SqueezedTex<'_>> = records
            .iter()
            .zip(&patched)
            .map(|(rec, p)| match p {
                Some(p) => SqueezedTex {
                    data: Cow::Borrowed(p.data.as_slice()),
                    patch: Some(p.patch),
                },
                None => SqueezedTex {
                    data: carried_payload(&gfx, rec, &extents),
                    patch: None,
                },
            })
            .collect();

        let repacked = repack_graphics(job.path, &mut sys, &records, &squeezed)?;
        if tally.optimized == 0 && repacked.duplicates == 0 {
            return Ok(tally
                .nothing_to_do(records.len(), "embedded textures")
                .with_textures(TextureBytes::unchanged(reserved_before)));
        }
        if !repacked.worth_taking(reserved_before, held_before) {
            return Ok(SqueezeOutcome::skipped(format!(
                "rebuilding would reserve {} bytes of graphics memory against the \
                 {reserved_before} it already uses",
                repacked.layout.total_size(),
            ))
            .with_textures(TextureBytes::unchanged(reserved_before)));
        }
        let flags = repacked.layout.to_flags(gfx_flags >> 28);
        let textures = TextureBytes {
            before: reserved_before,
            after: repacked.layout.total_size(),
        };
        gfx = repacked.gfx;
        (flags, textures)
    } else {
        for p in patched.iter().flatten() {
            let rec = &records[p.rec];
            let slot = &mut gfx[rec.data..rec.data + p.source_len];
            slot[..p.data.len()].copy_from_slice(&p.data);
            slot[p.data.len()..].fill(0);

            put_u16(&mut sys, rec.record + 0x50, p.patch.width);
            put_u16(&mut sys, rec.record + 0x52, p.patch.height);
            put_u16(&mut sys, rec.record + 0x56, p.patch.stride);
            put_u32(&mut sys, rec.record + 0x58, p.patch.format);
            sys[rec.record + 0x5D] = p.patch.levels;
        }
        if tally.optimized == 0 {
            return Ok(tally
                .nothing_to_do(records.len(), "embedded textures")
                .with_textures(TextureBytes::unchanged(reserved_before)));
        }
        (gfx_flags, TextureBytes::unchanged(reserved_before))
    };

    let out =
        deflate_container(job, version, sys_flags, new_gfx_flags, &sys, &gfx).map_err(&rsc7)?;

    match settle(job, out, gpu, repaired > 0) {
        Settled::Win(bytes) => {
            verify::verify_container(job.path, &bytes, Shape::Drawable(kind))?;
            Ok(SqueezeOutcome::optimized(bytes, extension).with_textures(textures))
        }

        Settled::RetryOnCpu => squeeze_drawable(job, settings, kind, extension, None),
        Settled::Loss(reason) => {
            Ok(SqueezeOutcome::skipped(reason).with_textures(textures.discarded()))
        }
    }
}

const A8R8G8B8: u32 = 21;

fn repair_render_target(
    settings: &SqueezeSettings,
    rec: &TexRecord,
    source: &[u8],
    layout: PixelLayout,
) -> Option<(Vec<u8>, TexPatch)> {
    if !settings.script_rt.repairs() {
        return None;
    }
    let compressed = matches!(layout, PixelLayout::Block { .. });
    if !compressed && rec.levels <= 1 {
        return None;
    }
    let top = source.get(..mip_len(rec.width, rec.height, layout))?;

    let (data, format, stride) = if compressed {
        let mut raw = match layout {
            PixelLayout::Block { dds, .. } => {
                codec::decode_block(top, dds, rec.width, rec.height)?.into_raw()
            }
            PixelLayout::Linear { .. } => return None,
        };

        for pixel in raw.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let stride = u16::try_from(rec.width.checked_mul(4)?).ok()?;
        (raw, A8R8G8B8, stride)
    } else {
        let stride = top_stride(rec.width, layout);

        if u64::from(stride) != u64::from(rec.width) * bytes_per_pixel(layout)? {
            return None;
        }
        (top.to_vec(), rec.format, stride)
    };

    Some((
        data,
        TexPatch {
            width: u16::try_from(rec.width).ok()?,
            height: u16::try_from(rec.height).ok()?,
            stride,
            format,
            levels: 1,
        },
    ))
}

fn bytes_per_pixel(layout: PixelLayout) -> Option<u64> {
    match layout {
        PixelLayout::Linear { bytes_per_pixel } => Some(bytes_per_pixel as u64),
        PixelLayout::Block { .. } => None,
    }
}

fn stored_payload<'a>(
    gfx: &'a [u8],
    rec: &TexRecord,
    extents: &FxHashMap<usize, usize>,
) -> &'a [u8] {
    let room = extents.get(&rec.data).copied().unwrap_or(0);
    let want = pixel_layout(rec.format)
        .map(|layout| chain_len(rec.width, rec.height, rec.levels, layout))
        .unwrap_or(room);
    gfx.get(rec.data..rec.data + want.min(room))
        .unwrap_or_default()
}

fn carried_payload<'a>(
    gfx: &'a [u8],
    rec: &TexRecord,
    extents: &FxHashMap<usize, usize>,
) -> Cow<'a, [u8]> {
    let stored = stored_payload(gfx, rec, extents);
    let Some(layout) = pixel_layout(rec.format) else {
        return Cow::Borrowed(stored);
    };
    let declared = chain_len(rec.width, rec.height, rec.levels, layout);
    if stored.len() >= declared {
        return Cow::Borrowed(stored);
    }
    let mut padded = stored.to_vec();
    padded.resize(declared, 0);
    Cow::Owned(padded)
}

fn graphics_holds_only_textures(sys: &[u8], records: &[TexRecord], gfx_len: usize) -> bool {
    let known: FxHashSet<usize> = records.iter().map(|rec| rec.data).collect();
    sys.as_chunks::<8>()
        .0
        .iter()
        .filter_map(|word| {
            let ptr = u64::from_le_bytes(*word);
            graphics_offset(ptr, gfx_len)
        })
        .all(|offset| known.contains(&offset))
}

const KEY_EDGE: usize = 32;

type PayloadKey = (usize, [u8; KEY_EDGE], [u8; KEY_EDGE]);

fn payload_key(data: &[u8]) -> PayloadKey {
    let mut head = [0u8; KEY_EDGE];
    let mut tail = [0u8; KEY_EDGE];
    let take = data.len().min(KEY_EDGE);
    head[..take].copy_from_slice(&data[..take]);
    tail[..take].copy_from_slice(&data[data.len() - take..]);
    (data.len(), head, tail)
}

struct Repacked {
    gfx: Vec<u8>,
    layout: PageLayout,
    duplicates: usize,
    held: u64,
}

impl Repacked {
    fn worth_taking(&self, reserved_before: u64, held_before: u64) -> bool {
        self.layout.total_size() <= reserved_before || self.held > held_before
    }
}

fn repack_graphics(
    path: &Path,
    sys: &mut [u8],
    records: &[TexRecord],
    squeezed: &[SqueezedTex<'_>],
) -> Result<Repacked> {
    let mut dedup: FxHashMap<PayloadKey, usize> = FxHashMap::default();
    dedup.reserve(squeezed.len());
    let mut blocks: Vec<&[u8]> = Vec::with_capacity(squeezed.len());
    let mut block_of: Vec<usize> = Vec::with_capacity(squeezed.len());
    let mut duplicates = 0usize;

    for (rec, tex) in records.iter().zip(squeezed) {
        verify_declaration(path, rec, tex)?;
        let payload = tex.data.as_ref();
        let mut index = blocks.len();
        match dedup.entry(payload_key(payload)) {
            Entry::Occupied(slot) if blocks[*slot.get()] == payload => {
                duplicates += 1;
                index = *slot.get();
            }
            Entry::Occupied(_) => blocks.push(payload),
            Entry::Vacant(slot) => {
                slot.insert(index);
                blocks.push(payload);
            }
        }
        block_of.push(index);
    }

    let sizes: Vec<usize> = blocks.iter().map(|block| block.len()).collect();
    let (layout, offsets) =
        PageLayout::pack(&sizes, DATA_ALIGN).ok_or_else(|| SqueezeError::Rsc7 {
            path: path.to_path_buf(),
            detail: "graphics segment will not fit any page layout".into(),
        })?;

    let mut gfx = vec![0u8; layout.total_size() as usize];
    for (block, &offset) in blocks.iter().zip(&offsets) {
        let at = offset as usize;
        gfx[at..at + block.len()].copy_from_slice(block);
    }

    for ((rec, tex), &index) in records.iter().zip(squeezed).zip(&block_of) {
        put_u64(sys, rec.record + 0x70, GRAPHICS_BASE | offsets[index]);
        if let Some(p) = &tex.patch {
            put_u16(sys, rec.record + 0x50, p.width);
            put_u16(sys, rec.record + 0x52, p.height);
            put_u16(sys, rec.record + 0x56, p.stride);
            put_u32(sys, rec.record + 0x58, p.format);
            sys[rec.record + 0x5D] = p.levels;
        }
    }

    Ok(Repacked {
        gfx,
        layout,
        duplicates,
        held: sizes.iter().map(|&size| size as u64).sum(),
    })
}

fn keep(data: Cow<'_, [u8]>, kind: TexKind) -> (SqueezedTex<'_>, TexKind) {
    (SqueezedTex { data, patch: None }, kind)
}

fn inflate_segments(
    job: &TextureJob<'_>,
    sys_flags: u32,
    gfx_flags: u32,
) -> std::result::Result<(Vec<u8>, Vec<u8>), String> {
    let sys_size = size_from_flags(sys_flags) as usize;
    let gfx_size = size_from_flags(gfx_flags) as usize;

    let mut segments = Vec::with_capacity(sys_size + gfx_size);
    DeflateDecoder::new(&job.bytes[16..])
        .read_to_end(&mut segments)
        .map_err(|e| format!("deflate payload corrupt: {e}"))?;
    segments.resize(sys_size + gfx_size, 0);

    let gfx = segments.split_off(sys_size);
    Ok((segments, gfx))
}

fn deflate_container(
    job: &TextureJob<'_>,
    version: u32,
    sys_flags: u32,
    gfx_flags: u32,
    sys: &[u8],
    gfx: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(job.bytes.len());
    out.extend_from_slice(&RSC7_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&sys_flags.to_le_bytes());
    out.extend_from_slice(&gfx_flags.to_le_bytes());

    let mut encoder = DeflateEncoder::new(&mut out, Compression::default());
    encoder
        .write_all(sys)
        .and_then(|_| encoder.write_all(gfx))
        .and_then(|_| encoder.finish().map(|_| ()))
        .map_err(|e| format!("recompression failed: {e}"))?;
    Ok(out)
}

enum Settled {
    Win(Vec<u8>),
    RetryOnCpu,
    Loss(String),
}

fn settle(
    job: &TextureJob<'_>,
    out: Vec<u8>,
    gpu: Option<&GpuContext>,
    must_write: bool,
) -> Settled {
    if out.len() < job.bytes.len() || must_write {
        return Settled::Win(out);
    }
    if gpu.is_some() && out.len() < job.bytes.len() + job.bytes.len() / 16 {
        tracing::debug!(path = %job.path.display(), "near break-even with GPU mix, retrying CPU-only");
        return Settled::RetryOnCpu;
    }
    Settled::Loss(format!(
        "no size win after rebuild ({} in, {} out)",
        job.bytes.len(),
        out.len()
    ))
}

fn map_records<'a, T, F>(records: &'a [TexRecord], parallel: bool, f: F) -> Result<Vec<T>>
where
    T: Send,
    F: Fn(usize, &'a TexRecord) -> Result<T> + Sync + Send,
{
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

fn payload_extents(
    records: &[TexRecord],
    layout: &PageLayout,
    gfx_len: usize,
) -> FxHashMap<usize, usize> {
    let mut starts: Vec<usize> = records.iter().map(|rec| rec.data).collect();
    starts.sort_unstable();
    starts.dedup();
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let next = starts.get(i + 1).copied().unwrap_or(gfx_len);
            let page_end = layout
                .page_end(start as u64)
                .map_or(gfx_len, |end| end as usize);
            (start, next.min(page_end).max(start) - start)
        })
        .collect()
}

fn diffuse_pair_targets(records: &[TexRecord], policy: &Policy) -> FxHashMap<String, u32> {
    let mut targets = FxHashMap::default();
    if !policy.overdrive {
        return targets;
    }
    for rec in records {
        let path = Path::new(&rec.name);
        if matches!(classify_name(path), None | Some(TextureRole::Diffuse)) {
            let (tw, th) = target_dimensions(rec.width, rec.height, TextureRole::Diffuse, policy);
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
             ({needed} bytes) but only {} bytes were produced, refusing to emit a container \
             that would read past its graphics segment",
            rec.name,
            tex.data.len(),
        )));
    }
    Ok(())
}

fn record_bytes(rec: &TexRecord) -> u64 {
    pixel_layout(rec.format)
        .map(|layout| chain_len(rec.width, rec.height, rec.levels, layout) as u64)
        .unwrap_or(0)
}

fn resident_bytes(records: &[TexRecord]) -> u64 {
    records.iter().map(record_bytes).sum()
}

fn warn_if_bloated(path: &Path, resident: u64) {
    if resident >= BLOATED_DICT_BYTES {
        tracing::warn!(
            path = %path.display(),
            reason = %format!(
                "{} MiB of textures resident, around {} MiB is where FiveM starts to struggle",
                resident / MIB,
                BLOATED_DICT_BYTES / MIB,
            ),
            "fat dictionary"
        );
    }
}

fn odd_sided(records: &[TexRecord]) -> Vec<&str> {
    records
        .iter()
        .filter(|r| !(r.width.is_power_of_two() && r.height.is_power_of_two()))
        .map(|r| r.name.as_str())
        .collect()
}

fn warn_if_not_power_of_two(path: &Path, records: &[TexRecord]) {
    const SHOWN: usize = 4;

    let odd = odd_sided(records);
    if odd.is_empty() {
        return;
    }

    let mut detail = format!(
        "not a power of two: {}",
        odd[..odd.len().min(SHOWN)].join(", ")
    );
    if let Some(rest) = odd.len().checked_sub(SHOWN).filter(|&n| n > 0) {
        detail.push_str(&format!(" and {rest} more"));
    }
    detail.push_str(". Crop them at the source rather than leaving them to be resampled");

    tracing::warn!(path = %path.display(), reason = %detail, "odd size");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::squeezers::gta5::format::rage_mip_levels;

    const BC1: u32 = 0x3154_5844;
    const BC3: u32 = 0x3554_5844;

    fn rec(name: &str, width: u32, height: u32, format: u32) -> TexRecord {
        TexRecord {
            record: 0,
            name: name.to_owned(),
            width,
            height,
            format,
            levels: rage_mip_levels(width, height),
            data: 0,
        }
    }

    fn mib(bytes: u64) -> f64 {
        bytes as f64 / MIB as f64
    }

    #[test]
    fn a_full_chain_matches_the_published_figures() {
        for (side, bc1, bc3) in [(1024, 0.7, 1.3), (2048, 2.7, 5.3), (4096, 10.7, 21.3)] {
            let measured = mib(record_bytes(&rec("t", side, side, BC1)));
            assert!(
                (measured - bc1).abs() < 0.05,
                "{side} BC1: {measured:.2} MiB against a published {bc1}"
            );
            let measured = mib(record_bytes(&rec("t", side, side, BC3)));
            assert!(
                (measured - bc3).abs() < 0.05,
                "{side} BC3: {measured:.2} MiB against a published {bc3}"
            );
        }
    }

    #[test]
    fn the_wider_block_format_costs_exactly_double() {
        let bc1 = record_bytes(&rec("t", 512, 512, BC1));
        let bc3 = record_bytes(&rec("t", 512, 512, BC3));
        assert_eq!(bc3, bc1 * 2);
    }

    #[test]
    fn a_dictionary_costs_the_sum_of_its_textures() {
        let records = vec![
            rec("a", 1024, 1024, BC1),
            rec("b", 512, 512, BC3),
            rec("c", 256, 256, BC1),
        ];
        let expected: u64 = records.iter().map(record_bytes).sum();
        assert_eq!(resident_bytes(&records), expected);
        assert!(expected > 0);
    }

    #[test]
    fn an_unreadable_format_costs_nothing_rather_than_a_guess() {
        assert_eq!(record_bytes(&rec("t", 512, 512, 0xDEAD_BEEF)), 0);
    }

    #[test]
    fn halving_both_sides_quarters_the_cost() {
        let full = record_bytes(&rec("t", 1024, 1024, BC1));
        let half = record_bytes(&rec("t", 512, 512, BC1));

        assert!(half < full / 3, "{half} against {full}");
    }

    #[test]
    fn odd_sides_are_picked_out_by_name() {
        let records = vec![
            rec("fine", 1024, 1024, BC1),
            rec("tall", 1024, 768, BC1),
            rec("square_but_odd", 384, 384, BC1),
            rec("also_fine", 64, 256, BC1),
        ];
        assert_eq!(odd_sided(&records), ["tall", "square_but_odd"]);
    }

    #[test]
    fn a_dictionary_of_power_of_two_textures_says_nothing() {
        let records = vec![rec("a", 2048, 512, BC1), rec("b", 4, 4, BC3)];
        assert!(odd_sided(&records).is_empty());
    }
}
