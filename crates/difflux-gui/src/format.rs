//! Small display helpers.

/// `1234567` → `1.2 MB`; exact counts stay exact below 1 KB.
pub(crate) fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 { format!("{value:.1} {}", UNITS[unit]) } else { format!("{value:.0} {}", UNITS[unit]) }
}

/// `1234567` → `1,234,567`.
pub(crate) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Local date and time for a millisecond timestamp.
pub(crate) fn timestamp(ms: Option<i64>, tz: &jiff::tz::TimeZone) -> String {
    ms.and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map(|t| t.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Number of decimal digits needed to print `n`.
pub(crate) fn digits(n: usize) -> usize {
    n.max(1).ilog10() as usize + 1
}

/// Replaces tabs with spaces so columns line up in the monospace view.
pub(crate) fn expand_tabs(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains('\t') { std::borrow::Cow::Owned(s.replace('\t', "    ")) } else { std::borrow::Cow::Borrowed(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(50 * 1024 * 1024), "50 MB");
    }

    #[test]
    fn grouping() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn digit_counts() {
        assert_eq!(digits(0), 1);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(10), 2);
        assert_eq!(digits(12_345), 5);
    }

    #[test]
    fn timestamps_render_in_the_given_zone() {
        let utc = jiff::tz::TimeZone::UTC;
        assert_eq!(timestamp(Some(0), &utc), "1970-01-01 00:00");
        assert_eq!(timestamp(None, &utc), "");
    }
}
