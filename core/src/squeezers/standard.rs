use crate::error::{Result, SqueezeError};
use crate::gpu::GpuContext;
use crate::gta5::AssetFamily;
use crate::policy::{MipRule, Policy};
use crate::settings::{FormatMode, ScriptRt, SqueezeSettings};
use crate::squeezers::{
    SqueezeOutcome, Squeezer, TextureBytes, TextureJob, claims_extension, codec,
};
use crate::texture::{TextureRole, classify_name, classify_pixels, target_dimensions};
use image::{DynamicImage, RgbaImage};
use image_dds::ImageFormat;
use image_dds::ddsfile::Dds;
use std::io::Cursor;
use std::path::Path;
pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tga", "bmp", "dds"];
const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];

pub struct StandardImageSqueezer;

impl Squeezer for StandardImageSqueezer {
    fn claims(&self, path: &Path) -> bool {
        claims_extension(path, EXTENSIONS)
    }

    fn squeeze(
        &self,
        job: &TextureJob<'_>,
        settings: &SqueezeSettings,
        gpu: Option<&GpuContext>,
    ) -> Result<SqueezeOutcome> {
        let name_role = classify_name(job.path);
        let source_ext = job.extension().unwrap_or_default();

        if name_role == Some(TextureRole::ScriptRenderTarget) {
            let resident = read_dds(job, &source_ext)
                .ok()
                .flatten()
                .map_or(0, |dds| texture_bytes(&dds));
            return Ok(SqueezeOutcome::locked(
                "script_rt render target, dynamic UI surface must stay native",
            )
            .with_textures(TextureBytes::unchanged(resident)));
        }

        let stem = job
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if settings.exclusions.excludes(stem) {
            return Ok(SqueezeOutcome::locked("named in the exclusion list"));
        }
        if settings.script_rt == ScriptRt::Only {
            return Ok(SqueezeOutcome::skipped(
                "only script_rt surfaces are being processed",
            ));
        }

        let (image, source_textures) = decode(job, &source_ext)?;
        warn_if_not_power_of_two(job.path, &source_ext, image.width(), image.height());

        let role = name_role
            .or_else(|| classify_pixels(image.as_raw(), image.width(), image.height()))
            .unwrap_or(TextureRole::Diffuse);

        let policy = Policy::resolve(settings, job.asset_hint.unwrap_or(AssetFamily::Generic));
        let policy = if role == TextureRole::Hair {
            policy.tighten_for_hair()
        } else {
            policy
        };

        let gpu = gpu
            .filter(|_| policy.gpu && !matches!(role, TextureRole::Livery | TextureRole::Weapon));

        let (target_w, target_h) = target_dimensions(image.width(), image.height(), role, &policy);

        let keep_source = settings.format == FormatMode::Keep && source_ext != "dds";
        let (bytes, extension, out_textures) = if keep_source {
            let image = codec::resize(image, target_w, target_h);
            (
                encode_source_format(image, &source_ext, job.path)?,
                same_extension(&source_ext),
                0,
            )
        } else {
            let (bytes, out_textures) = encode_dds(
                image, target_w, target_h, role, settings, &policy, job.path, gpu,
            )?;
            (bytes, "dds", out_textures)
        };

        let textures = if source_textures == 0 {
            TextureBytes::ZERO
        } else {
            TextureBytes {
                before: source_textures,
                after: out_textures,
            }
        };

        let converts_format = !extension.eq_ignore_ascii_case(&source_ext);
        let forced = settings.format == FormatMode::ForceDds && converts_format;
        if bytes.len() >= job.bytes.len() && !forced {
            return Ok(SqueezeOutcome::skipped(format!(
                "already optimal ({} bytes in, {} bytes out)",
                job.bytes.len(),
                bytes.len()
            ))
            .with_textures(textures.discarded()));
        }

        Ok(SqueezeOutcome::optimized(bytes, extension).with_textures(textures))
    }
}

fn same_extension(ext: &str) -> &'static str {
    EXTENSIONS
        .iter()
        .copied()
        .find(|c| c.eq_ignore_ascii_case(ext))
        .expect("we only get here for extensions we claimed")
}

fn read_dds(job: &TextureJob<'_>, source_ext: &str) -> Result<Option<Dds>> {
    if source_ext != "dds" {
        return Ok(None);
    }
    Dds::read(&mut Cursor::new(job.bytes))
        .map(Some)
        .map_err(|source| SqueezeError::DdsParse {
            path: job.path.to_path_buf(),
            source,
        })
}

fn texture_bytes(dds: &Dds) -> u64 {
    dds.get_data(0).map_or(0, |data| data.len() as u64)
}

