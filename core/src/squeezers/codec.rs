use std::cell::RefCell;

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{PixelType, ResizeOptions, Resizer};
use image::{RgbaImage, imageops};
use image_dds::{ImageFormat, Mipmaps, Surface, SurfaceRgba8};

use crate::gpu::{self, GpuContext};
use crate::settings::Quality;

thread_local! {

    static RESIZER: RefCell<Resizer> = RefCell::new(Resizer::new());
}

pub fn resize(image: RgbaImage, width: u32, height: u32) -> RgbaImage {
    if (width, height) == (image.width(), image.height()) {
        return image;
    }
    resize_simd(image.as_raw(), image.width(), image.height(), width, height)
        .unwrap_or_else(|| imageops::resize(&image, width, height, imageops::FilterType::Lanczos3))
}

fn resize_simd(src: &[u8], src_w: u32, src_h: u32, width: u32, height: u32) -> Option<RgbaImage> {
    let src = ImageRef::new(src_w, src_h, src, PixelType::U8x4).ok()?;
    let mut dst = Image::new(width, height, PixelType::U8x4);
    RESIZER
        .with_borrow_mut(|r| r.resize(&src, &mut dst, &ResizeOptions::default()))
        .ok()?;
    RgbaImage::from_raw(width, height, dst.into_vec())
}

