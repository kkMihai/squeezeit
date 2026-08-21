use rayon::prelude::*;
use rpf_archive::archive::{RpfArchive, RpfEncryption, RpfEntryKind, RpfVersion};
use rpf_archive::tree::{FileRef, build_directory_tree, list_all_files};
use rpf_archive::writer::RpfBuilder;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

use crate::error::Result;
use crate::gpu::GpuContext;
use crate::gta5::{self, DrawableKind, squeeze_drawable, squeeze_ytd};
use crate::settings::SqueezeSettings;
use crate::squeezers::{SqueezeOutcome, Squeezer, TextureBytes, TextureJob, claims_extension};
const MAX_NESTING: usize = 4;

#[derive(Default)]
pub struct RpfSqueezer;

impl Squeezer for RpfSqueezer {
    fn claims(&self, path: &Path) -> bool {
        claims_extension(path, &["rpf"])
    }

    fn squeeze(
        &self,
        job: &TextureJob<'_>,
        settings: &SqueezeSettings,
        gpu: Option<&GpuContext>,
    ) -> Result<SqueezeOutcome> {
        let ctx = Context {
            settings,
            gpu,
            stats: Stats::default(),
        };

        let rebuilt = match rebuild(&ctx, job.path, job.bytes, 0, !job.pool_saturated) {
            Err(reason) => return Ok(SqueezeOutcome::skipped(reason)),
            Ok(rebuilt) => rebuilt,
        };
        let textures = ctx.stats.textures();

        if ctx.stats.optimized.load(Relaxed) == 0 {
            return Ok(SqueezeOutcome::skipped(format!(
                "no texture container improved ({} .ytd/.ydr/.ydd/.yft seen)",
                ctx.stats.seen.load(Relaxed)
            ))
            .with_textures(textures.discarded()));
        }

        if rebuilt.len() >= job.bytes.len() {
            return Ok(SqueezeOutcome::skipped(format!(
                "no size win after rebuild ({} in, {} out)",
                job.bytes.len(),
                rebuilt.len()
            ))
            .with_textures(textures.discarded()));
        }
        Ok(SqueezeOutcome::optimized(rebuilt, "rpf").with_textures(textures))
    }
}

struct Context<'a> {
    settings: &'a SqueezeSettings,
    gpu: Option<&'a GpuContext>,
    stats: Stats,
}

#[derive(Default)]
struct Stats {
    seen: AtomicUsize,
    optimized: AtomicUsize,
    textures_before: AtomicU64,
    textures_after: AtomicU64,
}

impl Stats {
    fn add_textures(&self, textures: TextureBytes) {
        self.textures_before.fetch_add(textures.before, Relaxed);
        self.textures_after.fetch_add(textures.after, Relaxed);
    }

    fn textures(&self) -> TextureBytes {
        TextureBytes {
            before: self.textures_before.load(Relaxed),
            after: self.textures_after.load(Relaxed),
        }
    }
}

fn rebuild(
    ctx: &Context<'_>,
    path: &Path,
    bytes: &[u8],
    depth: usize,
    may_parallel: bool,
) -> std::result::Result<Vec<u8>, String> {
    if depth > MAX_NESTING {
        return Err(format!("archives nested deeper than {MAX_NESTING} levels"));
    }

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive.rpf");
    let archive =
        RpfArchive::parse(bytes, name, None).map_err(|e| format!("unparseable archive: {e}"))?;

    if archive.encryption.is_encrypted() {
        return Err("encrypted archive, skipped".into());
    }
    if archive.version == RpfVersion::V8 {
        return Err("RPF8 (RDR2 next-gen) rebuild is not supported".into());
    }
    if let Some(entry) = archive.entries.iter().find(|e| {
        matches!(
            e.kind,
            RpfEntryKind::BinaryFile {
                is_encrypted: true,
                ..
            } | RpfEntryKind::ResourceFile {
                is_encrypted: true,
                ..
            }
        )
    }) {
        return Err(format!(
            "archive contains escrow/per-file-encrypted resource `{}`, rebuild would corrupt \
             it, left untouched",
            entry.name
        ));
    }

    let tree = build_directory_tree(&archive.entries);
    let files = list_all_files(&tree);

    let process = |file: &&FileRef| -> std::result::Result<(String, Vec<u8>), String> {
        let entry = &archive.entries[file.entry_index];
        let data = archive
            .extract_entry(bytes, entry, None)
            .map_err(|e| format!("cannot extract `{}`: {e}", file.path))?;

        let lower = file.name.to_ascii_lowercase();
        let entry_path = path.join(&file.path);
        let drawable = match lower.rsplit('.').next() {
            Some("ydr") => Some((DrawableKind::Drawable, "ydr")),
            Some("ydd") => Some((DrawableKind::Dictionary, "ydd")),
            Some("yft") => Some((DrawableKind::Fragment, "yft")),
            _ => None,
        };

        let out = if lower.ends_with(".ytd") || drawable.is_some() {
            ctx.stats.seen.fetch_add(1, Relaxed);
            let job = TextureJob {
                path: &entry_path,
                bytes: &data,
                asset_hint: gta5::family_from_filename(&entry_path),
                pool_saturated: true,
            };
            let result = match drawable {
                None => squeeze_ytd(&job, ctx.settings, ctx.gpu),
                Some((kind, ext)) => squeeze_drawable(&job, ctx.settings, kind, ext, ctx.gpu),
            };
            match result {
                Ok(outcome) => {
                    ctx.stats.add_textures(outcome.textures());
                    match outcome {
                        SqueezeOutcome::Optimized { bytes: new, .. } => {
                            ctx.stats.optimized.fetch_add(1, Relaxed);
                            new
                        }
                        _ => data,
                    }
                }
                Err(_) => data,
            }
        } else if lower.ends_with(".rpf") {
            rebuild(ctx, &entry_path, &data, depth + 1, false).unwrap_or(data)
        } else {
            data
        };

        Ok((file.path.clone(), out))
    };

    let builder = std::sync::Mutex::new(RpfBuilder::for_version(
        archive.version,
        RpfEncryption::None,
    ));
    let add = |(entry_path, data): (String, Vec<u8>)| {
        builder.lock().unwrap().add_file(&entry_path, data);
    };

    if may_parallel {
        files
            .par_iter()
            .try_for_each(|file| process(file).map(add))?;
    } else {
        for file in &files {
            add(process(file)?);
        }
    }

    builder
        .into_inner()
        .unwrap()
        .build(None)
        .map_err(|e| format!("archive rebuild failed: {e}"))
}
