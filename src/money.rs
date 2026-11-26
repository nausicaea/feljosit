//! Money values.
//!
//! The application keeps all amounts as integers in the minor unit of the currency
//! (for CHF: Rappen). This prevents rounding errors from binary floating point.

use std::fmt;

/// The money value is not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyError(pub String);

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MoneyError {}

/// ISO 4217 minor unit exponents. The default is 2.
const EXPONENTS: &[(&str, u32)] = &[
    ("BHD", 3),
    ("CLP", 0),
    ("ISK", 0),
    ("JOD", 3),
    ("JPY", 0),
    ("KRW", 0),
    ("KWD", 3),
    ("OMR", 3),
    ("TND", 3),
];

/// Return the currency code in upper case. Return an error if it is not valid.
pub fn validate_currency(code: &str) -> Result<String, MoneyError> {
    let code = code.trim().to_uppercase();
    if code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()) {
        Ok(code)
    } else {
        Err(MoneyError(format!("Currency code is not valid: {code:?}")))
    }
}

/// Return the number of decimal places for the currency.
pub fn exponent(currency: &str) -> u32 {
    let upper = currency.to_uppercase();
    EXPONENTS.iter().find(|(code, _)| *code == upper).map_or(2, |(_, exp)| *exp)
}

/// A decimal number without an exponent, for example `-1234.50`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    pub negative: bool,
    /// The digits before the decimal point (at least one digit).
    pub integer: String,
    /// The digits after the decimal point (can be empty).
    pub fraction: String,
}

impl Decimal {
    /// Parse `[+-]digits[.digits]`. Only ASCII digits. Return `None` if the syntax is
    /// not correct.
    pub fn parse(text: &str) -> Option<Decimal> {
        let (negative, rest) = match text.as_bytes().first() {
            Some(b'-') => (true, &text[1..]),
            Some(b'+') => (false, &text[1..]),
            _ => (false, text),
        };
        let (integer, fraction) = match rest.split_once('.') {
            Some((i, f)) => (i, f),
            None => (rest, ""),
        };
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if integer.is_empty() && fraction.is_empty() {
            return None;
        }
        if !digits(integer) || !digits(fraction) || (rest.contains('.') && fraction.is_empty()) {
            return None;
        }
        Some(Decimal {
            negative,
            integer: if integer.is_empty() { "0".into() } else { integer.into() },
            fraction: fraction.into(),
        })
    }

    /// The value scaled by `10^exp`. Return `None` if the result is not an integer or
    /// is too large.
    pub fn scaled(&self, exp: u32) -> Option<i128> {
        let exp = exp as usize;
        let (kept, rest) = if self.fraction.len() > exp {
            self.fraction.split_at(exp)
        } else {
            (self.fraction.as_str(), "")
        };
        if rest.bytes().any(|b| b != b'0') {
            return None;
        }
        let mut digits = self.integer.trim_start_matches('0').to_string();
        digits.push_str(kept);
        for _ in kept.len()..exp {
            digits.push('0');
        }
        let digits = digits.trim_start_matches('0');
        if digits.len() > 30 {
            return None;
        }
        let value: i128 = if digits.is_empty() { 0 } else { digits.parse().ok()? };
        Some(if self.negative { -value } else { value })
    }

    /// The value in thousandths, rounded down (for negative values: towards zero).
    fn thousandths_floor(&self) -> i128 {
        let mut fraction = self.fraction.clone();
        fraction.truncate(3);
        while fraction.len() < 3 {
            fraction.push('0');
        }
        let integer: i128 = self.integer.parse().unwrap_or(0);
        let value = integer * 1000 + fraction.parse::<i128>().unwrap_or(0);
        if self.negative { -value } else { value }
    }

    /// The absolute value.
    pub fn abs(&self) -> Decimal {
        Decimal { negative: false, ..self.clone() }
    }

    /// Compare two decimals that have a maximum of 3 decimal places.
    pub fn thousandths(&self) -> i128 {
        self.thousandths_floor()
    }
}

/// Convert a decimal amount to minor units.
///
/// Return an error if the amount has more decimal places than the currency permits.
pub fn to_minor(value: &Decimal, currency: &str) -> Result<i64, MoneyError> {
    let scaled = value.scaled(exponent(currency)).ok_or_else(|| {
        MoneyError(format!("Amount {} has too many decimal places for {currency}", decimal_text(value)))
    })?;
    i64::try_from(scaled).map_err(|_| MoneyError("Amount is too large".into()))
}

fn decimal_text(value: &Decimal) -> String {
    let sign = if value.negative { "-" } else { "" };
    if value.fraction.is_empty() {
        format!("{sign}{}", value.integer)
    } else {
        format!("{sign}{}.{}", value.integer, value.fraction)
    }
}

