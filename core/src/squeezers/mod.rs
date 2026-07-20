pub mod codec;
pub mod games;
pub mod standard;
pub mod texture;

use std::path::Path;

use crate::error::Result;
use crate::settings::SqueezeSettings;

#[derive(Debug, Clone, Copy)]
pub struct TextureJob<'a> {
    pub path: &'a Path,

    pub bytes: &'a [u8],

    pub asset_hint: Option<crate::gta5::AssetFamily>,

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
    fn name(&self) -> &'static str;

    fn claims(&self, path: &Path) -> bool;

    fn squeeze(&self, job: &TextureJob<'_>, settings: &SqueezeSettings) -> Result<SqueezeOutcome>;
}

pub trait HybridSqueezer: Send + Sync {
    fn name(&self) -> &'static str;

    fn claims(&self, path: &Path) -> bool;

    fn squeeze(
        &self,
        job: &TextureJob<'_>,
        settings: &SqueezeSettings,
        gpu: Option<&crate::gpu::GpuContext>,
    ) -> Result<SqueezeOutcome>;
}

pub struct WithGpu<S> {
    inner: S,
    gpu: Option<std::sync::Arc<crate::gpu::GpuContext>>,
}

impl<S> WithGpu<S> {
    pub fn new(inner: S, gpu: Option<std::sync::Arc<crate::gpu::GpuContext>>) -> Self {
        Self { inner, gpu }
    }
}

impl<S: HybridSqueezer> Squeezer for WithGpu<S> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn claims(&self, path: &Path) -> bool {
        self.inner.claims(path)
    }

    fn squeeze(&self, job: &TextureJob<'_>, settings: &SqueezeSettings) -> Result<SqueezeOutcome> {
        let gpu = crate::gpu::select(settings.backend, self.gpu.as_deref());
        self.inner.squeeze(job, settings, gpu)
    }
}

#[derive(Default)]
pub struct SqueezerRegistry {
    squeezers: Vec<Box<dyn Squeezer>>,
}

impl SqueezerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_defaults() -> Self {
        Self::with_defaults_gpu(None)
    }

    pub fn with_defaults_gpu(gpu: Option<std::sync::Arc<crate::gpu::GpuContext>>) -> Self {
        let mut registry = Self::new();
        registry.register(WithGpu::new(crate::rpf::RpfSqueezer::new(), gpu.clone()));
        registry.register(WithGpu::new(crate::gta5::Gta5Squeezer, gpu.clone()));
        registry.register(WithGpu::new(crate::standard::StandardImageSqueezer, gpu));
        registry
    }

    pub fn with_gta_keys(keys: std::sync::Arc<crate::rpf::GtaKeys>) -> Self {
        Self::with_gta_keys_gpu(keys, None)
    }

    pub fn with_gta_keys_gpu(
        keys: std::sync::Arc<crate::rpf::GtaKeys>,
        gpu: Option<std::sync::Arc<crate::gpu::GpuContext>>,
    ) -> Self {
        let mut registry = Self::new();
        registry.register(WithGpu::new(
            crate::rpf::RpfSqueezer::with_keys(keys),
            gpu.clone(),
        ));
        registry.register(WithGpu::new(crate::gta5::Gta5Squeezer, gpu.clone()));
        registry.register(WithGpu::new(crate::standard::StandardImageSqueezer, gpu));
        registry
    }

    pub fn register(&mut self, squeezer: impl Squeezer + 'static) {
        self.squeezers.push(Box::new(squeezer));
    }

    pub fn find(&self, path: &Path) -> Option<&dyn Squeezer> {
        self.squeezers
            .iter()
            .map(Box::as_ref)
            .find(|s| s.claims(path))
    }

    pub fn claims(&self, path: &Path) -> bool {
        self.find(path).is_some()
    }
}
