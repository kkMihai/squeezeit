use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

use crate::backup::{BackupVault, DEFAULT_BACKUP_DIR_NAME};
use crate::error::{Result, SqueezeError};
use crate::report::SqueezeReport;
use crate::settings::SqueezeSettings;
use crate::squeezers::{SqueezeOutcome, SqueezerRegistry, TextureJob};

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

pub fn run(
    root: &Path,
    registry: &SqueezerRegistry,
    settings: &SqueezeSettings,
    backup_dir: Option<PathBuf>,
    report: &SqueezeReport,
    cancel: &AtomicBool,
) -> Result<()> {
    let targets = collect_targets(root, registry);
    run_targets(
        &targets, root, registry, settings, backup_dir, report, cancel,
    )
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
    let gpu_counters_before = crate::gpu::counters();

    let vault = if settings.dry_run || !settings.make_backup {
        None
    } else {
        let vault_root = if root.is_file() {
            root.parent().unwrap_or(root)
        } else {
            root
        };
        Some(BackupVault::new(vault_root, backup_dir)?)
    };

    let (huge_targets, normal_targets): (Vec<PathBuf>, Vec<PathBuf>) =
        targets.iter().cloned().partition(|p| {
            std::fs::metadata(p)
                .map(|m| m.len() > HUGE_FILE_THRESHOLD)
                .unwrap_or(false)
        });

    let process_target = |path: &PathBuf, pool_saturated: bool| {
        if cancel.load(Ordering::Relaxed) {
            report.record_unchanged(0, false);
            debug!(path = %path.display(), reason = "cancelled", "skipped");
            return;
        }
        match process_one(path, registry, settings, vault.as_ref(), pool_saturated) {
            Ok(FileResult::Optimized {
                bytes_before,
                bytes_after,
            }) => {
                report.record_optimized(bytes_before, bytes_after);
                info!(path = %path.display(), bytes_before, bytes_after, "squeezed");
            }
            Ok(FileResult::Locked { bytes, reason }) => {
                report.record_unchanged(bytes, true);
                warn!(path = %path.display(), reason, "locked");
            }
            Ok(FileResult::Skipped { bytes, reason }) => {
                report.record_unchanged(bytes, false);
                debug!(path = %path.display(), reason = %reason, "skipped");
            }
            Err(error) => {
                report.record_failed();
                error!(path = %path.display(), error = %error, "failed");
            }
        }
    };

    for path in &huge_targets {
        process_target(path, false);
    }

    let normal_pool_saturated = normal_targets.len() >= rayon::current_num_threads();
    normal_targets.par_iter().for_each(|path| {
        process_target(path, normal_pool_saturated);
    });

    let (compressed, busy, failures) = crate::gpu::counters();
    let (compressed, busy, failures) = (
        compressed - gpu_counters_before.0,
        busy - gpu_counters_before.1,
        failures - gpu_counters_before.2,
    );
    if compressed > 0 || busy > 0 || failures > 0 {
        info!(
            compressed,
            busy,
            failures,
            "GPU pipeline: {compressed} textures compressed, {busy} routed to CPU (busy), {failures} failed to CPU"
        );
    }

    Ok(())
}

enum FileResult {
    Optimized { bytes_before: u64, bytes_after: u64 },
    Locked { bytes: u64, reason: &'static str },
    Skipped { bytes: u64, reason: String },
}

fn process_one(
    path: &Path,
    registry: &SqueezerRegistry,
    settings: &SqueezeSettings,
    vault: Option<&BackupVault>,
    pool_saturated: bool,
) -> Result<FileResult> {
    let squeezer = registry
        .find(path)
        .ok_or_else(|| SqueezeError::NoSqueezer(path.to_path_buf()))?;

    let bytes = fs::read(path).map_err(|e| SqueezeError::io(path, e))?;
    let bytes_before = bytes.len() as u64;

    let job = TextureJob {
        path,
        bytes: &bytes,
        asset_hint: None,
        pool_saturated,
    };
    match squeezer.squeeze(&job, settings)? {
        SqueezeOutcome::Optimized {
            bytes: payload,
            extension,
        } => {
            let bytes_after = payload.len() as u64;
            if let Some(vault) = vault {
                vault.apply(path, &payload, extension)?;
            } else if !settings.dry_run {
                crate::backup::apply_in_place(path, &payload, extension)?;
            }
            Ok(FileResult::Optimized {
                bytes_before,
                bytes_after,
            })
        }
        SqueezeOutcome::Locked { reason } => Ok(FileResult::Locked {
            bytes: bytes_before,
            reason,
        }),
        SqueezeOutcome::Skipped { reason } => Ok(FileResult::Skipped {
            bytes: bytes_before,
            reason,
        }),
    }
}
