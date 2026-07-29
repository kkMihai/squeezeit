use std::ffi::OsString;
use std::fs;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Result, SqueezeError};

pub const DEFAULT_BACKUP_DIR_NAME: &str = ".squeezeit-backup";

const MANIFEST_NAME: &str = "squeezeit-manifest.tsv";

pub const VAULT_SUFFIX: &str = ".sqzbak";

fn shelved(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(VAULT_SUFFIX);
    path.with_file_name(name)
}

fn publish(original: &Path, payload: &[u8], extension: &str) -> Result<PathBuf> {
    let published = original.with_extension(extension);
    let tmp = original.with_extension(format!("{extension}.sqz-tmp"));
    fs::write(&tmp, payload).map_err(|e| SqueezeError::io(&tmp, e))?;
    fs::rename(&tmp, &published).map_err(|e| SqueezeError::io(&tmp, e))?;
    Ok(published)
}

pub fn apply_in_place(original: &Path, payload: &[u8], extension: &str) -> Result<PathBuf> {
    let published = publish(original, payload, extension)?;
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

        let backup_path = shelved(&self.backup_root.join(&relative));
        if let Some(parent) = backup_path.parent() {
            fs::create_dir_all(parent).map_err(|e| SqueezeError::io(parent, e))?;
        }
        fs::rename(&original, &backup_path).map_err(|e| SqueezeError::io(&original, e))?;

        let published = publish(&original, payload, extension)?;

        let line = format!(
            "{}\t{}\n",
            relative.display(),
            relative.with_extension(extension).display()
        );
        let mut manifest = self.manifest.lock().expect("manifest mutex poisoned");
        manifest
            .write_all(line.as_bytes())
            .and_then(|_| manifest.flush())
            .map_err(|e| SqueezeError::io(self.backup_root.join(MANIFEST_NAME), e))?;

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
            let plain = self.backup_root.join(backup_rel);
            let backup_path = match shelved(&plain) {
                shelved if shelved.is_file() => shelved,
                _ => plain,
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    const STREAMED: &[&str] = &[
        "ytd", "ydd", "ydr", "yft", "rpf", "dds", "ymap", "ytyp", "ybn", "awc",
    ];

    fn scratch(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("sqz-vault-{label}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn shelved_appends_after_the_real_extension() {
        assert_eq!(
            shelved(Path::new("a/hair_006_u.ydd")),
            Path::new("a/hair_006_u.ydd.sqzbak")
        );
        assert_eq!(
            shelved(Path::new("a/vehicle.ytd")).extension().unwrap(),
            "sqzbak"
        );
    }

    #[test]
    fn nothing_in_the_vault_looks_like_a_streamable_asset() {
        let root = scratch("hidden");
        let asset = root.join("mp_m_freemode_01^hair_006_u.ydd");
        fs::write(&asset, b"original bytes").unwrap();

        let vault = BackupVault::new(&root, None).unwrap();
        vault.apply(&asset, b"squeezed", "ydd").unwrap();

        let mut seen = 0;
        for entry in walkdir::WalkDir::new(vault.backup_root())
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            seen += 1;
            let ext = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            assert!(
                !STREAMED.contains(&ext.as_str()),
                "{} would still be streamed by FiveM",
                entry.path().display()
            );
        }
        assert!(seen > 0, "vault should not be empty");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn restore_puts_the_original_bytes_back() {
        let root = scratch("restore");
        let asset = root.join("prop_crate.ytd");
        fs::write(&asset, b"original bytes").unwrap();

        let vault = BackupVault::new(&root, None).unwrap();
        vault.apply(&asset, b"squeezed", "ytd").unwrap();
        assert_eq!(fs::read(&asset).unwrap(), b"squeezed");
        assert!(BackupVault::has_backups(&root, None));

        let vault = BackupVault::new(&root, None).unwrap();
        assert_eq!(vault.restore_all().unwrap(), 1);
        assert_eq!(fs::read(&asset).unwrap(), b"original bytes");
        assert!(!root.join(DEFAULT_BACKUP_DIR_NAME).exists());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn restore_still_handles_a_legacy_vault() {
        let root = scratch("legacy");
        let asset = root.join("old.ytd");
        fs::write(&asset, b"new bytes").unwrap();

        let backup_root = root.join(DEFAULT_BACKUP_DIR_NAME);
        fs::create_dir_all(&backup_root).unwrap();
        fs::write(backup_root.join("old.ytd"), b"legacy original").unwrap();
        fs::write(backup_root.join(MANIFEST_NAME), "old.ytd\told.ytd\n").unwrap();

        let vault = BackupVault::new(&root, None).unwrap();
        assert_eq!(vault.restore_all().unwrap(), 1);
        assert_eq!(fs::read(&asset).unwrap(), b"legacy original");

        fs::remove_dir_all(&root).ok();
    }
}
