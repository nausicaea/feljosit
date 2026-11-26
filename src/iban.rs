//! IBAN validation (ISO 13616).

/// Lengths for countries that Swiss users see most frequently.
const LENGTHS: &[(&str, usize)] = &[("CH", 21), ("LI", 21), ("DE", 22), ("AT", 20), ("FR", 27), ("IT", 27)];

/// Remove the white space and change the IBAN to upper case.
pub fn normalise_iban(value: &str) -> String {
    value.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

/// Return true if the value is a valid IBAN (format, length and check digits).
pub fn is_valid_iban(value: &str) -> bool {
    let iban = normalise_iban(value);
    let bytes = iban.as_bytes();
    let format_ok = (15..=34).contains(&bytes.len())
        && bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2..4].iter().all(u8::is_ascii_digit)
        && bytes[4..].iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    if !format_ok {
        return false;
    }
    if let Some((_, expected)) = LENGTHS.iter().find(|(code, _)| *code == &iban[..2])
        && bytes.len() != *expected
    {
        return false;
    }
    let rearranged = bytes[4..].iter().chain(&bytes[..4]);
    let mut remainder: u32 = 0;
    for &b in rearranged {
        let value = if b.is_ascii_digit() { u32::from(b - b'0') } else { u32::from(b - b'A') + 10 };
        remainder = if value >= 10 { (remainder * 100 + value) % 97 } else { (remainder * 10 + value) % 97 };
    }
    remainder == 1
}

/// Format an IBAN in groups of four characters.
pub fn format_iban(value: &str) -> String {
    let iban: Vec<char> = normalise_iban(value).chars().collect();
    iban.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

/// Return true if the value is a Swiss QR-IBAN (IID 30000 to 31999).
pub fn is_qr_iban(value: &str) -> bool {
    let iban = normalise_iban(value);
    if !(iban.starts_with("CH") || iban.starts_with("LI")) || iban.len() != 21 {
        return false;
    }
    let iid = &iban[4..9];
    iid.bytes().all(|b| b.is_ascii_digit()) && iid.parse::<u32>().is_ok_and(|v| (30000..=31999).contains(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ibans() {
        assert!(is_valid_iban("CH93 0076 2011 6238 5295 7"));
        assert!(is_valid_iban("ch9300762011623852957"));
        assert!(is_valid_iban("DE89370400440532013000"));
        assert!(!is_valid_iban("CH93 0076 2011 6238 5295 8"));
        assert!(!is_valid_iban("CH93007620116238529"));
        assert!(!is_valid_iban("XX"));
        assert!(!is_valid_iban(""));
        assert!(!is_valid_iban("CH93-0076-2011-6238-5295-7"));
    }

    #[test]
    fn format() {
        assert_eq!(format_iban("CH9300762011623852957"), "CH93 0076 2011 6238 5295 7");
        assert_eq!(normalise_iban(" ch93 0076 2011 6238 5295 7 "), "CH9300762011623852957");
    }

    #[test]
    fn qr_iban() {
        assert!(is_qr_iban("CH0430000001234567890"));
        assert!(!is_qr_iban("CH93 0076 2011 6238 5295 7"));
    }
}
