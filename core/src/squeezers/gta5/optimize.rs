use super::dictionary::TexRecord;
use super::format::{
    PixelLayout, chain_len, fourcc_of, mip_len, physical_mip_levels, pixel_layout_of,
    rage_mip_levels, top_stride,
};
use crate::gpu::GpuContext;
use crate::policy::{Container, FormatRule, MipRule, Policy};
use crate::settings::SqueezeSettings;
use crate::squeezers::codec;
use crate::texture::{TextureRole, cap_dimensions, classify_pixels, target_dimensions};
use image::RgbaImage;
use image_dds::ImageFormat;

#[derive(Clone, Copy)]
pub(super) struct TexPatch {
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) stride: u16,
    pub(super) format: u32,
    pub(super) levels: u8,
}

pub(super) struct Request<'a> {
    pub(super) rec: &'a TexRecord,
    pub(super) source: &'a [u8],
    pub(super) layout: PixelLayout,
    pub(super) name_role: Option<TextureRole>,
    pub(super) pair_cap: Option<u32>,
    pub(super) max_len: Option<usize>,
    pub(super) container: Container,
    pub(super) gpu: Option<&'a GpuContext>,
    pub(super) policy: &'a Policy,
}

pub(super) fn optimize_texture(
    req: Request<'_>,
    settings: &SqueezeSettings,
) -> Option<(Vec<u8>, TexPatch)> {
    let Request {
        rec,
        source,
        layout,
        name_role,
        pair_cap,
        max_len,
        container,
        gpu,
        policy,
    } = req;

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
        None if policy.overdrive => {
            let image = decode_top()?;
            let role = classify_pixels(image.as_raw(), image.width(), image.height())
                .unwrap_or(TextureRole::Diffuse);
            decoded = Some(image);
            role
        }
        None => TextureRole::Diffuse,
    };

    let policy = if role == TextureRole::Hair {
        policy.tighten_for_hair()
    } else {
        *policy
    };

    let gpu =
        gpu.filter(|_| policy.gpu && !matches!(role, TextureRole::Livery | TextureRole::Weapon));
    let quality = policy.quality(settings);

    let (mut tw, mut th) = target_dimensions(rec.width, rec.height, role, &policy);
    if policy.overdrive
        && matches!(role, TextureRole::Normal | TextureRole::Specular)
        && let Some(cap) = pair_cap
    {
        (tw, th) = cap_dimensions(tw, th, cap);
    }

    let out_dds = choose_format(
        dds,
        role,
        &policy,
        rec,
        source,
        layout,
        container,
        &mut decoded,
    )?;

    let mip_rule = if policy.mip_exception {
        let opaque = is_opaque(rec, source, layout, dds, &mut decoded, decode_top);
        if policy.mip_exception_applies(opaque, out_dds == dds, rec.levels, tw, th, container) {
            MipRule::GenerateFull
        } else {
            policy.mips
        }
    } else {
        policy.mips
    };

    let full_chain = rage_mip_levels(tw, th);
    let gen_mips = mip_rule == MipRule::GenerateFull && container.may_grow();
    let mip_levels = match mip_rule {
        MipRule::Preserve => rec.levels.clamp(1, physical_mip_levels(tw, th)),
        _ if gen_mips => full_chain,
        _ => rec.levels.clamp(1, full_chain),
    };
    let needs_mips = gen_mips && rec.levels <= 1 && tw.min(th) >= 8;
    let mips_trimmed = mip_levels < rec.levels;
    let same_surface = (tw, th) == (rec.width, rec.height) && out_dds == dds && !needs_mips;

    if same_surface && !mips_trimmed {
        return None;
    }

    if same_surface
        && let Some(data) = source.get(..chain_len(rec.width, rec.height, mip_levels, layout))
    {
        return Some((data.to_vec(), patch(tw, th, out_dds, mip_levels as u8)));
    }

    let encoded = match (decoded, gpu) {
        (Some(image), _) => {
            codec::resize_and_encode(image, tw, th, out_dds, quality, mip_levels, gpu)?
        }
        (None, Some(ctx))
            if ctx.supports_bc_source(dds)
                && rec.width.is_multiple_of(4)
                && rec.height.is_multiple_of(4) =>
        {
            let top = source.get(..mip_len(rec.width, rec.height, layout))?;
            codec::resize_and_encode_from_blocks(
                top, dds, rec.width, rec.height, tw, th, out_dds, quality, mip_levels, ctx,
            )?
        }
        (None, gpu) => {
            codec::resize_and_encode(decode_top()?, tw, th, out_dds, quality, mip_levels, gpu)?
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

    Some((encoded.data, patch(tw, th, out_dds, encoded.mipmaps as u8)))
}

fn patch(w: u32, h: u32, format: ImageFormat, levels: u8) -> TexPatch {
    TexPatch {
        width: w as u16,
        height: h as u16,
        stride: top_stride(w, pixel_layout_of(format)),
        format: fourcc_of(format),
        levels,
    }
}

#[allow(clippy::too_many_arguments)]
fn choose_format(
    dds: ImageFormat,
    role: TextureRole,
    policy: &Policy,
    rec: &TexRecord,
    source: &[u8],
    layout: PixelLayout,
    container: Container,
    decoded: &mut Option<RgbaImage>,
) -> Option<ImageFormat> {
    let promoted = match policy.format {
        FormatRule::Locked => dds,
        FormatRule::Conservative => match dds {
            ImageFormat::BC2RgbaUnorm | ImageFormat::BC3RgbaUnorm if container.may_grow() => {
                ImageFormat::BC7RgbaUnorm
            }
            ImageFormat::BC2RgbaUnorm => ImageFormat::BC3RgbaUnorm,
            other => other,
        },
        FormatRule::Aggressive => match dds {
            ImageFormat::BC2RgbaUnorm => ImageFormat::BC3RgbaUnorm,
            other => other,
        },
    };

    let bc1_allowed = policy.format == FormatRule::Aggressive
        && !matches!(
            role,
            TextureRole::Normal | TextureRole::Livery | TextureRole::Weapon
        );
    if !bc1_allowed {
        return Some(promoted);
    }

    let downgrade = match dds {
        ImageFormat::BC2RgbaUnorm | ImageFormat::BC3RgbaUnorm => source
            .get(..mip_len(rec.width, rec.height, layout))
            .and_then(|top| codec::bc_alpha_summary(top, dds, rec.width, rec.height))
            .is_some_and(|summary| {
                matches!(summary, codec::BcAlpha::Opaque | codec::BcAlpha::Binary)
            }),
        ImageFormat::BC7RgbaUnorm => {
            if decoded.is_none() {
                *decoded = Some(decode_or(source, rec, layout, dds)?);
            }
            !codec::has_alpha(decoded.as_ref()?)
        }
        _ => false,
    };

    Some(if downgrade {
        ImageFormat::BC1RgbaUnorm
    } else {
        promoted
    })
}

fn decode_or(
    source: &[u8],
    rec: &TexRecord,
    layout: PixelLayout,
    dds: ImageFormat,
) -> Option<RgbaImage> {
    let top = source.get(..mip_len(rec.width, rec.height, layout))?;
    codec::decode_block(top, dds, rec.width, rec.height)
}

fn is_opaque(
    rec: &TexRecord,
    source: &[u8],
    layout: PixelLayout,
    dds: ImageFormat,
    decoded: &mut Option<RgbaImage>,
    decode_top: impl Fn() -> Option<RgbaImage>,
) -> bool {
    if let Some(top) = source.get(..mip_len(rec.width, rec.height, layout))
        && let Some(summary) = codec::bc_alpha_summary(top, dds, rec.width, rec.height)
    {
        return summary == codec::BcAlpha::Opaque;
    }
    if decoded.is_none() {
        *decoded = decode_top();
    }
    decoded
        .as_ref()
        .is_some_and(|image| !codec::has_alpha(image))
}
