use chrono::{DateTime, Datelike, Local};

/// Width of the strings returned by [`ctime_str`]:
/// 'Jan  1  2000' or 'Dec 31 12:34'
pub const CTIME_FMT_WIDTH: usize = 12;

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
