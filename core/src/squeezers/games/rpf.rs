use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use rayon::prelude::*;
use rpf_archive::archive::{RpfArchive, RpfEncryption, RpfEntryKind, RpfVersion};
pub use rpf_archive::crypto::GtaKeys;
use rpf_archive::tree::{FileRef, build_directory_tree, list_all_files};
use rpf_archive::writer::RpfBuilder;

use crate::error::Result;
use crate::gpu::GpuContext;
use crate::gta5::{DrawableKind, squeeze_drawable, squeeze_ytd};
use crate::settings::SqueezeSettings;
use crate::squeezers::{HybridSqueezer, SqueezeOutcome, TextureJob};

const MAX_NESTING: usize = 4;

#[derive(Default)]
pub struct RpfSqueezer {
    keys: Option<Arc<GtaKeys>>,
}

impl RpfSqueezer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_keys(keys: Arc<GtaKeys>) -> Self {
        Self { keys: Some(keys) }
    }
}

impl HybridSqueezer for RpfSqueezer {
    fn name(&self) -> &'static str {
        "GTA V Archive Squeezer"
    }

    fn claims(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rpf"))
    }

    fn squeeze(
        &self,
        job: &TextureJob<'_>,
        settings: &SqueezeSettings,
        gpu: Option<&GpuContext>,
    ) -> Result<SqueezeOutcome> {
        let stats = Stats::default();
        let keys = self.keys.as_deref();
        match squeeze_archive(
            job.path,
            job.bytes,
            settings,
            keys,
            gpu,
            0,
            &stats,
            !job.pool_saturated,
        ) {
            Err(reason) => Ok(SqueezeOutcome::Skipped { reason }),
            Ok(rebuilt) => {
                if stats.optimized.load(Relaxed) == 0 {
                    return Ok(SqueezeOutcome::Skipped {
                        reason: format!(
                            "no texture container improved ({} .ytd/.ydr/.ydd/.yft seen)",
                            stats.seen.load(Relaxed)
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
    }
}

#[derive(Default)]
struct Stats {
    seen: AtomicUsize,
    optimized: AtomicUsize,
}

#[allow(clippy::too_many_arguments)]
fn squeeze_archive(
    path: &Path,
    bytes: &[u8],
    settings: &SqueezeSettings,
    keys: Option<&GtaKeys>,
    gpu: Option<&GpuContext>,
    depth: usize,
    stats: &Stats,
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
        RpfArchive::parse(bytes, name, keys).map_err(|e| format!("unparseable archive: {e}"))?;

    if archive.encryption.is_encrypted() && keys.is_none() {
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
            "archive contains escrow/per-file-encrypted resource `{}` — rebuild would corrupt it, left untouched",
            entry.name
        ));
    }

    let tree = build_directory_tree(&archive.entries);
    let files = list_all_files(&tree);

    let process_file = |file: &&FileRef| -> std::result::Result<(String, Vec<u8>), String> {
        let entry = &archive.entries[file.entry_index];
        let data = archive
            .extract_entry(bytes, entry, keys)
            .map_err(|e| format!("cannot extract `{}`: {e}", file.path))?;

        let lower = file.name.to_ascii_lowercase();
        let entry_path = path.join(&file.path);

        let drawable_kind = match lower.rsplit('.').next() {
            Some("ydr") => Some((DrawableKind::Drawable, "ydr")),
            Some("ydd") => Some((DrawableKind::Dictionary, "ydd")),
            Some("yft") => Some((DrawableKind::Fragment, "yft")),
            _ => None,
        };

        let out = if lower.ends_with(".ytd") || drawable_kind.is_some() {
            stats.seen.fetch_add(1, Relaxed);
            let job = TextureJob {
                path: &entry_path,
                bytes: &data,
                asset_hint: None,
                pool_saturated: true,
            };
            let result = match drawable_kind {
                None => squeeze_ytd(&job, settings, gpu),
                Some((kind, ext)) => squeeze_drawable(&job, settings, kind, ext, gpu),
            };
            match result {
                Ok(SqueezeOutcome::Optimized { bytes: new, .. }) => {
                    stats.optimized.fetch_add(1, Relaxed);
                    new
                }

                _ => data,
            }
        } else if lower.ends_with(".rpf") {
            squeeze_archive(
                &entry_path,
                &data,
                settings,
                keys,
                gpu,
                depth + 1,
                stats,
                false,
            )
            .unwrap_or(data)
        } else {
            data
        };

        Ok((file.path.clone(), out))
    };

    let builder = std::sync::Mutex::new(RpfBuilder::for_version(
        archive.version,
        RpfEncryption::None,
    ));

    if may_parallel {
        files
            .par_iter()
            .try_for_each(|file| -> std::result::Result<(), String> {
                let (entry_path, data) = process_file(file)?;
                builder.lock().unwrap().add_file(&entry_path, data);
                Ok(())
            })?;
    } else {
        for file in &files {
            let (entry_path, data) = process_file(file)?;
            builder.lock().unwrap().add_file(&entry_path, data);
        }
    }

    builder
        .into_inner()
        .unwrap()
        .build(None)
        .map_err(|e| format!("archive rebuild failed: {e}"))
}
