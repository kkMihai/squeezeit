use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, RgbaImage};
use image_dds::ImageFormat;
use image_dds::ddsfile::Dds;

use crate::error::{Result, SqueezeError};
use crate::gpu::GpuContext;
use crate::settings::SqueezeSettings;
use crate::squeezers::codec;
use crate::squeezers::{HybridSqueezer, SqueezeOutcome, TextureJob};
use crate::texture::{TextureRole, classify_name, classify_pixels, target_dimensions};

const CLAIMED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tga", "bmp", "dds"];

pub struct StandardImageSqueezer;

impl HybridSqueezer for StandardImageSqueezer {
    fn name(&self) -> &'static str {
        "Standard Image Squeezer"
    }

    fn claims(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                CLAIMED_EXTENSIONS
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
        let name_role = classify_name(job.path);

        if name_role == Some(TextureRole::ScriptRenderTarget) {
            return Ok(SqueezeOutcome::Locked {
                reason: "script_rt render target — dynamic UI surface must stay native",
            });
        }

        let source_ext = job.extension().unwrap_or_default();
        let image = decode(job, &source_ext)?;

        let role = name_role
            .or_else(|| classify_pixels(image.as_raw(), image.width(), image.height()))
            .unwrap_or(TextureRole::Diffuse);

        let gpu = if matches!(role, TextureRole::Livery | TextureRole::Weapon) {
            None
        } else {
            gpu
        };

        let (width, height) = (image.width(), image.height());
        let (target_w, target_h) = target_dimensions(width, height, role, settings);

        let keep_source =
            settings.keep_source_format && !settings.force_convert && source_ext != "dds";
        let payload = if keep_source {
            let image = codec::resize(image, target_w, target_h);
            Payload {
                bytes: encode_source_format(image, &source_ext, job.path)?,
                extension: leak_free_extension(&source_ext),
            }
        } else {
            Payload {
                bytes: encode_dds(image, target_w, target_h, role, settings, job.path, gpu)?,
                extension: "dds",
            }
        };

        let converts_format = !payload.extension.eq_ignore_ascii_case(&source_ext);
        if payload.bytes.len() >= job.bytes.len() && !(settings.force_convert && converts_format) {
            return Ok(SqueezeOutcome::Skipped {
                reason: format!(
                    "already optimal ({} bytes in, {} bytes out)",
                    job.bytes.len(),
                    payload.bytes.len()
                ),
            });
        }

        Ok(SqueezeOutcome::Optimized {
            bytes: payload.bytes,
            extension: payload.extension,
        })
    }
}

struct Payload {
    bytes: Vec<u8>,
    extension: &'static str,
}

fn leak_free_extension(ext: &str) -> &'static str {
    CLAIMED_EXTENSIONS
        .iter()
        .copied()
        .find(|c| c.eq_ignore_ascii_case(ext))
        .expect("claimed extension is always in CLAIMED_EXTENSIONS")
}

fn decode(job: &TextureJob<'_>, source_ext: &str) -> Result<RgbaImage> {
    if source_ext == "dds" {
        let dds =
            Dds::read(&mut Cursor::new(job.bytes)).map_err(|source| SqueezeError::DdsParse {
                path: job.path.to_path_buf(),
                source,
            })?;
        image_dds::image_from_dds(&dds, 0).map_err(|source| SqueezeError::DdsDecode {
            path: job.path.to_path_buf(),
            source,
        })
    } else {
        if let Some(image) = decode_png_simd(job.bytes) {
            return Ok(image);
        }
        Ok(image::load_from_memory(job.bytes)
            .map_err(|source| SqueezeError::Decode {
                path: job.path.to_path_buf(),
                source,
            })?
            .into_rgba8())
    }
}

const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];

fn decode_png_simd(bytes: &[u8]) -> Option<RgbaImage> {
    use zune_png::zune_core::colorspace::ColorSpace;
    use zune_png::zune_core::result::DecodingResult;

    if !bytes.starts_with(&PNG_MAGIC) {
        return None;
    }

    let mut decoder = zune_png::PngDecoder::new(bytes);
    let result = match decoder.decode() {
        Ok(result) => result,
        Err(error) => {
            tracing::debug!(%error, "zune-png decode failed — using the image crate");
            return None;
        }
    };
    let (width, height) = decoder.get_dimensions()?;
    let pixels = width.checked_mul(height)?;
    let DecodingResult::U8(data) = result else {
        return None;
    };

    let rgba = match decoder.get_colorspace()? {
        ColorSpace::RGBA => data,
        ColorSpace::RGB => {
            let mut rgba = Vec::with_capacity(pixels * 4);
            for px in data.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            rgba
        }
        ColorSpace::Luma => {
            let mut rgba = Vec::with_capacity(pixels * 4);
            for &l in &data {
                rgba.extend_from_slice(&[l, l, l, 255]);
            }
            rgba
        }
        ColorSpace::LumaA => {
            let mut rgba = Vec::with_capacity(pixels * 4);
            for px in data.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            rgba
        }
        _ => return None,
    };

    RgbaImage::from_raw(width as u32, height as u32, rgba)
}

