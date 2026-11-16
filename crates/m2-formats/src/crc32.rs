//! CRC-32 as in zlib and PNG (IEEE 802.3): reflected polynomial `0xEDB88320`,
//! initial value and final XOR `0xFFFFFFFF`. Check value: `crc32(b"123456789")
//! == 0xCBF43926`.
//!
//! EterPack indexes store a CRC-32 of each entry's name and data
//! (`docs/specs/formats/eterpack.md`).

const POLYNOMIAL: u32 = 0xEDB8_8320;

/// CRC of every byte value, computed at compile time.
const TABLE: [u32; 256] = make_table();

// Indexing in a const fn: an index out of range would fail the build, not run.
#[allow(clippy::indexing_slicing)]
const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut byte = 0;
    while byte < 256 {
        let mut crc = byte as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ POLYNOMIAL
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[byte] = crc;
        byte += 1;
    }
    table
}

/// CRC-32 of `bytes`.
pub fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0u32, |crc, &byte| {
        // The index is a u8, always in range; `get` keeps the no-indexing rule.
        let entry = TABLE
            .get(usize::from(crc as u8 ^ byte))
            .copied()
            .unwrap_or_default();
        (crc >> 8) ^ entry
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn empty_and_known_strings() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }
}
