mod dictionary;
mod format;
mod optimize;
mod raw;
mod shaders;
mod squeeze;

use std::path::Path;

use raw::RSC7_MAGIC;

use crate::error::Result;
use crate::gpu::GpuContext;
use crate::settings::SqueezeSettings;
use crate::squeezers::{HybridSqueezer, SqueezeOutcome, TextureJob};

pub use dictionary::{TextureInfo, inspect_ytd};
pub use shaders::AssetFamily;
pub(crate) use squeeze::{DrawableKind, squeeze_drawable, squeeze_ytd};

pub struct Gta5Squeezer;

impl HybridSqueezer for Gta5Squeezer {
    fn name(&self) -> &'static str {
        "GTA V Squeezer"
    }

    fn claims(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                ["ytd", "ydr", "ydd", "yft"]
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(ext))
            })
    }

    fn squeeze(
        &self,
        job: &TextureJob<'_>,
        settings: &SqueezeSettings,
        gpu: Option<&GpuContext>,
    ) -> Result<SqueezeOutcome> {
        if job.bytes.get(..4) != Some(&RSC7_MAGIC) {
            return Ok(SqueezeOutcome::Skipped {
                reason: "not an RSC7 container (raw/encrypted resource?) — left untouched".into(),
            });
        }
        match job.extension().as_deref() {
            Some("ydr") => squeeze_drawable(job, settings, DrawableKind::Drawable, "ydr", gpu),
            Some("ydd") => squeeze_drawable(job, settings, DrawableKind::Dictionary, "ydd", gpu),
            Some("yft") => squeeze_drawable(job, settings, DrawableKind::Fragment, "yft", gpu),
            _ => squeeze_ytd(job, settings, gpu),
        }
    }
}
