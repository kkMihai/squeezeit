use image_dds::ImageFormat;

#[derive(Clone, Copy)]
pub(super) enum PixelLayout {
    Block {
        bytes_per_block: usize,
        dds: ImageFormat,
    },

    Linear {
        bytes_per_pixel: usize,
    },
}

pub(super) fn pixel_layout(format: u32) -> Option<PixelLayout> {
    use PixelLayout::*;
    Some(match format {
        0x3154_5844 => Block {
            bytes_per_block: 8,
            dds: ImageFormat::BC1RgbaUnorm,
        },
        0x3354_5844 => Block {
            bytes_per_block: 16,
            dds: ImageFormat::BC2RgbaUnorm,
        },
        0x3554_5844 => Block {
            bytes_per_block: 16,
            dds: ImageFormat::BC3RgbaUnorm,
        },
        0x3149_5441 => Block {
            bytes_per_block: 8,
            dds: ImageFormat::BC4RUnorm,
        },
        0x3249_5441 => Block {
            bytes_per_block: 16,
            dds: ImageFormat::BC5RgUnorm,
        },
        0x2037_4342 => Block {
            bytes_per_block: 16,
            dds: ImageFormat::BC7RgbaUnorm,
        },
        21 | 22 | 32 => Linear { bytes_per_pixel: 4 },
        23 | 25 | 26 => Linear { bytes_per_pixel: 2 },
        28 | 50 => Linear { bytes_per_pixel: 1 },
        113 => Linear { bytes_per_pixel: 8 },
        _ => return None,
    })
}

pub(super) fn mip_len(width: u32, height: u32, layout: PixelLayout) -> usize {
    match layout {
        PixelLayout::Block {
            bytes_per_block, ..
        } => (width.div_ceil(4) as usize) * (height.div_ceil(4) as usize) * bytes_per_block,
        PixelLayout::Linear { bytes_per_pixel } => {
            width as usize * height as usize * bytes_per_pixel
        }
    }
}

pub(super) fn chain_len(width: u32, height: u32, levels: u32, layout: PixelLayout) -> usize {
    (0..levels.max(1))
        .map(|i| mip_len((width >> i).max(1), (height >> i).max(1), layout))
        .sum()
}

pub(super) fn top_stride(width: u32, layout: PixelLayout) -> u16 {
    match layout {
        PixelLayout::Block {
            bytes_per_block, ..
        } => (width.div_ceil(4) as usize * bytes_per_block) as u16,
        PixelLayout::Linear { bytes_per_pixel } => (width as usize * bytes_per_pixel) as u16,
    }
}

pub(super) fn rage_mip_levels(w: u32, h: u32) -> u32 {
    let min_side = w.min(h).max(1);
    (min_side.ilog2()).saturating_sub(1).max(1)
}

pub(super) fn fourcc_of(f: ImageFormat) -> u32 {
    match f {
        ImageFormat::BC1RgbaUnorm => 0x3154_5844,
        ImageFormat::BC2RgbaUnorm => 0x3354_5844,
        ImageFormat::BC3RgbaUnorm => 0x3554_5844,
        ImageFormat::BC4RUnorm => 0x3149_5441,
        ImageFormat::BC5RgUnorm => 0x3249_5441,
        ImageFormat::BC7RgbaUnorm => 0x2037_4342,
        _ => unreachable!("only block formats are emitted into containers"),
    }
}

pub(super) fn pixel_layout_of(f: ImageFormat) -> PixelLayout {
    PixelLayout::Block {
        bytes_per_block: match f {
            ImageFormat::BC1RgbaUnorm | ImageFormat::BC4RUnorm => 8,
            _ => 16,
        },
        dds: f,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_math() {
        let bc1 = pixel_layout(0x3154_5844).unwrap();
        assert_eq!(mip_len(1024, 1024, bc1), 1024 / 4 * 1024 / 4 * 8);
        assert_eq!(mip_len(2, 2, bc1), 8);
        assert_eq!(top_stride(1024, bc1), 2048);

        assert_eq!(chain_len(4, 4, 3, bc1), 24);
    }

    #[test]
    fn rage_mip_convention() {
        assert_eq!(rage_mip_levels(512, 512), 8);
        assert_eq!(rage_mip_levels(256, 256), 7);
        assert_eq!(rage_mip_levels(256, 128), 6);
        assert_eq!(rage_mip_levels(512, 32), 4);
        assert_eq!(rage_mip_levels(8, 8), 2);
        assert_eq!(rage_mip_levels(4, 4), 1);
    }
}
