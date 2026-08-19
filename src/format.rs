use chrono::{DateTime, Datelike, Local};

/// Width of the strings returned by [`ctime_str`]:
/// 'Jan  1  2000' or 'Dec 31 12:34'
pub const CTIME_FMT_WIDTH: usize = 12;

/// Advance to the next unit when rounding to one decimal would carry, so that
/// 999_999 shows as "1.0 MB" rather than "1000.0 KB", which is a character wider
/// than the columns allow.
fn carry_unit(value: usize, unit_index: u32, num_units: usize) -> u32 {
    let scaled = value as f64 / 1000f64.powi(unit_index as i32);
    if scaled >= 999.95 && (unit_index as usize) + 1 < num_units {
        unit_index + 1
    } else {
        unit_index
    }
}

/// Format a byte count with base-1000 units. `align` pads the unit-less case so
/// that the unit suffixes line up in a right-aligned column.
pub fn size_str(size: Option<usize>, align: bool) -> String {
    if size.is_none() {
        return "".to_string();
    }
    let size = size.unwrap();
    let units = [" B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
    let base: usize = 1000;
    let i = if size > 0 {
        size.ilog10() / base.ilog10()
    } else {
        0
    };
    let i = carry_unit(size, i, units.len());
    let size = size as f64 / base.pow(i) as f64;
    if i == 0 {
        format!(
            "{:.0}{}{}",
            size,
            if align { "  " } else { "" },
            units[i as usize]
        )
    } else {
        format!("{:.1} {}", size, units[i as usize])
    }
}

/// Format a file count with base-1000 units.
pub fn rentries_str(rentries: Option<usize>, align: bool) -> String {
    if rentries.is_none() {
        return "".to_string();
    }
    let rentries = rentries.unwrap();
    let units = ["", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let base: usize = 1000;
    let i = if rentries > 0 {
        rentries.ilog10() / base.ilog10()
    } else {
        0
    };
    let i = carry_unit(rentries, i, units.len());
    let rentries = rentries as f64 / base.pow(i) as f64;
    if i == 0 {
        format!("{:.0}{}", rentries, if align { "    " } else { "" })
    } else {
        format!("{:.1} {}", rentries, units[i as usize])
    }
}

/// Format a Unix timestamp like 'ls -l' does: times within `current_year` show
/// the clock time, older ones show the year instead.
pub fn ctime_str(ctime_seconds: usize, current_year: isize) -> String {
    let Ok(secs) = i64::try_from(ctime_seconds) else {
        return String::new();
    };
    let Some(ctime) = DateTime::from_timestamp_secs(secs) else {
        return String::new();
    };
    let ctime: DateTime<Local> = ctime.into();
    let fmt = if (ctime.year() as isize) == current_year {
        "%b %e %H:%M"
    } else {
        "%b %e  %Y"
    };
    ctime.format(fmt).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_units() {
        assert_eq!(size_str(None, false), "");
        assert_eq!(size_str(Some(0), false), "0 B");
        assert_eq!(size_str(Some(1), false), "1 B");
        assert_eq!(size_str(Some(999), false), "999 B");
        assert_eq!(size_str(Some(1000), false), "1.0 KB");
        assert_eq!(size_str(Some(1500), false), "1.5 KB");
        assert_eq!(size_str(Some(1_000_000), false), "1.0 MB");
        assert_eq!(size_str(Some(1_500_000_000), false), "1.5 GB");
        assert_eq!(size_str(Some(1_000_000_000_000), false), "1.0 TB");
        assert_eq!(size_str(Some(1_000_000_000_000_000), false), "1.0 PB");
    }

    /// `align` may only pad: it must not change the value or the unit.
    #[test]
    fn size_align_only_pads() {
        for size in [0usize, 1, 999, 1000, 1_500_000, usize::MAX] {
            let tokens = |align| -> Vec<String> {
                size_str(Some(size), align)
                    .split_whitespace()
                    .map(str::to_string)
                    .collect()
            };
            assert_eq!(tokens(true), tokens(false), "size {} changed", size);
        }
    }

    /// The list layout right-aligns sizes in an 8-wide field; overflowing it would
    /// shift every column after it.
    #[test]
    fn size_fits_its_column() {
        for size in [0usize, 1, 999, 1000, 999_999, usize::MAX] {
            let s = size_str(Some(size), true);
            assert!(s.len() <= 8, "{:?} is wider than its column", s);
        }
    }

    #[test]
    fn rentries_units() {
        assert_eq!(rentries_str(None, false), "");
        assert_eq!(rentries_str(Some(0), false), "0");
        assert_eq!(rentries_str(Some(999), false), "999");
        assert_eq!(rentries_str(Some(1000), false), "1.0 K");
        assert_eq!(rentries_str(Some(48_213), false), "48.2 K");
        assert_eq!(rentries_str(Some(1_000_000), false), "1.0 M");
    }

    #[test]
    fn rentries_align_only_pads() {
        for rentries in [0usize, 1, 999, 1000, 48_213, usize::MAX] {
            let tokens = |align| -> Vec<String> {
                rentries_str(Some(rentries), align)
                    .split_whitespace()
                    .map(str::to_string)
                    .collect()
            };
            assert_eq!(tokens(true), tokens(false), "count {} changed", rentries);
        }
    }

    /// The list layout right-aligns counts in a 7-wide field.
    #[test]
    fn rentries_fits_its_column() {
        for rentries in [0usize, 1, 999, 1000, 999_999, usize::MAX] {
            let s = rentries_str(Some(rentries), true);
            assert!(s.len() <= 7, "{:?} is wider than its column", s);
        }
    }

    /// Both branches of the format must produce the width the column layout assumes.
    /// Timestamps here are mid-year and mid-day so no timezone can shift the year
    /// or the day-of-month digit count.
    #[test]
    fn ctime_width_is_constant() {
        // 2001-06-05T12:00:00Z (one-digit day) and 2001-06-15T12:00:00Z
        for secs in [991_742_400usize, 992_606_400] {
            for current_year in [2001isize, 2026] {
                let s = ctime_str(secs, current_year);
                assert_eq!(s.len(), CTIME_FMT_WIDTH, "{:?} has wrong width", s);
            }
        }
    }

    #[test]
    fn ctime_shows_year_only_for_other_years() {
        let secs = 992_606_400; // 2001-06-15T12:00:00Z
        assert!(ctime_str(secs, 2026).contains("2001"));
        assert!(!ctime_str(secs, 2001).contains("2001"));
    }

    /// A garbage rctime xattr must not panic.
    #[test]
    fn ctime_out_of_range_is_empty() {
        assert_eq!(ctime_str(usize::MAX, 2026), "");
    }
}
