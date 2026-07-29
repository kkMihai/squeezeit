pub(super) const RSC7_MAGIC: [u8; 4] = *b"RSC7";
pub(super) const YTD_VERSION: u32 = 13;
pub(super) const SYSTEM_BASE: u64 = 0x5000_0000;
pub(super) const GRAPHICS_BASE: u64 = 0x6000_0000;
pub(super) const TEXTURE_RECORD_SIZE: usize = 0x90;
pub(super) const DATA_ALIGN: usize = 16;

pub(super) fn size_from_flags(flags: u32) -> u64 {
    let s0 = ((flags >> 27) & 0x1) as u64;
    let s1 = (((flags >> 26) & 0x1) as u64) << 1;
    let s2 = (((flags >> 25) & 0x1) as u64) << 2;
    let s3 = (((flags >> 24) & 0x1) as u64) << 3;
    let s4 = (((flags >> 17) & 0x7F) as u64) << 4;
    let s5 = (((flags >> 11) & 0x3F) as u64) << 5;
    let s6 = (((flags >> 7) & 0xF) as u64) << 6;
    let s7 = (((flags >> 5) & 0x3) as u64) << 7;
    let s8 = (((flags >> 4) & 0x1) as u64) << 8;
    let base = 0x200u64 << (flags & 0xF);
    base * (s0 + s1 + s2 + s3 + s4 + s5 + s6 + s7 + s8)
}

pub(super) fn flags_from_size(size: u64, version_nibble: u32) -> Option<(u32, u64)> {
    for ss in 0u32..16 {
        let base = 0x200u64 << ss;
        let blocks = size.div_ceil(base);

        let mut rem = blocks;
        let s8 = (rem / 256).min(1);
        rem -= s8 * 256;
        let s7 = (rem / 128).min(3);
        rem -= s7 * 128;
        let s6 = (rem / 64).min(15);
        rem -= s6 * 64;
        let s5 = (rem / 32).min(63);
        rem -= s5 * 32;
        let s4 = (rem / 16).min(127);
        rem -= s4 * 16;
        if rem > 15 {
            continue;
        }

        let flags = (version_nibble << 28)
            | ((rem as u32 & 0x1) << 27)
            | (((rem as u32 >> 1) & 0x1) << 26)
            | (((rem as u32 >> 2) & 0x1) << 25)
            | (((rem as u32 >> 3) & 0x1) << 24)
            | ((s4 as u32) << 17)
            | ((s5 as u32) << 11)
            | ((s6 as u32) << 7)
            | ((s7 as u32) << 5)
            | ((s8 as u32) << 4)
            | ss;
        return Some((flags, size_from_flags(flags)));
    }
    None
}

pub(super) fn u16_at(buf: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(buf.get(off..off + 2)?.try_into().ok()?))
}

pub(super) fn u32_at(buf: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(off..off + 4)?.try_into().ok()?))
}

pub(super) fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(buf.get(off..off + 8)?.try_into().ok()?))
}

pub(super) fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

pub(super) fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub(super) fn put_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

pub(super) fn system_offset(ptr: u64, len: usize) -> Option<usize> {
    (ptr >> 28 == SYSTEM_BASE >> 28)
        .then_some((ptr & 0x0FFF_FFFF) as usize)
        .filter(|&o| o < len)
}

pub(super) fn graphics_offset(ptr: u64, len: usize) -> Option<usize> {
    (ptr >> 28 == GRAPHICS_BASE >> 28)
        .then_some((ptr & 0x0FFF_FFFF) as usize)
        .filter(|&o| o < len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_roundtrip() {
        for size in [
            0u64, 0x200, 0x1000, 0x12345, 0x8_0000, 0x100_0000, 0x1F0_0000,
        ] {
            let (flags, padded) = flags_from_size(size, 0xD).expect("encodable");
            assert!(padded >= size, "padded {padded:#x} < size {size:#x}");
            assert_eq!(size_from_flags(flags), padded);
            assert_eq!(flags >> 28, 0xD, "version nibble preserved");
        }
    }

    #[test]
    fn pointer_resolution() {
        assert_eq!(system_offset(0x5000_0040, 0x100), Some(0x40));
        assert_eq!(graphics_offset(0x6000_0040, 0x100), Some(0x40));
        assert_eq!(system_offset(0x6000_0040, 0x100), None);
        assert_eq!(system_offset(0x5000_0200, 0x100), None);
    }
}
