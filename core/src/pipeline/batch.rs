use crate::backup::{BackupVault, DEFAULT_BACKUP_DIR_NAME};
use crate::error::{Result, SqueezeError};
use crate::log::FILE_RESULT;
use crate::report::SqueezeReport;
use crate::settings::SqueezeSettings;
use crate::squeezers::{SqueezeOutcome, SqueezerRegistry, TextureBytes, TextureJob};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;
const HUGE_FILE_THRESHOLD: u64 = 250 * 1024 * 1024;

pub fn collect_targets(root: &Path, registry: &SqueezerRegistry) -> Vec<PathBuf> {
    if root.is_file() {
        return if registry.claims(root) {
            vec![root.to_path_buf()]
        } else {
            Vec::new()
        };
    }

    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !(entry.file_type().is_dir() && entry.file_name() == DEFAULT_BACKUP_DIR_NAME)
        })
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| registry.claims(path))
        .collect()
}

pub fn run_targets(
    targets: &[PathBuf],
    root: &Path,
    registry: &SqueezerRegistry,
    settings: &SqueezeSettings,
    backup_dir: Option<PathBuf>,
    report: &SqueezeReport,
    cancel: &AtomicBool,
) -> Result<()> {
    report.begin(targets.len() as u64);
    let gpu_before = crate::gpu::counters();

    let vault = if settings.dry_run || !settings.backup {
        None
    } else {
        let vault_root = if root.is_file() {
            root.parent().unwrap_or(root)
        } else {
            root
        };
        Some(BackupVault::new(vault_root, backup_dir)?)
    };

    let (huge, normal): (Vec<&PathBuf>, Vec<&PathBuf>) = targets.iter().partition(|p| {
        fs::metadata(p)
            .map(|m| m.len() > HUGE_FILE_THRESHOLD)
            .unwrap_or(false)
    });

    let run_one = |path: &Path, pool_saturated: bool| {
        if cancel.load(Ordering::Relaxed) {
            report.record_unchanged(0, false, TextureBytes::ZERO);
            debug!(target: FILE_RESULT, path = %path.display(), reason = "cancelled", "skipped");
            return;
        }
        match process_one(path, registry, settings, vault.as_ref(), pool_saturated) {
            Ok(FileResult::Optimized {
                bytes_before,
                bytes_after,
                textures,
            }) => {
                report.record_optimized(bytes_before, bytes_after, textures);
                info!(target: FILE_RESULT, path = %path.display(), bytes_before, bytes_after, "squeezed");
            }
            Ok(FileResult::Locked {
                bytes,
                reason,
                textures,
            }) => {
                report.record_unchanged(bytes, true, textures);
                warn!(target: FILE_RESULT, path = %path.display(), reason, "locked");
            }
            Ok(FileResult::Skipped {
                bytes,
                reason,
                textures,
            }) => {
                report.record_unchanged(bytes, false, textures);
                debug!(target: FILE_RESULT, path = %path.display(), reason = %reason, "skipped");
            }
            Err(error) => {
                report.record_failed();
                error!(target: FILE_RESULT, path = %path.display(), error = %error, "failed");
            }
        }
    };

    for path in huge {
        run_one(path, false);
    }

    let saturated = normal.len() >= rayon::current_num_threads();
    normal.par_iter().for_each(|path| run_one(path, saturated));

    let after = crate::gpu::counters();
    let (compressed, busy, failures) = (
        after.0 - gpu_before.0,
        after.1 - gpu_before.1,
        after.2 - gpu_before.2,
    );
    if compressed > 0 || busy > 0 || failures > 0 {
        info!(
            compressed,
            busy,
            failures,
            "GPU pipeline: {compressed} textures compressed, {busy} routed to CPU (busy), \
             {failures} failed to CPU"
        );
    }

    Ok(())
}

enum FileResult {
    Optimized {
        bytes_before: u64,
        bytes_after: u64,
        textures: TextureBytes,
    },
    Locked {
        bytes: u64,
        reason: &'static str,
        textures: TextureBytes,
    },
    Skipped {
        bytes: u64,
        reason: String,
        textures: TextureBytes,
    },
}

fn process_one(
    path: &Path,
    registry: &SqueezerRegistry,
    settings: &SqueezeSettings,
    vault: Option<&BackupVault>,
    pool_saturated: bool,
) -> Result<FileResult> {
    let bytes = fs::read(path).map_err(|e| SqueezeError::io(path, e))?;
    let bytes_before = bytes.len() as u64;

    let job = TextureJob {
        path,
        bytes: &bytes,
        asset_hint: crate::gta5::family_from_filename(path),
        pool_saturated,
    };

    match registry.squeeze(&job, settings)? {
        SqueezeOutcome::Optimized {
            bytes: payload,
            extension,
            textures,
        } => {
            let bytes_after = payload.len() as u64;
            match vault {
                Some(vault) => vault.apply(path, &payload, extension)?,
                None if !settings.dry_run => {
                    crate::backup::apply_in_place(path, &payload, extension)?
                }
                None => path.to_path_buf(),
            };
            Ok(FileResult::Optimized {
                bytes_before,
                bytes_after,
                textures,
            })
        }
        SqueezeOutcome::Locked { reason, textures } => Ok(FileResult::Locked {
            bytes: bytes_before,
            reason,
            textures,
        }),
        SqueezeOutcome::Skipped { reason, textures } => Ok(FileResult::Skipped {
            bytes: bytes_before,
            reason,
            textures,
        }),
    }
}
