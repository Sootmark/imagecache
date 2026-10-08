//! Little-endian integers at an offset, `None` past the end.

pub(crate) fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(array(data, at)?))
}

pub(crate) fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(array(data, at)?))
}

pub(crate) fn u64_at(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(array(data, at)?))
}

fn array<const N: usize>(data: &[u8], at: usize) -> Option<[u8; N]> {
    data.get(at..at.checked_add(N)?)?.try_into().ok()
}
