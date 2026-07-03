//! Windows-1251 (CP1251) decoding — INPA `.ipo` strings are CP1251, `0x0a`-terminated.
//!
//! Dependency-free: bytes `<0x80` are ASCII, `0xC0..=0xFF` map linearly onto the
//! Cyrillic block `U+0410..U+044F`, and the `0x80..=0xBF` block uses the table below.

/// CP1251 code points for bytes `0x80..=0xBF` (index = `byte - 0x80`).
/// `0x98` is undefined in CP1251 → replacement char.
const CP1251_HIGH: [char; 64] = [
    '\u{0402}', '\u{0403}', '\u{201A}', '\u{0453}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{20AC}', '\u{2030}', '\u{0409}', '\u{2039}', '\u{040A}', '\u{040C}', '\u{040B}', '\u{040F}',
    '\u{0452}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{FFFD}', '\u{2122}', '\u{0459}', '\u{203A}', '\u{045A}', '\u{045C}', '\u{045B}', '\u{045F}',
    '\u{00A0}', '\u{040E}', '\u{045E}', '\u{0408}', '\u{00A4}', '\u{0490}', '\u{00A6}', '\u{00A7}',
    '\u{0401}', '\u{00A9}', '\u{0404}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{0407}',
    '\u{00B0}', '\u{00B1}', '\u{0406}', '\u{0456}', '\u{0491}', '\u{00B5}', '\u{00B6}', '\u{00B7}',
    '\u{0451}', '\u{2116}', '\u{0454}', '\u{00BB}', '\u{0458}', '\u{0405}', '\u{0455}', '\u{0457}',
];

#[inline]
fn cp1251_char(b: u8) -> char {
    if b < 0x80 {
        b as char
    } else if b >= 0xC0 {
        // 0xC0..=0xFF → U+0410..=U+044F (А..я)
        char::from_u32(0x0410 + (b as u32 - 0xC0)).unwrap()
    } else {
        CP1251_HIGH[(b - 0x80) as usize]
    }
}

/// Decode a CP1251 byte slice into a `String`.
pub fn decode_cp1251(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| cp1251_char(b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_passthrough() {
        assert_eq!(decode_cp1251(b"STAT_AC_EIN"), "STAT_AC_EIN");
    }

    #[test]
    fn cyrillic_block() {
        // 0xCF 0xEE 0xF2 0xEE 0xEA = "Поток" (П о т о к)
        assert_eq!(decode_cp1251(&[0xCF, 0xEE, 0xF2, 0xEE, 0xEA]), "Поток");
    }

    #[test]
    fn special_block() {
        assert_eq!(decode_cp1251(&[0xB9]), "№"); // U+2116
        assert_eq!(decode_cp1251(&[0xB0]), "°"); // U+00B0
    }
}
