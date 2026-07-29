pub mod codec;
pub mod gta5;
pub mod policy;
pub mod rpf;
pub mod standard;
pub mod texture;

use std::path::Path;
use std::sync::Arc;

use crate::error::{Result, SqueezeError};
use crate::gpu::{self, GpuContext};
use crate::settings::SqueezeSettings;

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

#[derive(Debug)]
pub enum SqueezeOutcome {
    Optimized {
        bytes: Vec<u8>,
        extension: &'static str,
    },
    Locked {
        reason: &'static str,
    },
    Skipped {
        reason: String,
    },
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
    pub fn new(gpu: Option<Arc<GpuContext>>, gta_keys: Option<Arc<rpf::GtaKeys>>) -> Self {
        Self {
            squeezers: vec![
                Box::new(rpf::RpfSqueezer::new(gta_keys)),
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