/// Parse a text with the syntax `[+-]digits[.digits]` to minor units.
pub fn text_to_minor(text: &str, currency: &str) -> Result<i64, MoneyError> {
    let value =
        Decimal::parse(text).ok_or_else(|| MoneyError(format!("Amount is not a number: {text:?}")))?;
    to_minor(&value, currency)
}

/// Parse an amount that a person typed, for example `1'234.50` or `-12`.
///
/// The function accepts Swiss thousands separators (apostrophe, right single quote,
/// space) and a comma as the decimal separator. The result has no currency.
pub fn parse_user_decimal(text: &str) -> Result<Decimal, MoneyError> {
    let mut cleaned: String =
        text.trim().chars().filter(|c| !matches!(c, '\'' | '\u{2019}' | ' ' | '\u{a0}' | '_')).collect();
    if cleaned.matches(',').count() == 1 && !cleaned.contains('.') {
        cleaned = cleaned.replace(',', ".");
    }
    let error = || MoneyError(format!("Amount is not valid: {text:?}"));
    let value = Decimal::parse(&cleaned).ok_or_else(error)?;
    let has_dot = cleaned.contains('.');
    let unsigned = cleaned.trim_start_matches(['+', '-']);
    let integer_digits = unsigned.split('.').next().unwrap_or("");
    if integer_digits.is_empty()
        || integer_digits.len() > 12
        || (has_dot && !(1..=3).contains(&value.fraction.len()))
        || cleaned.matches(['+', '-']).count() > 1
    {
        return Err(error());
    }
    Ok(value)
}

/// Parse an amount that a person typed (see [`parse_user_decimal`]) to minor units.
pub fn parse_user_amount(text: &str, currency: &str) -> Result<i64, MoneyError> {
    to_minor(&parse_user_decimal(text)?, currency)
}

/// Format minor units in the Swiss style, for example `-1'234.50`.
pub fn format_minor(amount: i64, currency: &str, with_currency: bool) -> String {
    let exp = exponent(currency);
    let sign = if amount < 0 { "-" } else { "" };
    let absolute = amount.unsigned_abs();
    let factor = 10u64.pow(exp);
    let (major, minor) = (absolute / factor, absolute % factor);
    let text = if exp > 0 {
        format!("{sign}{}.{minor:0width$}", group(major), width = exp as usize)
    } else {
        format!("{sign}{}", group(major))
    };
    if with_currency { format!("{currency} {text}") } else { text }
}

/// Group the digits in threes with an apostrophe, for example `1'234'567`.
fn group(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\'');
        }
        out.push(ch);
    }
    out
}

/// Round half to even, as Python's `round()`.
pub fn round_half_even(value: f64) -> i64 {
    value.round_ties_even() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format() {
        assert_eq!(format_minor(-123450, "CHF", false), "-1'234.50");
        assert_eq!(format_minor(5, "CHF", false), "0.05");
        assert_eq!(format_minor(0, "CHF", false), "0.00");
        assert_eq!(format_minor(123456789, "CHF", true), "CHF 1'234'567.89");
        assert_eq!(format_minor(1234, "JPY", false), "1'234");
        assert_eq!(format_minor(1234, "KWD", false), "1.234");
    }

    #[test]
    fn user_amounts() {
        assert_eq!(parse_user_amount("1'234.50", "CHF"), Ok(123450));
        assert_eq!(parse_user_amount("1’234.50", "CHF"), Ok(123450));
        assert_eq!(parse_user_amount("1 234,5", "CHF"), Ok(123450));
        assert_eq!(parse_user_amount("-12", "CHF"), Ok(-1200));
        assert_eq!(parse_user_amount("+3.1", "CHF"), Ok(310));
        assert!(parse_user_amount("12.345", "CHF").is_err());
        assert_eq!(parse_user_amount("12.345", "KWD"), Ok(12345));
        for bad in ["", "abc", "1.2.3", "1e5", "1234567890123", "1.", ".5", "--1", "1.2345"] {
            assert!(parse_user_amount(bad, "CHF").is_err(), "{bad}");
        }
    }

    #[test]
    fn minor_units() {
        assert_eq!(text_to_minor("12.500", "CHF"), Ok(1250));
        assert!(text_to_minor("12.505", "CHF").is_err());
        assert_eq!(text_to_minor("7", "JPY"), Ok(7));
        assert!(text_to_minor("7.5", "JPY").is_err());
    }

    #[test]
    fn currencies() {
        assert_eq!(validate_currency(" chf "), Ok("CHF".into()));
        assert!(validate_currency("CH").is_err());
        assert!(validate_currency("C1F").is_err());
        assert_eq!(exponent("chf"), 2);
        assert_eq!(exponent("JPY"), 0);
    }

    #[test]
    fn rounding() {
        assert_eq!(round_half_even(2.5), 2);
        assert_eq!(round_half_even(3.5), 4);
        assert_eq!(round_half_even(-2.5), -2);
        assert_eq!(round_half_even(2.6), 3);
    }
}
