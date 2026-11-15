use std::borrow::Cow;

use crate::Error;

/// Converts a Java Modified UTF-8 (CESU-8) byte slice into a Rust UTF-8 string.
///
/// Fast-paths standard UTF-8 without 4-byte sequences (which Java rejects),
/// avoiding allocation and using the standard library's SIMD validation.
///
/// # Errors
/// Returns [`Error::Cesu8DecodingError`] if the bytes are not valid Java CESU-8.
pub fn from_java_cesu8(bytes: &[u8]) -> Result<Cow<'_, str>, Error> {
    if !bytes.iter().any(|&b| b >= 0xF0)
        && let Ok(s) = std::str::from_utf8(bytes)
    {
        return Ok(Cow::Borrowed(s));
    }
    decode_java_cesu8(bytes).map(Cow::Owned)
}

/// Converts a Rust UTF-8 string into Java Modified UTF-8 (CESU-8) bytes.
///
/// Fast-paths strings without null bytes or 4-byte UTF-8 sequences (`b >= 0xF0`),
/// returning a borrowed slice of the existing string without allocating.
#[must_use]
pub fn to_java_cesu8(s: &str) -> Cow<'_, [u8]> {
    if s.as_bytes().iter().any(|&b| b == 0 || b >= 0xF0) {
        Cow::Owned(encode_java_cesu8(s))
    } else {
        Cow::Borrowed(s.as_bytes())
    }
}

/// Java `DataInputStream.readUTF` (what `StringTag` reads with): fails on
/// broken sequences (a stray continuation byte, a truncated sequence, a
/// byte from `0xF0`) like vanilla, but takes raw `0`, overlong forms and
/// surrogates as Java does. Lone surrogates have no Rust `char` and become
/// U+FFFD.
fn decode_java_cesu8(bytes: &[u8]) -> Result<String, Error> {
    let continuation = |i: usize| match bytes.get(i) {
        Some(&b) if b & 0xC0 == 0x80 => Ok(u16::from(b & 0x3F)),
        _ => Err(Error::Cesu8DecodingError),
    };
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b0 = u16::from(bytes[i]);
        match b0 >> 4 {
            0..=7 => {
                units.push(b0);
                i += 1;
            }
            12 | 13 => {
                units.push(((b0 & 0x1F) << 6) | continuation(i + 1)?);
                i += 2;
            }
            14 => {
                units
                    .push(((b0 & 0x0F) << 12) | (continuation(i + 1)? << 6) | continuation(i + 2)?);
                i += 3;
            }
            _ => return Err(Error::Cesu8DecodingError),
        }
    }
    Ok(char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect())
}

fn encode_java_cesu8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 4);
    for c in s.chars() {
        let cp = c as u32;
        if cp == 0 {
            out.push(0xC0);
            out.push(0x80);
        } else if cp < 0x80 {
            out.push(cp as u8);
        } else if cp < 0x800 {
            out.push((0xC0 | (cp >> 6)) as u8);
            out.push((0x80 | (cp & 0x3F)) as u8);
        } else if cp < 0x10000 {
            out.push((0xE0 | (cp >> 12)) as u8);
            out.push((0x80 | ((cp >> 6) & 0x3F)) as u8);
            out.push((0x80 | (cp & 0x3F)) as u8);
        } else {
            let offset = cp - 0x10000;
            let high = 0xD800 + (offset >> 10);
            let low = 0xDC00 + (offset & 0x3FF);
            out.push((0xE0 | (high >> 12)) as u8);
            out.push((0x80 | ((high >> 6) & 0x3F)) as u8);
            out.push((0x80 | (high & 0x3F)) as u8);
            out.push((0xE0 | (low >> 12)) as u8);
            out.push((0x80 | ((low >> 6) & 0x3F)) as u8);
            out.push((0x80 | (low & 0x3F)) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cesu8_fast_path_roundtrip() {
        // Standard ASCII
        let ascii = "minecraft:stone";
        let encoded = to_java_cesu8(ascii);
        assert!(matches!(encoded, Cow::Borrowed(_)));
        let decoded = from_java_cesu8(&encoded).unwrap();
        assert_eq!(decoded, ascii);

        // 2-byte UTF-8
        let utf8_2 = "café";
        let encoded = to_java_cesu8(utf8_2);
        assert!(matches!(encoded, Cow::Borrowed(_)));
        let decoded = from_java_cesu8(&encoded).unwrap();
        assert_eq!(decoded, utf8_2);

        // 3-byte UTF-8
        let utf8_3 = "中文测试";
        let encoded = to_java_cesu8(utf8_3);
        assert!(matches!(encoded, Cow::Borrowed(_)));
        let decoded = from_java_cesu8(&encoded).unwrap();
        assert_eq!(decoded, utf8_3);

        // 4-byte UTF-8 (emoji)
        let emoji = "Hello 😀 World";
        let encoded = to_java_cesu8(emoji);
        assert!(matches!(encoded, Cow::Owned(_)));
        let decoded = from_java_cesu8(&encoded).unwrap();
        assert_eq!(decoded, emoji);

        // Null byte in string
        let with_null = "Hello\0World";
        let encoded = to_java_cesu8(with_null);
        assert!(matches!(encoded, Cow::Owned(_)));
        let decoded = from_java_cesu8(&encoded).unwrap();
        assert_eq!(decoded, with_null);

        // Verify exact byte encoding of null and supplementary characters
        assert_eq!(to_java_cesu8("\0").as_ref(), &[0xC0, 0x80]);
        assert_eq!(
            to_java_cesu8("\u{10401}").as_ref(),
            &[0xED, 0xA0, 0x81, 0xED, 0xB0, 0x81]
        );
        assert_eq!(
            from_java_cesu8(&[0xED, 0xA0, 0x81, 0xED, 0xB0, 0x81]).unwrap(),
            "\u{10401}"
        );
    }

    #[test]
    fn decoding_follows_javas_read_utf() {
        // Accepted like Java: raw 0, an overlong form, a lone surrogate.
        assert_eq!(from_java_cesu8(&[b'a', 0, b'b']).unwrap(), "a\0b");
        assert_eq!(from_java_cesu8(&[0xC1, 0x81]).unwrap(), "A");
        assert_eq!(from_java_cesu8(&[0xED, 0xA0, 0x81]).unwrap(), "\u{FFFD}");
        // Rejected like Java: Latin-1 (a lead byte without continuation),
        // a stray continuation byte, a 4-byte UTF-8 lead, a truncated end.
        for bad in [
            &[0xE4, b'u', b'f'][..],
            &[0x80],
            &[0xF0, 0x9F, 0x98, 0x80],
            &[0xC3],
        ] {
            assert!(from_java_cesu8(bad).is_err(), "{bad:x?} was accepted");
        }
    }
}
