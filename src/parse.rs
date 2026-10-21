//! Parsers for the duration and size strings used in rules and flags.

use std::time::Duration;

/// Parse `30d`, `12h`, `2w`, `45s`, or `5m`.
///
/// The number and the unit may be separated by whitespace. Units are
/// case-insensitive. `m` means minutes, not months. The result is a whole
/// number of seconds.
pub fn parse_duration(input: &str) -> Result<Duration, String> {
    let (n, unit) = split_number_unit(input, "duration")?;
    if n == 0 {
        return Err("duration must be greater than zero".to_string());
    }
    let secs = match unit.to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => n,
        "m" | "min" | "mins" | "minute" | "minutes" => checked_mul(n, 60, input)?,
        "h" | "hr" | "hrs" | "hour" | "hours" => checked_mul(n, 60 * 60, input)?,
        "d" | "day" | "days" => checked_mul(n, 24 * 60 * 60, input)?,
        "w" | "week" | "weeks" => checked_mul(n, 7 * 24 * 60 * 60, input)?,
        other => {
            return Err(format!(
                "duration `{input}` has an unknown unit `{other}` (use s, m, h, d, or w)"
            ));
        }
    };
    Ok(Duration::from_secs(secs))
}

/// Parse `100MB`, `500KB`, `1GiB`, or a bare number of bytes.
///
/// Decimal units (KB, MB, GB, TB, and the single-letter forms K/M/G/T) are
/// powers of 1000. Binary units (KiB, MiB, GiB, TiB) are powers of 1024.
/// The comparison that uses the result is inclusive.
pub fn parse_size(input: &str) -> Result<u64, String> {
    let (n, unit) = split_number_unit(input, "size")?;
    let mult: u64 = match unit.to_ascii_lowercase().as_str() {
        "" | "b" | "byte" | "bytes" => 1,
        "k" | "kb" => 1_000,
        "m" | "mb" => 1_000_000,
        "g" | "gb" => 1_000_000_000,
        "t" | "tb" => 1_000_000_000_000,
        "kib" => 1024,
        "mib" => 1024 * 1024,
        "gib" => 1024u64.pow(3),
        "tib" => 1024u64.pow(4),
        other => {
            return Err(format!(
                "size `{input}` has an unknown unit `{other}` (examples: 500KB, 100MB, 1GiB)"
            ));
        }
    };
    n.checked_mul(mult)
        .ok_or_else(|| format!("size `{input}` is too large"))
}

/// Decimal (1000-based) size for tables. `1536` is `1.5 KB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[(u64, &str)] = &[
        (1_000_000_000_000, "TB"),
        (1_000_000_000, "GB"),
        (1_000_000, "MB"),
        (1_000, "KB"),
    ];
    for (unit, name) in UNITS {
        if bytes >= *unit {
            let value = bytes as f64 / *unit as f64;
            if value >= 10.0 {
                return format!("{value:.0} {name}");
            }
            return format!("{value:.1} {name}");
        }
    }
    format!("{bytes} B")
}

fn split_number_unit<'a>(input: &'a str, kind: &str) -> Result<(u64, &'a str), String> {
    let s = input.trim();
    if s.is_empty() {
        return Err(format!("{kind} is empty"));
    }
    let digits = s
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit())
        .map(|(i, _)| i)
        .last();
    let Some(last) = digits else {
        return Err(format!(
            "{kind} `{input}` must start with a number (examples: 30d, 12h, 100MB)"
        ));
    };
    let (num, rest) = s.split_at(last + 1);
    let n: u64 = num
        .parse()
        .map_err(|_| format!("{kind} `{input}` is not a valid number"))?;
    Ok((n, rest.trim()))
}

fn checked_mul(n: u64, factor: u64, input: &str) -> Result<u64, String> {
    n.checked_mul(factor)
        .ok_or_else(|| format!("duration `{input}` is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duration_units() {
        assert_eq!(parse_duration("45s").unwrap(), Duration::from_secs(45));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(5 * 60));
        assert_eq!(
            parse_duration("12h").unwrap(),
            Duration::from_secs(12 * 3600)
        );
        assert_eq!(
            parse_duration("30d").unwrap(),
            Duration::from_secs(30 * 86400)
        );
        assert_eq!(
            parse_duration("2w").unwrap(),
            Duration::from_secs(14 * 86400)
        );
        assert_eq!(
            parse_duration(" 2W ").unwrap(),
            Duration::from_secs(14 * 86400)
        );
        assert_eq!(parse_duration("1 day").unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn rejects_bad_durations() {
        for bad in ["", "d", "10x", "1.5d", "-1d", "0d", "30", "  d"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_decimal_and_binary_sizes() {
        assert_eq!(parse_size("100MB").unwrap(), 100 * 1_000_000);
        assert_eq!(parse_size("500KB").unwrap(), 500 * 1_000);
        assert_eq!(parse_size("1GiB").unwrap(), 1024u64.pow(3));
        assert_eq!(parse_size("1B").unwrap(), 1);
        assert_eq!(parse_size("2K").unwrap(), 2_000);
        assert_eq!(parse_size("2048").unwrap(), 2048);
        assert_eq!(parse_size("1 mib").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1TB").unwrap(), 1_000_000_000_000);
    }

    #[test]
    fn rejects_bad_sizes() {
        for bad in ["", "MB", "1.5MB", "-1KB", "10xb", "1PB"] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn formats_sizes_in_decimal_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1500), "1.5 KB");
        assert_eq!(format_size(1000), "1.0 KB");
        assert_eq!(format_size(12_400_000), "12 MB");
        assert_eq!(format_size(1_500_000_000), "1.5 GB");
    }
}
