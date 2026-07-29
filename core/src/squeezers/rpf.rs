use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use rayon::prelude::*;
use rpf_archive::archive::{RpfArchive, RpfEncryption, RpfEntryKind, RpfVersion};
use rpf_archive::tree::{FileRef, build_directory_tree, list_all_files};
use rpf_archive::writer::RpfBuilder;

pub use rpf_archive::crypto::GtaKeys;

use crate::error::Result;
use crate::gpu::GpuContext;
use crate::gta5::{self, DrawableKind, squeeze_drawable, squeeze_ytd};
use crate::settings::SqueezeSettings;
use crate::squeezers::{SqueezeOutcome, Squeezer, TextureJob, claims_extension};

const MAX_NESTING: usize = 4;

#[derive(Default)]
pub struct RpfSqueezer {
    keys: Option<Arc<GtaKeys>>,
}

impl RpfSqueezer {
    pub fn new(keys: Option<Arc<GtaKeys>>) -> Self {
        Self { keys }
    }
}

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
            keys: self.keys.as_deref(),
            gpu,
            stats: Stats::default(),
        };

        let rebuilt = match rebuild(&ctx, job.path, job.bytes, 0, !job.pool_saturated) {
            Err(reason) => return Ok(SqueezeOutcome::Skipped { reason }),
            Ok(rebuilt) => rebuilt,
        };

        if ctx.stats.optimized.load(Relaxed) == 0 {
            return Ok(SqueezeOutcome::Skipped {
                reason: format!(
                    "no texture container improved ({} .ytd/.ydr/.ydd/.yft seen)",
                    ctx.stats.seen.load(Relaxed)
                ),
            });
        }
        if rebuilt.len() >= job.bytes.len() {
            return Ok(SqueezeOutcome::Skipped {
                reason: format!(
                    "no size win after rebuild ({} in, {} out)",
                    job.bytes.len(),
                    rebuilt.len()
                ),
            });
        }
        Ok(SqueezeOutcome::Optimized {
            bytes: rebuilt,
            extension: "rpf",
        })
    }
}

struct Context<'a> {
    settings: &'a SqueezeSettings,
    keys: Option<&'a GtaKeys>,
    gpu: Option<&'a GpuContext>,
    stats: Stats,
}

#[derive(Default)]
struct Stats {
    seen: AtomicUsize,
    optimized: AtomicUsize,
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
    let archive = RpfArchive::parse(bytes, name, ctx.keys)
        .map_err(|e| format!("unparseable archive: {e}"))?;

    if archive.encryption.is_encrypted() && ctx.keys.is_none() {
        return Err(format!(
            "{:?}-encrypted archive — provide GTA keys (--gta-keys-dir / --gta-exe)",
            archive.encryption
        ));
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
            "archive contains escrow/per-file-encrypted resource `{}` — rebuild would corrupt \
             it, left untouched",
            entry.name
        ));
    }

    let tree = build_directory_tree(&archive.entries);
    let files = list_all_files(&tree);

    let process = |file: &&FileRef| -> std::result::Result<(String, Vec<u8>), String> {
        let entry = &archive.entries[file.entry_index];
        let data = archive
            .extract_entry(bytes, entry, ctx.keys)
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
                Ok(SqueezeOutcome::Optimized { bytes: new, .. }) => {
                    ctx.stats.optimized.fetch_add(1, Relaxed);
                    new
                }
                _ => data,
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
