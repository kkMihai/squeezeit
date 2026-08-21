pub(super) const RSC7_MAGIC: [u8; 4] = *b"RSC7";
pub(super) const YTD_VERSION: u32 = 13;
pub(super) const SYSTEM_BASE: u64 = 0x5000_0000;
pub(super) const GRAPHICS_BASE: u64 = 0x6000_0000;
pub(super) const TEXTURE_RECORD_SIZE: usize = 0x90;
pub(super) const DATA_ALIGN: usize = 16;

pub(super) use crate::rsc7::pages::size_from_flags;

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
    fn pointer_resolution() {
        assert_eq!(system_offset(0x5000_0040, 0x100), Some(0x40));
        assert_eq!(graphics_offset(0x6000_0040, 0x100), Some(0x40));
        assert_eq!(system_offset(0x6000_0040, 0x100), None);
        assert_eq!(system_offset(0x5000_0200, 0x100), None);
    }
}
