use image_dds::ImageFormat;
use thiserror::Error;

use crate::settings::{Backend, Quality};

#[derive(Debug, Error)]
pub enum GpuError {
    #[error("GPU feature is not enabled")]
    Disabled,
    #[error("GPU compressor busy")]
    Busy,
}

pub struct GpuContext;

impl GpuContext {
    pub(crate) fn supports_bc_source(&self, _format: ImageFormat) -> bool {
        false
    }

    pub fn create_standalone() -> Result<Self, GpuError> {
        Err(GpuError::Disabled)
    }

    pub fn warm_up(&self) {}
}

pub fn select<'a>(_backend: Backend, _gpu: Option<&'a GpuContext>) -> Option<&'a GpuContext> {
    None
}

pub fn counters() -> (u64, u64, u64) {
    (0, 0, 0)
}

pub fn supported_mip_levels(width: u32, height: u32, requested: u32) -> u32 {
    let mut levels = 0;
    while levels < requested {
        let (w, h) = (width >> levels, height >> levels);
        if w < 4 || h < 4 || w % 4 != 0 || h % 4 != 0 {
            break;
        }
        levels += 1;
    }
    levels
}

#[allow(clippy::too_many_arguments)]
pub fn resize_and_compress(
    _ctx: &GpuContext,
    rgba: Vec<u8>,
    _source_w: u32,
    _source_h: u32,
    _target_w: u32,
    _target_h: u32,
    _format: ImageFormat,
    _quality: Quality,
    _mip_levels: u32,
) -> Result<Vec<u8>, (GpuError, Vec<u8>)> {
    Err((GpuError::Disabled, rgba))
}

#[allow(clippy::too_many_arguments)]
pub fn resize_and_compress_bc(
    _ctx: &GpuContext,
    blocks: Vec<u8>,
    _src_format: ImageFormat,
    _source_w: u32,
    _source_h: u32,
    _target_w: u32,
    _target_h: u32,
    _out_format: ImageFormat,
    _quality: Quality,
    _mip_levels: u32,
) -> Result<Vec<u8>, (GpuError, Vec<u8>)> {
    Err((GpuError::Disabled, blocks))
}
