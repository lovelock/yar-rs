//! Helpers shared by the profiler tooling (CLI arguments, load generation).
//!
//! This module holds parsing utilities used by the profiler binaries so the
//! bins stay thin. The load generator itself (Task 4) will build on this
//! module as well.

use std::time::Duration;

/// Split `s` at the start of its trailing ASCII-alphabetic suffix.
///
/// Returns `(digits, suffix)` where `suffix` may be empty (bare number).
/// Returns `None` when `s` is empty or contains no non-alphabetic character
/// at all (e.g. `"ms"` alone).
fn split_numeric_tail(s: &str) -> Option<(&str, &str)> {
    let last_non_alpha = s.rfind(|c: char| !c.is_ascii_alphabetic())?;
    let digits_end = last_non_alpha + 1;
    Some((&s[..digits_end], &s[digits_end..]))
}

/// Parse a size string such as `"256"`, `"256b"`, `"1k"` or `"8m"`.
///
/// A bare number is bytes. Suffixes are case-insensitive:
/// `b` = 1, `k` = 1024, `m` = 1024 * 1024.
///
/// Errors on empty input, unknown suffixes, non-numeric components and
/// overflow (checked at parse and at multiplication).
pub fn parse_size_suffix(s: &str) -> Result<usize, String> {
    let (digits, suffix) =
        split_numeric_tail(s).ok_or_else(|| format!("invalid size '{}': empty", s))?;

    let multiplier: usize = match suffix {
        "" => 1,
        "b" | "B" => 1,
        "k" | "K" => 1024,
        "m" | "M" => 1024 * 1024,
        _ => return Err(format!("invalid size '{}': unknown suffix '{}'", s, suffix)),
    };

    if digits.is_empty() {
        return Err(format!("invalid size '{}': missing number", s));
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid size '{}': not a number", s));
    }

    let n: usize = digits
        .parse()
        .map_err(|_| format!("invalid size '{}': too large", s))?;
    n.checked_mul(multiplier)
        .ok_or_else(|| format!("invalid size '{}': too large", s))
}

/// Parse a duration string such as `"5"`, `"5s"`, `"250ms"` or `"2MS"`.
///
/// A bare number is seconds. Supported suffixes (case-insensitive): `s`
/// (seconds) and `ms` (milliseconds).
///
/// Errors on empty input, unknown suffixes, non-numeric components and
/// overflow.
pub fn parse_duration_suffix(s: &str) -> Result<Duration, String> {
    let (digits, suffix) =
        split_numeric_tail(s).ok_or_else(|| format!("invalid duration '{}': empty", s))?;

    if digits.is_empty() {
        return Err(format!("invalid duration '{}': missing number", s));
    }
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("invalid duration '{}': too large", s))?;

    match suffix {
        "" | "s" | "S" => Ok(Duration::from_secs(n)),
        "ms" | "MS" | "Ms" | "mS" => Ok(Duration::from_millis(n)),
        _ => Err(format!(
            "invalid duration '{}': unknown suffix '{}'",
            s, suffix
        )),
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- sizes ----------------------------------------------------------------

    #[test]
    fn parse_size_bare_number_is_bytes() {
        assert_eq!(parse_size_suffix("256").unwrap(), 256);
        assert_eq!(parse_size_suffix("0").unwrap(), 0);
    }

    #[test]
    fn parse_size_b_suffix() {
        assert_eq!(parse_size_suffix("256b").unwrap(), 256);
        assert_eq!(parse_size_suffix("256B").unwrap(), 256);
    }

    #[test]
    fn parse_size_k_suffix() {
        assert_eq!(parse_size_suffix("1k").unwrap(), 1024);
        assert_eq!(parse_size_suffix("1K").unwrap(), 1024);
    }

    #[test]
    fn parse_size_m_suffix() {
        assert_eq!(parse_size_suffix("8m").unwrap(), 8 * 1024 * 1024);
        assert_eq!(parse_size_suffix("8M").unwrap(), 8 * 1024 * 1024);
    }

    #[test]
    fn parse_size_rejects_empty() {
        assert!(parse_size_suffix("").is_err());
    }

    #[test]
    fn parse_size_rejects_unknown_suffix() {
        assert!(parse_size_suffix("512x").is_err());
        assert!(parse_size_suffix("512g").is_err());
    }

    #[test]
    fn parse_size_rejects_suffix_without_number() {
        assert!(parse_size_suffix("m").is_err());
        assert!(parse_size_suffix("kb").is_err());
    }

    #[test]
    fn parse_size_rejects_non_numeric() {
        assert!(parse_size_suffix("12x3k").is_err());
        assert!(parse_size_suffix("abc").is_err());
    }

    #[test]
    fn parse_size_rejects_overflow() {
        // Parse overflow: exceeds usize range.
        assert!(parse_size_suffix("99999999999999999999").is_err());
        // Multiplication overflow: parses, but exceeds usize when scaled.
        assert!(parse_size_suffix("17863391866455421m").is_err());
    }

    // -- durations ------------------------------------------------------------

    #[test]
    fn parse_duration_bare_number_is_seconds() {
        assert_eq!(parse_duration_suffix("5").unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn parse_duration_seconds_suffix() {
        assert_eq!(parse_duration_suffix("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_duration_suffix("5S").unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn parse_duration_milliseconds_suffix() {
        assert_eq!(
            parse_duration_suffix("250ms").unwrap(),
            Duration::from_millis(250)
        );
        assert_eq!(
            parse_duration_suffix("250MS").unwrap(),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn parse_duration_rejects_empty() {
        assert!(parse_duration_suffix("").is_err());
    }

    #[test]
    fn parse_duration_rejects_unknown_suffix() {
        assert!(parse_duration_suffix("5x").is_err());
        assert!(parse_duration_suffix("5m").is_err());
        assert!(parse_duration_suffix("5us").is_err());
    }

    #[test]
    fn parse_duration_rejects_suffix_without_number() {
        assert!(parse_duration_suffix("s").is_err());
        assert!(parse_duration_suffix("ms").is_err());
    }

    #[test]
    fn parse_duration_rejects_non_numeric() {
        assert!(parse_duration_suffix("5x7s").is_err());
        assert!(parse_duration_suffix("abc").is_err());
    }
}
