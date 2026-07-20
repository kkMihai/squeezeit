use image_dds::ImageFormat;

use super::dictionary::TexRecord;
use super::format::{
    PixelLayout, chain_len, fourcc_of, mip_len, pixel_layout_of, rage_mip_levels, top_stride,
};
use super::shaders::AssetFamily;
use image::RgbaImage;

use crate::gpu::GpuContext;
use crate::settings::SqueezeSettings;
use crate::squeezers::codec;
use crate::texture::{TextureRole, cap_dimensions, classify_pixels, target_dimensions_capped};

fn guide_cap(role: TextureRole) -> u32 {
    match role {
        TextureRole::Normal | TextureRole::Specular => 1024,
        _ => u32::MAX,
    }
}

pub(super) struct TexPatch {
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) stride: u16,
    pub(super) format: u32,
    pub(super) levels: u8,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn optimize_texture(
    rec: &TexRecord,
    source: &[u8],
    layout: PixelLayout,
    name_role: Option<TextureRole>,
    pair_cap: Option<u32>,
    settings: &SqueezeSettings,
    max_len: Option<usize>,
    gpu: Option<&GpuContext>,
    family: AssetFamily,
) -> Option<(Vec<u8>, TexPatch)> {
    let PixelLayout::Block { dds, .. } = layout else {
        return None;
    };
    let decode_top = || {
        let top = source.get(..mip_len(rec.width, rec.height, layout))?;
        codec::decode_block(top, dds, rec.width, rec.height)
    };

    let mut decoded: Option<RgbaImage> = None;
    let role = match name_role {
        Some(role) => role,
        None if settings.overdrive => {
            let image = decode_top()?;
            let role = classify_pixels(image.as_raw(), image.width(), image.height())
                .unwrap_or(TextureRole::Diffuse);
            decoded = Some(image);
            role
        }
        None => TextureRole::Diffuse,
    };
    let gpu = if matches!(role, TextureRole::Livery | TextureRole::Weapon) {
        None
    } else {
        gpu
    };

    let (mut tw, mut th) = target_dimensions_capped(
        rec.width,
        rec.height,
        role,
        settings,
        guide_cap(role),
        family == AssetFamily::PedCloth,
    );

    if family != AssetFamily::PedCloth
        && matches!(role, TextureRole::Normal | TextureRole::Specular)
        && let Some(cap) = pair_cap
    {
        (tw, th) = cap_dimensions(tw, th, cap);
    }

    let mut out_dds = if family == AssetFamily::PedCloth
        && matches!(dds, ImageFormat::BC2RgbaUnorm | ImageFormat::BC3RgbaUnorm)
    {
        ImageFormat::BC7RgbaUnorm
    } else if dds == ImageFormat::BC2RgbaUnorm && !settings.keep_source_format {
        ImageFormat::BC3RgbaUnorm
    } else {
        dds
    };

    if !settings.keep_source_format
        && role != TextureRole::Normal
        && family != AssetFamily::PedCloth
        && !matches!(role, TextureRole::Livery | TextureRole::Weapon)
    {
        match dds {
            ImageFormat::BC2RgbaUnorm | ImageFormat::BC3RgbaUnorm => {
                if let Some(top) = source.get(..mip_len(rec.width, rec.height, layout))
                    && matches!(
                        codec::bc_alpha_summary(top, dds, rec.width, rec.height),
                        Some(codec::BcAlpha::Opaque | codec::BcAlpha::Binary)
                    )
                {
                    out_dds = ImageFormat::BC1RgbaUnorm;
                }
            }
            ImageFormat::BC7RgbaUnorm => {
                if decoded.is_none() {
                    decoded = Some(decode_top()?);
                }
                if !codec::has_alpha(decoded.as_ref()?) {
                    out_dds = ImageFormat::BC1RgbaUnorm;
                }
            }
            _ => {}
        }
    }

    let gen_mips = settings.generate_mipmaps && family.allows_mip_generation();
    let full_chain = rage_mip_levels(tw, th);
    let mip_levels = if gen_mips {
        full_chain
    } else {
        // Always cap at rage_mip_levels — strip useless tail mips
        // (2×2, 1×1, etc.) that RAGE never samples.
        rec.levels.clamp(1, full_chain)
    };
    let needs_mips = gen_mips && rec.levels <= 1 && tw.min(th) >= 8;
    let mips_trimmed = mip_levels < rec.levels;

    if (tw, th) == (rec.width, rec.height) && out_dds == dds && !needs_mips && !mips_trimmed {
        return None;
    }

    // Fast path: if only trimming mip levels (no resize, no format change),
    // just truncate the existing byte slice — skip the expensive decode → encode.
    if (tw, th) == (rec.width, rec.height) && out_dds == dds && !needs_mips && mips_trimmed {
        let trimmed_len = chain_len(rec.width, rec.height, mip_levels, layout);
        if let Some(data) = source.get(..trimmed_len) {
            let patch = TexPatch {
                width: tw as u16,
                height: th as u16,
                stride: top_stride(tw, pixel_layout_of(out_dds)),
                format: fourcc_of(out_dds),
                levels: mip_levels as u8,
            };
            return Some((data.to_vec(), patch));
        }
    }

    let encoded = match (decoded, gpu) {
        (Some(image), _) => {
            codec::resize_and_encode(image, tw, th, out_dds, settings.quality, mip_levels, gpu)?
        }
        (None, Some(ctx))
            if ctx.supports_bc_source(dds)
                && rec.width.is_multiple_of(4)
                && rec.height.is_multiple_of(4) =>
        {
            let top = source.get(..mip_len(rec.width, rec.height, layout))?;
            codec::resize_and_encode_from_blocks(
                top,
                dds,
                rec.width,
                rec.height,
                tw,
                th,
                out_dds,
                settings.quality,
                mip_levels,
                ctx,
            )?
        }
        (None, gpu) => {
            let image = decode_top()?;
            codec::resize_and_encode(image, tw, th, out_dds, settings.quality, mip_levels, gpu)?
        }
    };

    if max_len.is_some_and(|cap| encoded.data.len() > cap) {
        return None;
    }
    let worth = (tw, th) != (rec.width, rec.height)
        || out_dds != dds
        || needs_mips
        || encoded.data.len() < source.len();
    if !worth {
        return None;
    }

    let patch = TexPatch {
        width: tw as u16,
        height: th as u16,
        stride: top_stride(tw, pixel_layout_of(out_dds)),
        format: fourcc_of(out_dds),
        levels: encoded.mipmaps as u8,
    };
    Some((encoded.data, patch))
}
