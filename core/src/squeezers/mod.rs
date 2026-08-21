pub mod codec;
pub mod gta5;
pub mod policy;
pub mod rpf;
pub mod standard;
pub mod texture;

use crate::error::{Result, SqueezeError};
use crate::gpu::{self, GpuContext};
use crate::settings::SqueezeSettings;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub struct TextureJob<'a> {
    pub path: &'a Path,
    pub bytes: &'a [u8],
    pub asset_hint: Option<gta5::AssetFamily>,
    pub pool_saturated: bool,
}

impl TextureJob<'_> {
    pub fn extension(&self) -> Option<String> {
        self.path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextureBytes {
    pub before: u64,
    pub after: u64,
}

impl TextureBytes {
    pub const ZERO: Self = Self {
        before: 0,
        after: 0,
    };

    pub fn unchanged(bytes: u64) -> Self {
        Self {
            before: bytes,
            after: bytes,
        }
    }

    pub fn saved(&self) -> u64 {
        self.before.saturating_sub(self.after)
    }

    pub fn add(&mut self, other: Self) {
        self.before = self.before.saturating_add(other.before);
        self.after = self.after.saturating_add(other.after);
    }

    pub fn discarded(self) -> Self {
        Self::unchanged(self.before)
    }
}

#[derive(Debug)]
pub enum SqueezeOutcome {
    Optimized {
        bytes: Vec<u8>,
        extension: &'static str,
        textures: TextureBytes,
    },
    Locked {
        reason: &'static str,
        textures: TextureBytes,
    },
    Skipped {
        reason: String,
        textures: TextureBytes,
    },
}

impl SqueezeOutcome {
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self::Skipped {
            reason: reason.into(),
            textures: TextureBytes::ZERO,
        }
    }

    pub fn locked(reason: &'static str) -> Self {
        Self::Locked {
            reason,
            textures: TextureBytes::ZERO,
        }
    }

    pub fn optimized(bytes: Vec<u8>, extension: &'static str) -> Self {
        Self::Optimized {
            bytes,
            extension,
            textures: TextureBytes::ZERO,
        }
    }

    #[must_use]
    pub fn with_textures(self, textures: TextureBytes) -> Self {
        match self {
            Self::Optimized {
                bytes, extension, ..
            } => Self::Optimized {
                bytes,
                extension,
                textures,
            },
            Self::Locked { reason, .. } => Self::Locked { reason, textures },
            Self::Skipped { reason, .. } => Self::Skipped { reason, textures },
        }
    }

    pub fn textures(&self) -> TextureBytes {
        match self {
            Self::Optimized { textures, .. }
            | Self::Locked { textures, .. }
            | Self::Skipped { textures, .. } => *textures,
        }
    }
}

pub trait Squeezer: Send + Sync {
    fn claims(&self, path: &Path) -> bool;

    fn squeeze(
        &self,
        job: &TextureJob<'_>,
        settings: &SqueezeSettings,
        gpu: Option<&GpuContext>,
    ) -> Result<SqueezeOutcome>;
}

pub struct SqueezerRegistry {
    squeezers: Vec<Box<dyn Squeezer>>,
    gpu: Option<Arc<GpuContext>>,
}

impl SqueezerRegistry {
    pub fn new(gpu: Option<Arc<GpuContext>>) -> Self {
        Self {
            squeezers: vec![
                Box::new(rpf::RpfSqueezer),
                Box::new(gta5::Gta5Squeezer),
                Box::new(standard::StandardImageSqueezer),
            ],
            gpu,
        }
    }

    pub fn register(&mut self, squeezer: impl Squeezer + 'static) {
        self.squeezers.insert(0, Box::new(squeezer));
    }

    pub fn claims(&self, path: &Path) -> bool {
        self.squeezers.iter().any(|s| s.claims(path))
    }

    pub fn squeeze(
        &self,
        job: &TextureJob<'_>,
        settings: &SqueezeSettings,
    ) -> Result<SqueezeOutcome> {
        let squeezer = self
            .squeezers
            .iter()
            .find(|s| s.claims(job.path))
            .ok_or_else(|| SqueezeError::NoSqueezer(job.path.to_path_buf()))?;
        squeezer.squeeze(
            job,
            settings,
            gpu::select(settings.backend, self.gpu.as_deref()),
        )
    }
}

pub(crate) fn claims_extension(path: &Path, wanted: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| wanted.iter().any(|c| c.eq_ignore_ascii_case(ext)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_outcome_carries_a_measurement() {
        let textures = TextureBytes {
            before: 900,
            after: 300,
        };
        for outcome in [
            SqueezeOutcome::optimized(vec![1, 2, 3], "ytd"),
            SqueezeOutcome::locked("render target"),
            SqueezeOutcome::skipped("already optimal"),
        ] {
            assert_eq!(outcome.textures(), TextureBytes::ZERO);
            assert_eq!(outcome.with_textures(textures).textures(), textures);
        }
    }

    #[test]
    fn attaching_a_measurement_keeps_the_payload() {
        let outcome =
            SqueezeOutcome::optimized(vec![7; 4], "ydd").with_textures(TextureBytes::unchanged(64));
        match outcome {
            SqueezeOutcome::Optimized {
                bytes, extension, ..
            } => {
                assert_eq!(bytes, vec![7; 4]);
                assert_eq!(extension, "ydd");
            }
            other => panic!("expected Optimized, got {other:?}"),
        }
    }

    #[test]
    fn a_discarded_rebuild_reports_no_saving() {
        let measured = TextureBytes {
            before: 900,
            after: 300,
        };
        assert_eq!(measured.saved(), 600);
        assert_eq!(measured.discarded().saved(), 0);
        assert_eq!(measured.discarded().after, 900);
    }

    #[test]
    fn a_container_of_containers_adds_its_children_up() {
        let mut total = TextureBytes::ZERO;
        total.add(TextureBytes {
            before: 100,
            after: 40,
        });
        total.add(TextureBytes::unchanged(60));
        assert_eq!(total.before, 160);
        assert_eq!(total.after, 100);
        assert_eq!(total.saved(), 60);
    }

    #[test]
    fn a_texture_that_grew_reports_zero_saved() {
        let grew = TextureBytes {
            before: 10,
            after: 99,
        };
        assert_eq!(grew.saved(), 0);
    }
}
