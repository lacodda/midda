//! Numbers as a reader reads them: `1.2 GB`, `36%`.
//!
//! The window writes the same numbers with `formatBytes` and `formatShare`
//! (`src/format.ts`). A sentence the core writes is read beside the window's
//! figures, and the two must say the same thing for the same bytes - a story
//! that says 43 GB over a row that says 42.7 GB reads as two measurements. The
//! tests here are the window's, case for case; a change to one is a change to
//! both.

/// The units, in binary steps with the names Explorer gives them: the number
/// a reader compares this with is the one Windows shows them.
const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// A size: one decimal below ten from a kilobyte up, none past ten or for
/// bare bytes.
#[must_use]
pub fn bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_owned();
    }
    let mut step = ((63 - bytes.leading_zeros()) / 10).min(5) as usize;
    #[allow(clippy::cast_precision_loss, reason = "a size is shown to three figures")]
    let mut value = bytes as f64 / 1024_f64.powi(i32::try_from(step).unwrap_or(0));
    let mut digits = if step == 0 { 0 } else { usize::from(value < 10.0) };
    // Rounding can fill the unit it was chosen for - 1023.6 GB is "1024 GB",
    // which nobody writes - so it carries into the next one.
    if rounded(value, digits) >= 1024.0 && step < UNITS.len() - 1 {
        step += 1;
        value /= 1024.0;
        digits = usize::from(value < 10.0);
    }
    format!("{:.*} {}", digits, rounded(value, digits), UNITS[step])
}

/// What share of `total` `part` is, as a percentage: one decimal below ten,
/// and "<0.1%" for a share too small to round to anything but zero.
#[must_use]
pub fn share(part: u64, total: u64) -> String {
    if total == 0 {
        return "0%".to_owned();
    }
    #[allow(clippy::cast_precision_loss, reason = "a share is shown to three figures")]
    let share = part as f64 / total as f64 * 100.0;
    if share > 0.0 && share < 0.1 {
        return "<0.1%".to_owned();
    }
    let digits = usize::from(share < 10.0);
    format!("{:.*}%", digits, rounded(share, digits))
}

/// `value` rounded to `digits` decimals, a half upwards - as JavaScript's
/// `toFixed` rounds, where Rust's own formatting would round a half to even
/// and write 2.2 where the window writes 2.3.
fn rounded(value: f64, digits: usize) -> f64 {
    let scale = 10_f64.powi(i32::try_from(digits).unwrap_or(0));
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    const KB: u64 = 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "test sizes are small and positive")]
    fn of(gigabytes: f64) -> u64 {
        (gigabytes * GB as f64) as u64
    }

    #[test]
    fn names_the_binary_steps_the_way_explorer_does() {
        assert_eq!(bytes(KB), "1.0 KB");
        assert_eq!(bytes(KB * KB), "1.0 MB");
        assert_eq!(bytes(GB), "1.0 GB");
        assert_eq!(bytes(GB * KB), "1.0 TB");
    }

    #[test]
    fn keeps_a_decimal_where_it_separates_two_folders_and_drops_it_where_it_does_not() {
        assert_eq!(bytes(of(1.2)), "1.2 GB");
        assert_eq!(bytes(of(1.9)), "1.9 GB");
        assert_eq!(bytes(of(42.7)), "43 GB");
    }

    #[test]
    fn shows_bare_bytes_as_whole_numbers() {
        assert_eq!(bytes(1), "1 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(0), "0 B");
    }

    #[test]
    fn carries_into_the_next_unit_when_rounding_fills_the_current_one() {
        assert_eq!(bytes(GB * KB - 1), "1.0 TB");
        assert_eq!(bytes(of(1023.6)), "1.0 TB");
        assert_eq!(bytes(KB * KB - 1), "1.0 MB");
        assert_eq!(bytes(1023 * GB), "1023 GB");
    }

    #[test]
    fn does_not_run_off_the_end_of_its_units() {
        assert!(bytes(u64::MAX).ends_with("PB"));
    }

    #[test]
    fn rounds_a_half_up_as_the_window_does() {
        // 2.25 GB is exact in binary; half-to-even would write 2.2.
        assert_eq!(bytes(2 * GB + GB / 4), "2.3 GB");
        assert_eq!(share(1, 8), "13%", "12.5 rounds up");
    }

    #[test]
    fn gives_a_percentage_of_a_total() {
        assert_eq!(share(50, 200), "25%");
        assert_eq!(share(1, 200), "0.5%");
        assert_eq!(share(0, 0), "0%");
        assert_eq!(share(1, 10_000), "<0.1%");
        assert_eq!(share(0, 10), "0.0%");
    }
}
