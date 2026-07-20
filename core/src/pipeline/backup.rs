use std::fs;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Result, SqueezeError};

pub const DEFAULT_BACKUP_DIR_NAME: &str = ".squeezeit-backup";

const MANIFEST_NAME: &str = "squeezeit-manifest.tsv";

pub fn apply_in_place(original: &Path, payload: &[u8], extension: &str) -> Result<PathBuf> {
    let published = original.with_extension(extension);
    let tmp = original.with_extension(format!("{extension}.sqz-tmp"));
    fs::write(&tmp, payload).map_err(|e| SqueezeError::io(&tmp, e))?;
    fs::rename(&tmp, &published).map_err(|e| SqueezeError::io(&tmp, e))?;
    if published != original {
        fs::remove_file(original).map_err(|e| SqueezeError::io(original, e))?;
    }
    Ok(published)
}

pub struct BackupVault {
    root: PathBuf,
    backup_root: PathBuf,
    manifest: Mutex<BufWriter<fs::File>>,
}

impl BackupVault {
    pub fn new(root: &Path, backup_root: Option<PathBuf>) -> Result<Self> {
        let root = root.canonicalize().map_err(|e| SqueezeError::io(root, e))?;
        let backup_root = backup_root.unwrap_or_else(|| root.join(DEFAULT_BACKUP_DIR_NAME));
        fs::create_dir_all(&backup_root).map_err(|e| SqueezeError::io(&backup_root, e))?;

        let manifest_path = backup_root.join(MANIFEST_NAME);
        let manifest = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&manifest_path)
            .map_err(|e| SqueezeError::io(&manifest_path, e))?;

        Ok(Self {
            root,
            backup_root,
            manifest: Mutex::new(BufWriter::new(manifest)),
        })
    }

    pub fn backup_root(&self) -> &Path {
        &self.backup_root
    }

    pub fn has_backups(root: &Path, backup_root: Option<&Path>) -> bool {
        let backup_root = backup_root
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.join(DEFAULT_BACKUP_DIR_NAME));
        fs::read_to_string(backup_root.join(MANIFEST_NAME))
            .is_ok_and(|m| m.lines().any(|l| l.contains('\t')))
    }

    pub fn apply(&self, original: &Path, payload: &[u8], extension: &str) -> Result<PathBuf> {
        let original = original
            .canonicalize()
            .map_err(|e| SqueezeError::io(original, e))?;
        let relative = original
            .strip_prefix(&self.root)
            .map_err(|_| SqueezeError::OutsideRoot {
                path: original.clone(),
            })?
            .to_path_buf();

        let backup_path = self.backup_root.join(&relative);
        if let Some(parent) = backup_path.parent() {
            fs::create_dir_all(parent).map_err(|e| SqueezeError::io(parent, e))?;
        }
        fs::rename(&original, &backup_path).map_err(|e| SqueezeError::io(&original, e))?;

        let published = original.with_extension(extension);
        let tmp = original.with_extension(format!("{extension}.sqz-tmp"));
        fs::write(&tmp, payload).map_err(|e| SqueezeError::io(&tmp, e))?;
        fs::rename(&tmp, &published).map_err(|e| SqueezeError::io(&tmp, e))?;

        let published_relative = relative.with_extension(extension);
        let line = format!("{}\t{}\n", relative.display(), published_relative.display());
        {
            let mut manifest = self.manifest.lock().expect("manifest mutex poisoned");
            manifest
                .write_all(line.as_bytes())
                .and_then(|_| manifest.flush())
                .map_err(|e| SqueezeError::io(self.backup_root.join(MANIFEST_NAME), e))?;
        }

        Ok(published)
    }

    pub fn restore_all(self) -> Result<usize> {
        let manifest_path = self.backup_root.join(MANIFEST_NAME);
        let contents =
            fs::read_to_string(&manifest_path).map_err(|e| SqueezeError::io(&manifest_path, e))?;

        let mut restored = 0;
        for line in contents.lines() {
            let Some((backup_rel, published_rel)) = line.split_once('\t') else {
                continue;
            };
            let backup_path = self.backup_root.join(backup_rel);
            let published_path = self.root.join(published_rel);
            let original_path = self.root.join(backup_rel);

            if !backup_path.is_file() {
                continue;
            }
            if published_path.is_file() {
                fs::remove_file(&published_path)
                    .map_err(|e| SqueezeError::io(&published_path, e))?;
            }
            if let Some(parent) = original_path.parent() {
                fs::create_dir_all(parent).map_err(|e| SqueezeError::io(parent, e))?;
            }
            fs::rename(&backup_path, &original_path)
                .map_err(|e| SqueezeError::io(&backup_path, e))?;
            restored += 1;
        }

        let Self {
            backup_root,
            manifest,
            ..
        } = self;
        drop(manifest);
        fs::remove_dir_all(&backup_root).map_err(|e| SqueezeError::io(&backup_root, e))?;
        Ok(restored)
    }
}