#[inline(always)]
pub(crate) fn has_alpha(image: &RgbaImage) -> bool {
    image
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .any(|chunk| chunk[3] < 255)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BcAlpha {
    Opaque,
    Binary,
    Smooth,
}

fn bc2_alpha_block(bytes: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (t, slot) in out.iter_mut().enumerate() {
        let byte = bytes[t / 2];
        let nibble = if t % 2 == 0 { byte & 0x0F } else { byte >> 4 };
        *slot = nibble * 17;
    }
    out
}

fn bc3_alpha_block(bytes: &[u8]) -> [u8; 16] {
    let (a0, a1) = (bytes[0] as u16, bytes[1] as u16);
    let mut pal = [0u16; 8];
    pal[0] = a0;
    pal[1] = a1;
    if a0 > a1 {
        for i in 1..7 {
            pal[i + 1] = ((7 - i as u16) * a0 + i as u16 * a1) / 7;
        }
    } else {
        for i in 1..5 {
            pal[i + 1] = ((5 - i as u16) * a0 + i as u16 * a1) / 5;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let mut bits = 0u64;
    for (i, &b) in bytes[2..8].iter().enumerate() {
        bits |= (b as u64) << (8 * i);
    }
    let mut out = [0u8; 16];
    for (t, slot) in out.iter_mut().enumerate() {
        let index = ((bits >> (3 * t)) & 0x7) as usize;
        *slot = pal[index] as u8;
    }
    out
}

pub(crate) fn bc_alpha_summary(
    blocks: &[u8],
    format: ImageFormat,
    width: u32,
    height: u32,
) -> Option<BcAlpha> {
    let expand: fn(&[u8]) -> [u8; 16] = match format {
        ImageFormat::BC2RgbaUnorm => bc2_alpha_block,
        ImageFormat::BC3RgbaUnorm => bc3_alpha_block,
        _ => return None,
    };
    let (bw, bh) = (width.div_ceil(4), height.div_ceil(4));
    let mut translucent = false;
    for by in 0..bh {
        for bx in 0..bw {
            let start = (by * bw + bx) as usize * 16;
            let block = blocks.get(start..start + 16)?;
            let alphas = expand(&block[..8]);
            for ty in 0..4 {
                for tx in 0..4 {
                    if bx * 4 + tx >= width || by * 4 + ty >= height {
                        continue;
                    }
                    let a = alphas[(ty * 4 + tx) as usize];
                    if a != 0 && a != 255 {
                        return Some(BcAlpha::Smooth);
                    }
                    if a != 255 {
                        translucent = true;
                    }
                }
            }
        }
    }
    Some(if translucent {
        BcAlpha::Binary
    } else {
        BcAlpha::Opaque
    })
}

pub(crate) fn decode_block(
    texels: &[u8],
    format: ImageFormat,
    width: u32,
    height: u32,
) -> Option<RgbaImage> {
    Surface {
        width,
        height,
        depth: 1,
        layers: 1,
        mipmaps: 1,
        image_format: format,
        data: texels,
    }
    .decode_rgba8()
    .ok()?
    .to_image(0)
    .ok()
}

pub(crate) fn resize_and_encode(
    image: RgbaImage,
    width: u32,
    height: u32,
    format: ImageFormat,
    quality: Quality,
    mip_levels: u32,
    gpu: Option<&GpuContext>,
) -> Option<Surface<Vec<u8>>> {
    let mip_levels = gpu::supported_mip_levels(width, height, mip_levels.max(1)).max(1);
    let image = match gpu {
        Some(ctx) => {
            let (source_w, source_h) = (image.width(), image.height());
            match gpu::resize_and_compress(
                ctx,
                image.into_raw(),
                source_w,
                source_h,
                width,
                height,
                format,
                quality,
                mip_levels,
            ) {
                Ok(data) => {
                    return Some(Surface {
                        width,
                        height,
                        depth: 1,
                        layers: 1,
                        mipmaps: mip_levels,
                        image_format: format,
                        data,
                    });
                }
                Err((error, pixels)) => {
                    if !matches!(error, gpu::GpuError::Busy) {
                        tracing::warn!(%error, "GPU pipeline failed, falling back to CPU");
                    }
                    RgbaImage::from_raw(source_w, source_h, pixels)?
                }
            }
        }
        None => image,
    };
    encode_block(&resize(image, width, height), format, quality, mip_levels)
}

pub fn encode_block(
    image: &RgbaImage,
    format: ImageFormat,
    quality: Quality,
    mip_levels: u32,
) -> Option<Surface<Vec<u8>>> {
    SurfaceRgba8::from_image(image)
        .encode(format, quality.into(), Mipmaps::GeneratedExact(mip_levels))
        .ok()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn resize_and_encode_from_blocks(
    blocks: &[u8],
    src_format: ImageFormat,
    source_w: u32,
    source_h: u32,
    width: u32,
    height: u32,
    out_format: ImageFormat,
    quality: Quality,
    mip_levels: u32,
    ctx: &GpuContext,
) -> Option<Surface<Vec<u8>>> {
    let mip_levels = gpu::supported_mip_levels(width, height, mip_levels.max(1)).max(1);
    match gpu::resize_and_compress_bc(
        ctx,
        blocks.to_vec(),
        src_format,
        source_w,
        source_h,
        width,
        height,
        out_format,
        quality,
        mip_levels,
    ) {
        Ok(data) => Some(Surface {
            width,
            height,
            depth: 1,
            layers: 1,
            mipmaps: mip_levels,
            image_format: out_format,
            data,
        }),
        Err((error, _)) => {
            if !matches!(error, gpu::GpuError::Busy) {
                tracing::warn!(%error, "GPU BC-source pipeline failed, falling back to CPU decode");
            }
            let image = decode_block(blocks, src_format, source_w, source_h)?;
            encode_block(
                &resize(image, width, height),
                out_format,
                quality,
                mip_levels,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn alpha_image(width: u32, height: u32, alpha: impl Fn(u32, u32) -> u8) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| Rgba([180, 60, 40, alpha(x, y)]))
    }

    #[test]
    fn has_alpha_detects_transparency() {
        assert!(!has_alpha(&alpha_image(4, 4, |_, _| 255)));
        assert!(has_alpha(&alpha_image(4, 4, |x, _| if x == 0 {
            0
        } else {
            255
        })));
    }

    #[test]
    fn bc3_alpha_summary_matches_content() {
        let cases = [
            (BcAlpha::Opaque, alpha_image(16, 16, |_, _| 255)),
            (
                BcAlpha::Binary,
                alpha_image(16, 16, |x, _| if x % 2 == 0 { 0 } else { 255 }),
            ),
            (
                BcAlpha::Smooth,
                alpha_image(16, 16, |x, _| (x * 16).min(255) as u8),
            ),
        ];
        for (want, image) in cases {
            let blocks = encode_block(&image, ImageFormat::BC3RgbaUnorm, Quality::Slow, 1)
                .expect("bc3 encode")
                .data;
            let got = bc_alpha_summary(&blocks, ImageFormat::BC3RgbaUnorm, 16, 16)
                .expect("bc3 is a summarisable format");
            assert_eq!(got, want);
        }
    }

    #[test]
    fn bc_alpha_summary_rejects_non_bc23() {
        assert!(bc_alpha_summary(&[0; 16], ImageFormat::BC1RgbaUnorm, 4, 4).is_none());
    }

    #[test]
    fn resize_keeps_content_and_dimensions() {
        let src = RgbaImage::from_pixel(64, 64, Rgba([12, 200, 90, 255]));
        let out = resize(src, 32, 32);
        assert_eq!((out.width(), out.height()), (32, 32));
        let px = out.get_pixel(16, 16).0;
        assert!(px[1] > 180 && px[0] < 40, "{px:?}");
    }
}