fn choose_block_format(role: TextureRole, has_alpha: bool) -> ImageFormat {
    match role {
        TextureRole::Normal => ImageFormat::BC5RgUnorm,

        TextureRole::Livery | TextureRole::Weapon => ImageFormat::BC7RgbaUnorm,

        _ if has_alpha => ImageFormat::BC7RgbaUnorm,

        _ => ImageFormat::BC1RgbaUnorm,
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_dds(
    image: RgbaImage,
    target_w: u32,
    target_h: u32,
    role: TextureRole,
    settings: &SqueezeSettings,
    path: &Path,
    gpu: Option<&GpuContext>,
) -> Result<Vec<u8>> {
    let has_alpha = codec::has_alpha(&image);
    let format = choose_block_format(role, has_alpha);

    let mip_levels = if settings.generate_mipmaps {
        u32::MAX
    } else {
        1
    };
    let surface = codec::resize_and_encode(
        image,
        target_w,
        target_h,
        format,
        settings.quality,
        mip_levels,
        gpu,
    )
    .ok_or_else(|| SqueezeError::EncodeFailed {
        path: path.to_path_buf(),
    })?;

    let dds = surface.to_dds().map_err(|source| SqueezeError::Encode {
        path: path.to_path_buf(),
        source,
    })?;
    write_dds(&dds, path)
}

fn write_dds(dds: &Dds, path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    dds.write(&mut bytes)
        .map_err(|source| SqueezeError::DdsWrite {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(bytes)
}

fn encode_source_format(image: RgbaImage, ext: &str, path: &Path) -> Result<Vec<u8>> {
    let format = match ext {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "tga" => image::ImageFormat::Tga,
        "bmp" => image::ImageFormat::Bmp,
        _ => unreachable!("unclaimed extension reached encode_source_format"),
    };

    let dynamic = if format == image::ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(image).into_rgb8())
    } else {
        DynamicImage::ImageRgba8(image)
    };

    let mut cursor = Cursor::new(Vec::new());
    dynamic
        .write_to(&mut cursor, format)
        .map_err(|source| SqueezeError::ReEncode {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tiny_png() -> Vec<u8> {
        let img = RgbaImage::from_pixel(16, 16, image::Rgba([90, 120, 40, 255]));
        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        cursor.into_inner()
    }

    fn squeeze(path: &str, bytes: &[u8], settings: &SqueezeSettings) -> SqueezeOutcome {
        let path = PathBuf::from(path);
        let job = TextureJob {
            path: &path,
            bytes,
            asset_hint: None,
            pool_saturated: false,
        };
        StandardImageSqueezer.squeeze(&job, settings, None).unwrap()
    }

    #[test]
    fn small_png_skips_without_force_convert() {
        let png = tiny_png();
        let outcome = squeeze("tiny_flat.png", &png, &SqueezeSettings::default());
        assert!(matches!(outcome, SqueezeOutcome::Skipped { .. }));
    }

    #[test]
    fn force_convert_transcodes_small_png_to_dds() {
        let png = tiny_png();
        let settings = SqueezeSettings {
            force_convert: true,
            ..Default::default()
        };
        match squeeze("tiny_flat.png", &png, &settings) {
            SqueezeOutcome::Optimized { bytes, extension } => {
                assert_eq!(extension, "dds");
                assert_eq!(&bytes[..4], b"DDS ");
            }
            other => panic!("expected Optimized, got {other:?}"),
        }
    }

    #[test]
    fn force_convert_wins_over_keep_source_format() {
        let png = tiny_png();
        let settings = SqueezeSettings {
            force_convert: true,
            keep_source_format: true,
            ..Default::default()
        };
        match squeeze("tiny_flat.png", &png, &settings) {
            SqueezeOutcome::Optimized { extension, .. } => assert_eq!(extension, "dds"),
            other => panic!("expected Optimized, got {other:?}"),
        }
    }

    #[test]
    fn force_convert_still_skips_dds_that_grew() {
        let png = tiny_png();
        let settings = SqueezeSettings {
            force_convert: true,
            ..Default::default()
        };
        let SqueezeOutcome::Optimized { bytes: dds, .. } =
            squeeze("tiny_flat.png", &png, &settings)
        else {
            panic!("conversion failed");
        };
        let outcome = squeeze("tiny_flat.dds", &dds, &settings);
        assert!(matches!(outcome, SqueezeOutcome::Skipped { .. }));
    }

    #[test]
    fn livery_textures_use_cpu_and_can_still_optimize() {
        let png = tiny_png();
        let settings = SqueezeSettings {
            force_convert: true,
            ..Default::default()
        };
        match squeeze("police_lvr_01.png", &png, &settings) {
            SqueezeOutcome::Optimized { extension, .. } => assert_eq!(extension, "dds"),
            other => panic!("expected optimized CPU fallback, got {other:?}"),
        }
    }
}
