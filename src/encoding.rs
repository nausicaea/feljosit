//! Small text encodings: hex, Base64, form data (`application/x-www-form-urlencoded`)
//! and JSON strings.
//!
//! These functions replace the crates `hex`, `base64`, `form_urlencoded` and
//! `serde_json` (see ADR-0021). The tests at the end of the file use the test vectors
//! of RFC 4648.

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
const BASE64_STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64_URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Lower-case hex digits of the bytes.
pub fn hex(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(HEX_DIGITS[usize::from(b >> 4)]));
        out.push(char::from(HEX_DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

fn base64_encode(bytes: &[u8], alphabet: &[u8; 64]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        // 2, 3 or 4 characters for 1, 2 or 3 bytes. No padding.
        for i in 0..=chunk.len() {
            out.push(char::from(alphabet[((n >> (18 - 6 * i)) & 0x3f) as usize]));
        }
    }
    out
}

/// Base64 with the standard alphabet and without padding (as in PHC strings).
pub fn base64_standard_no_pad(bytes: &[u8]) -> String {
    base64_encode(bytes, BASE64_STANDARD)
}

/// Base64 with the URL-safe alphabet and without padding (as Python's
/// `token_urlsafe`).
pub fn base64_url_safe_no_pad(bytes: &[u8]) -> String {
    base64_encode(bytes, BASE64_URL_SAFE)
}

/// Decode Base64 with the standard alphabet and without padding. The function refuses
/// padding, other characters, a length that is not possible and bits after the
/// last byte that are not zero. Thus each byte sequence has only one encoding.
pub fn base64_standard_no_pad_decode(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| BASE64_STANDARD.iter().position(|&a| a == c).map(|p| p as u32);
    let input = text.as_bytes();
    if input.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for chunk in input.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c)? << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let count = chunk.len() - 1;
        // The bits after the last byte must be zero.
        if count < 3 && n & ((1u32 << (8 * (3 - count))) - 1) != 0 {
            return None;
        }
        out.extend_from_slice(&bytes[..count]);
    }
    Some(out)
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decode one component of form data: `+` is a space and `%XX` is a byte. A `%`
/// without two hex digits stays as it is. Bytes that are not UTF-8 become U+FFFD.
/// This is the procedure of the WHATWG URL standard.
pub fn form_decode(component: &[u8]) -> String {
    let mut out = Vec::with_capacity(component.len());
    let mut i = 0;
    while i < component.len() {
        match component[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let high = component.get(i + 1).copied().and_then(hex_value);
                let low = component.get(i + 2).copied().and_then(hex_value);
                if let (Some(high), Some(low)) = (high, low) {
                    out.push(high << 4 | low);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse form data (`a=1&b=x+y`) into name and value pairs, in their sequence.
pub fn form_parse(data: &[u8]) -> Vec<(String, String)> {
    data.split(|&b| b == b'&')
        .filter(|part| !part.is_empty())
        .map(|part| match part.iter().position(|&b| b == b'=') {
            Some(pos) => (form_decode(&part[..pos]), form_decode(&part[pos + 1..])),
            None => (form_decode(part), String::new()),
        })
        .collect()
}

/// A JSON string literal (with the quotes) for the text.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON object with one member that has a text value, for example
/// `{"error":"..."}`.
pub fn json_object_with_string(name: &str, value: &str) -> String {
    format!("{{{}:{}}}", json_string(name), json_string(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_digits() {
        assert_eq!(hex([0x00, 0x7f, 0xab, 0xff]), "007fabff");
        assert_eq!(hex([]), "");
    }

    #[test]
    fn base64_rfc4648_vectors() {
        let vectors = [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg"),
            ("fooba", "Zm9vYmE"),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in vectors {
            assert_eq!(base64_standard_no_pad(plain.as_bytes()), encoded);
            assert_eq!(base64_standard_no_pad_decode(encoded).unwrap(), plain.as_bytes());
        }
        assert_eq!(base64_standard_no_pad(&[0xfb, 0xff]), "+/8");
        assert_eq!(base64_url_safe_no_pad(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn base64_decode_refuses_bad_input() {
        assert!(base64_standard_no_pad_decode("Zg==").is_none());
        assert!(base64_standard_no_pad_decode("Z").is_none());
        assert!(base64_standard_no_pad_decode("Zh").is_none()); // Bits after the byte.
        assert!(base64_standard_no_pad_decode("Zm9v!").is_none());
        assert!(base64_standard_no_pad_decode("-_8").is_none());
    }

    #[test]
    fn form_data() {
        assert_eq!(
            form_parse(b"a=1&b=x+y&&c&d=%C3%A4%2B%zz"),
            vec![
                ("a".into(), "1".into()),
                ("b".into(), "x y".into()),
                ("c".into(), String::new()),
                ("d".into(), "\u{e4}+%zz".into()),
            ]
        );
        assert_eq!(form_decode(b"%FF"), "\u{fffd}");
    }

    #[test]
    fn json() {
        assert_eq!(json_object_with_string("error", "a \"b\"\\\n\u{1}"), r#"{"error":"a \"b\"\\\n\u0001"}"#);
    }
}
