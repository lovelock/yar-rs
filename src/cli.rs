//! Small command-line parsing helpers shared by the binaries.

use std::time::Duration;

/// Split `s` at the start of its trailing ASCII-alphabetic suffix.
fn split_numeric_tail(s: &str) -> Option<(&str, &str)> {
    let last_non_alpha = s.rfind(|c: char| !c.is_ascii_alphabetic())?;
    let digits_end = last_non_alpha + 1;
    Some((&s[..digits_end], &s[digits_end..]))
}

/// Parse a size string such as `"256"`, `"256b"`, `"1k"` or `"8m"`.
///
/// A bare number is bytes. Suffixes are case-insensitive:
/// `b` = 1, `k` = 1024, `m` = 1024 * 1024.
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
