mod dictionary;
mod format;
mod naming;
mod optimize;
mod raw;
mod shaders;
mod squeeze;
mod verify;

use crate::error::Result;
use crate::gpu::GpuContext;
use crate::settings::SqueezeSettings;
use crate::squeezers::{SqueezeOutcome, Squeezer, TextureJob, claims_extension};
use raw::RSC7_MAGIC;
use std::path::Path;

pub use naming::family_from_filename;
pub use shaders::AssetFamily;
pub(crate) use squeeze::{DrawableKind, squeeze_drawable, squeeze_ytd};
pub use verify::{ContainerInfo, TextureInfo, inspect_bytes, verify_bytes};

pub fn resolve_family(detected: AssetFamily, hint: Option<AssetFamily>) -> AssetFamily {
    match (detected, hint) {
        (AssetFamily::Generic, Some(hint)) => hint,
        (AssetFamily::PedCloth, Some(AssetFamily::PedHair)) => AssetFamily::PedHair,
        _ => detected,
    }
}

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
            return Ok(SqueezeOutcome::skipped(
                "not an RSC7 container (raw/encrypted resource?), left untouched",
            ));
        }
        match job.extension().as_deref() {
            Some("ydr") => squeeze_drawable(job, settings, DrawableKind::Drawable, "ydr", gpu),
            Some("ydd") => squeeze_drawable(job, settings, DrawableKind::Dictionary, "ydd", gpu),
            Some("yft") => squeeze_drawable(job, settings, DrawableKind::Fragment, "yft", gpu),
            _ => squeeze_ytd(job, settings, gpu),
        }
    }
}
