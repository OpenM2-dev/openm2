//! Hex digits for hashes, keys and signatures.

use std::fmt::Write as _;

/// Bytes as lowercase hex digits.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// `N` bytes from exactly `2 * N` hex digits; with `lowercase`, capitals are
/// refused.
pub(crate) fn decode<const N: usize>(text: &str, lowercase: bool) -> Option<[u8; N]> {
    let digits = text.as_bytes();
    if digits.len() != N * 2 {
        return None;
    }
    let value = |digit: u8| match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' if !lowercase => Some(digit - b'A' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    let (pairs, _) = digits.as_chunks::<2>();
    for (byte, [high, low]) in out.iter_mut().zip(pairs) {
        *byte = value(*high)? << 4 | value(*low)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_both_ways() {
        assert_eq!(encode(&[0x00, 0xab, 0x7f]), "00ab7f");
        assert_eq!(decode::<3>("00ab7f", true), Some([0x00, 0xab, 0x7f]));
        assert_eq!(decode::<3>("00AB7F", false), Some([0x00, 0xab, 0x7f]));
        assert_eq!(decode::<3>("00AB7F", true), None);
        assert_eq!(decode::<3>("00ab7", true), None);
        assert_eq!(decode::<3>("00ab7g", true), None);
        assert_eq!(decode::<2>("00ab7f", true), None);
    }
}