fn decode(job: &TextureJob<'_>, source_ext: &str) -> Result<(RgbaImage, u64)> {
    if let Some(dds) = read_dds(job, source_ext)? {
        let resident = texture_bytes(&dds);
        let image =
            image_dds::image_from_dds(&dds, 0).map_err(|source| SqueezeError::DdsDecode {
                path: job.path.to_path_buf(),
                source,
            })?;
        return Ok((image, resident));
    }
    if let Some(image) = decode_png_simd(job.bytes) {
        return Ok((image, 0));
    }
    Ok((
        image::load_from_memory(job.bytes)
            .map_err(|source| SqueezeError::Decode {
                path: job.path.to_path_buf(),
                source,
            })?
            .into_rgba8(),
        0,
    ))
}

fn is_odd_sided(source_ext: &str, width: u32, height: u32) -> bool {
    source_ext == "dds" && !(width.is_power_of_two() && height.is_power_of_two())
}

fn warn_if_not_power_of_two(path: &Path, source_ext: &str, width: u32, height: u32) {
    if is_odd_sided(source_ext, width, height) {
        tracing::warn!(
            path = %path.display(),
            reason = %format!(
                "{width}x{height} is not a power of two, crop the source rather than \
                 leaving it to be resampled"
            ),
            "odd size"
        );
    }
}

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
            tracing::debug!(%error, "zune-png decode failed, using the image crate");
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
        TextureRole::Hair | TextureRole::Livery | TextureRole::Weapon => ImageFormat::BC7RgbaUnorm,
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
    policy: &Policy,
    path: &Path,
    gpu: Option<&GpuContext>,
) -> Result<(Vec<u8>, u64)> {
    let format = choose_block_format(role, codec::has_alpha(&image));

    let mip_levels = if policy.mips == MipRule::GenerateFull {
        u32::MAX
    } else {
        1
    };
    let surface = codec::resize_and_encode(
        image,
        target_w,
        target_h,
        format,
        policy.quality(settings),
        mip_levels,
        gpu,
    )
    .ok_or_else(|| SqueezeError::EncodeFailed {
        path: path.to_path_buf(),
    })?;
    let resident = surface.data.len() as u64;

    let dds = surface.to_dds().map_err(|source| SqueezeError::Encode {
        path: path.to_path_buf(),
        source,
    })?;

    let mut bytes = Vec::new();
    dds.write(&mut bytes)
        .map_err(|source| SqueezeError::DdsWrite {
            path: path.to_path_buf(),
            source,
        })?;
    Ok((bytes, resident))
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
    use crate::settings::SizeLimit;
    use std::path::PathBuf;

    fn tiny_png() -> Vec<u8> {
        let img = RgbaImage::from_pixel(16, 16, image::Rgba([90, 120, 40, 255]));
        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        cursor.into_inner()
    }

    fn noisy_png(side: u32) -> Vec<u8> {
        let mut seed = 0x1234_5678u32;
        let img = RgbaImage::from_fn(side, side, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let [r, g, b, _] = seed.to_le_bytes();
            image::Rgba([r, g, b, 255])
        });
        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        cursor.into_inner()
    }

    fn squeeze(path: &str, bytes: &[u8], settings: &SqueezeSettings) -> SqueezeOutcome {
        squeeze_as(path, bytes, settings, None)
    }

    fn squeeze_as(
        path: &str,
        bytes: &[u8],
        settings: &SqueezeSettings,
        asset_hint: Option<AssetFamily>,
    ) -> SqueezeOutcome {
        let path = PathBuf::from(path);
        let job = TextureJob {
            path: &path,
            bytes,
            asset_hint,
            pool_saturated: false,
        };
        StandardImageSqueezer.squeeze(&job, settings, None).unwrap()
    }

    fn dds_of(outcome: SqueezeOutcome) -> Dds {
        match outcome {
            SqueezeOutcome::Optimized {
                bytes, extension, ..
            } => {
                assert_eq!(extension, "dds");
                Dds::read(&mut Cursor::new(bytes)).expect("output parses as DDS")
            }
            other => panic!("expected Optimized, got {other:?}"),
        }
    }

    fn dds_bytes(outcome: SqueezeOutcome) -> Vec<u8> {
        match outcome {
            SqueezeOutcome::Optimized { bytes, .. } => bytes,
            other => panic!("expected Optimized, got {other:?}"),
        }
    }

    #[test]
    fn a_small_png_is_left_alone_under_auto() {
        let outcome = squeeze("tiny_flat.png", &tiny_png(), &SqueezeSettings::default());
        assert!(matches!(outcome, SqueezeOutcome::Skipped { .. }));
    }

    #[test]
    fn always_dds_transcodes_a_small_png() {
        let settings = SqueezeSettings {
            format: FormatMode::ForceDds,
            ..Default::default()
        };
        match squeeze("tiny_flat.png", &tiny_png(), &settings) {
            SqueezeOutcome::Optimized {
                bytes, extension, ..
            } => {
                assert_eq!(extension, "dds");
                assert_eq!(&bytes[..4], b"DDS ");
            }
            other => panic!("expected Optimized, got {other:?}"),
        }
    }

    #[test]
    fn converting_a_png_reports_no_texture_memory() {
        let settings = SqueezeSettings {
            format: FormatMode::ForceDds,
            ..Default::default()
        };
        let outcome = squeeze("prop_crate_01.png", &noisy_png(256), &settings);
        assert_eq!(outcome.textures(), TextureBytes::ZERO);
    }

    #[test]
    fn odd_sides_are_called_out_on_dds_only() {
        assert!(is_odd_sided("dds", 1500, 900));
        assert!(is_odd_sided("dds", 1024, 900));
        assert!(is_odd_sided("dds", 384, 384));

        assert!(!is_odd_sided("dds", 1024, 1024));
        assert!(!is_odd_sided("dds", 2048, 64));

        for ext in ["png", "jpg", "tga", "bmp", ""] {
            assert!(!is_odd_sided(ext, 1500, 900), "{ext} was called out");
        }
    }

    #[test]
    fn shrinking_a_dds_reports_the_texture_memory_it_freed() {
        let source = dds_bytes(squeeze(
            "prop_crate_01.png",
            &noisy_png(512),
            &SqueezeSettings {
                format: FormatMode::ForceDds,
                ..Default::default()
            },
        ));

        let outcome = squeeze(
            "prop_crate_01.dds",
            &source,
            &SqueezeSettings {
                size_limit: SizeLimit::Max(128),
                ..Default::default()
            },
        );
        let textures = outcome.textures();
        assert!(textures.before > 0, "the source DDS was never measured");
        assert!(
            textures.after < textures.before,
            "512 -> 128 freed nothing: {textures:?}"
        );
    }

    #[test]
    fn a_skipped_dds_reports_the_same_bytes_on_both_sides() {
        let source = dds_bytes(squeeze(
            "prop_crate_01.png",
            &tiny_png(),
            &SqueezeSettings {
                format: FormatMode::ForceDds,
                ..Default::default()
            },
        ));

        let outcome = squeeze("prop_crate_01.dds", &source, &SqueezeSettings::default());
        assert!(matches!(outcome, SqueezeOutcome::Skipped { .. }));
        let textures = outcome.textures();
        assert!(textures.before > 0);
        assert_eq!(textures.before, textures.after);
    }

    #[test]
    fn always_dds_still_skips_a_dds_that_grew() {
        let settings = SqueezeSettings {
            format: FormatMode::ForceDds,
            ..Default::default()
        };
        let SqueezeOutcome::Optimized { bytes: dds, .. } =
            squeeze("tiny_flat.png", &tiny_png(), &settings)
        else {
            panic!("conversion failed");
        };
        let outcome = squeeze("tiny_flat.dds", &dds, &settings);
        assert!(matches!(outcome, SqueezeOutcome::Skipped { .. }));
    }

    #[test]
    fn keeping_the_source_format_stays_png() {
        let settings = SqueezeSettings {
            format: FormatMode::Keep,
            size_limit: SizeLimit::Max(128),
            ..Default::default()
        };
        match squeeze("wall_big.png", &noisy_png(512), &settings) {
            SqueezeOutcome::Optimized { extension, .. } => assert_eq!(extension, "png"),
            other => panic!("expected Optimized, got {other:?}"),
        }
    }

    #[test]
    fn hair_keeps_bc7_and_no_mips_while_a_prop_gets_bc1_and_a_chain() {
        let png = noisy_png(256);
        let settings = SqueezeSettings::default();

        let hair = dds_of(squeeze_as(
            "mp_f_freemode_01^hair_006_u.png",
            &png,
            &settings,
            Some(AssetFamily::PedHair),
        ));
        assert_eq!(hair.get_data(0).unwrap().len(), 256 * 256);
        assert_eq!(hair.get_num_mipmap_levels(), 1);

        let prop = dds_of(squeeze("prop_crate_01.png", &png, &settings));
        assert!(prop.get_data(0).unwrap().len() < 256 * 256);
        assert!(
            prop.get_num_mipmap_levels() > 1,
            "props still get a full chain"
        );
    }

    #[test]
    fn a_hair_name_alone_is_enough_to_lock_the_format() {
        let hair = dds_of(squeeze(
            "hair_diff_000_a_uni.png",
            &noisy_png(256),
            &Default::default(),
        ));
        assert_eq!(hair.get_data(0).unwrap().len(), 256 * 256);
        assert_eq!(hair.get_num_mipmap_levels(), 1);
    }

    #[test]
    fn livery_textures_use_cpu_and_can_still_optimize() {
        let settings = SqueezeSettings {
            format: FormatMode::ForceDds,
            ..Default::default()
        };
        match squeeze("police_lvr_01.png", &tiny_png(), &settings) {
            SqueezeOutcome::Optimized { extension, .. } => assert_eq!(extension, "dds"),
            other => panic!("expected optimized CPU fallback, got {other:?}"),
        }
    }
}
