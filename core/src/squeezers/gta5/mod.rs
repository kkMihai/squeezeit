mod dictionary;
mod format;
mod naming;
mod optimize;
mod raw;
mod shaders;
mod squeeze;

use std::path::Path;

use crate::error::Result;
use crate::gpu::GpuContext;
use crate::settings::SqueezeSettings;
use crate::squeezers::{SqueezeOutcome, Squeezer, TextureJob, claims_extension};
use raw::RSC7_MAGIC;

pub use naming::family_from_filename;
pub use shaders::AssetFamily;
pub(crate) use squeeze::{DrawableKind, squeeze_drawable, squeeze_ytd};

pub const EXTENSIONS: &[&str] = &["ytd", "ydr", "ydd", "yft"];

pub struct Gta5Squeezer;

impl Squeezer for Gta5Squeezer {
    fn claims(&self, path: &Path) -> bool {
        claims_extension(path, EXTENSIONS)
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
