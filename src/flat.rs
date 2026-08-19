use std::io::Write;

use crate::app::{DirEntry, DirListing};
use crate::format::{CTIME_FMT_WIDTH, ctime_str, rentries_str, size_str};

/// Stands in for a value the filesystem didn't give us, so that every row has the
/// same number of fields.
const MISSING: &str = "-";

/// Flat mode always emits every column. The TUI hides owner and time because
/// terminal width is scarce, but a pipe has no width limit, and a fixed column
/// set is what makes the output parsable without flag-dependent field offsets.
pub enum Format {
    /// Tab-separated raw values: bytes, entry count, ctime as Unix seconds.
    /// Deliberately independent of whether stdout is a terminal.
    Parseable,
    /// Aligned columns with the same units the TUI shows.
    Human,
}

pub fn write_listing(
    listing: &DirListing,
    format: &Format,
    current_year: isize,
    out: &mut impl Write,
) -> std::io::Result<()> {
    // ".." is deliberately omitted: it isn't part of this directory's contents.
    let entries: Vec<&DirEntry> = listing.iter_entries_sorted().collect();

    match format {
        Format::Parseable => write_parseable(&entries, out),
        Format::Human => write_human(&entries, current_year, out),
    }
}

fn write_parseable(entries: &[&DirEntry], out: &mut impl Write) -> std::io::Result<()> {
    for entry in entries {
        let num = |v: Option<usize>| v.map_or(MISSING.to_string(), |v| v.to_string());
        let text = |v: &Option<String>| v.clone().unwrap_or(MISSING.to_string());

        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}",
            num(entry.size),
            num(entry.rentries),
            num(entry.ctime),
            text(&entry.user),
            text(&entry.group),
            entry.name,
        )?;
    }
    Ok(())
}

fn write_human(
    entries: &[&DirEntry],
    current_year: isize,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let width = |f: &dyn Fn(&DirEntry) -> String| -> usize {
        entries.iter().map(|e| f(e).len()).max().unwrap_or(0)
    };

    let size = |e: &DirEntry| match e.size {
        Some(_) => size_str(e.size, true),
        None => MISSING.to_string(),
    };
    let rentries = |e: &DirEntry| match e.rentries {
        Some(_) => rentries_str(e.rentries, true),
        None => MISSING.to_string(),
    };
    let owner = |e: &DirEntry| {
        format!(
            "{}:{}",
            e.user.clone().unwrap_or(MISSING.to_string()),
            e.group.clone().unwrap_or(MISSING.to_string())
        )
    };

    let size_width = width(&size);
    let rentries_width = width(&rentries);
    let owner_width = width(&owner);

    for entry in entries {
        let ctime = entry
            .ctime
            .map(|c| ctime_str(c, current_year))
            .unwrap_or(MISSING.to_string());

        writeln!(
            out,
            "{:>swidth$}  {:>rwidth$}  {:>cwidth$}  {:<owidth$}  {}",
            size(entry),
            rentries(entry),
            ctime,
            owner(entry),
            entry.name,
            swidth = size_width,
            rwidth = rentries_width,
            cwidth = CTIME_FMT_WIDTH,
            owidth = owner_width,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{EntryKind, SortField, SortMode};

    fn entry(
        name: &str,
        kind: EntryKind,
        size: Option<usize>,
        rentries: Option<usize>,
    ) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            kind,
            size,
            rentries,
            // 2001-06-15T12:00:00Z: mid-year, so the year is timezone-independent,
            // and old enough to exercise the year-showing branch of the format
            ctime: Some(992_606_400),
            user: Some("alice".to_string()),
            group: Some("scc".to_string()),
        }
    }

    fn listing() -> DirListing {
        DirListing::from_entries(
            vec![
                entry(
                    "data/",
                    EntryKind::Dir,
                    Some(1_099_511_627_776),
                    Some(48_213),
                ),
                entry("bigfile.h5", EntryKind::File, Some(2_147_483_648), None),
                entry("notes.txt", EntryKind::File, Some(120), None),
            ],
            true,
            SortMode::Reversed(SortField::Size),
            false,
        )
    }

    fn render(listing: &DirListing, format: &Format) -> String {
        let mut buf: Vec<u8> = Vec::new();
        write_listing(listing, format, 2026, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn parseable_is_tab_separated_with_six_fields() {
        let out = render(&listing(), &Format::Parseable);
        assert_eq!(
            out,
            "1099511627776\t48213\t992606400\talice\tscc\tdata/\n\
             2147483648\t-\t992606400\talice\tscc\tbigfile.h5\n\
             120\t-\t992606400\talice\tscc\tnotes.txt\n"
        );
    }

    /// ".." exists in the listing but must never be printed.
    #[test]
    fn dotdot_is_omitted() {
        for format in [Format::Parseable, Format::Human] {
            let out = render(&listing(), &format);
            assert!(!out.contains(".."), "{:?} leaked '..'", out);
            assert_eq!(out.lines().count(), 3);
        }
    }

    #[test]
    fn parseable_survives_missing_values() {
        let listing = DirListing::from_entries(
            vec![DirEntry {
                user: None,
                group: None,
                ctime: None,
                ..entry("opaque/", EntryKind::Dir, None, None)
            }],
            false,
            SortMode::Reversed(SortField::Size),
            false,
        );
        assert_eq!(
            render(&listing, &Format::Parseable),
            "-\t-\t-\t-\t-\topaque/\n"
        );
    }

    #[test]
    fn human_columns_are_aligned() {
        let out = render(&listing(), &Format::Human);
        let lines: Vec<&str> = out.lines().collect();

        // Every row has to agree on where the name column starts.
        let name_starts: Vec<usize> = lines
            .iter()
            .zip(["data/", "bigfile.h5", "notes.txt"])
            .map(|(line, name)| line.rfind(name).unwrap())
            .collect();
        assert!(
            name_starts.windows(2).all(|w| w[0] == w[1]),
            "ragged name column in\n{}",
            out
        );

        assert!(lines[0].contains("1.1 TB"), "{}", lines[0]);
        assert!(lines[0].contains("48.2 K"), "{}", lines[0]);
        assert!(lines[0].contains("2001"), "{}", lines[0]);
        assert!(lines[0].contains("alice:scc"), "{}", lines[0]);
    }

    /// Display order follows the sort mode, and reversal must not be re-sorted in.
    #[test]
    fn respects_sort_mode() {
        let mut listing = listing();

        listing.sort(SortMode::Normal(SortField::Size));
        let names: Vec<String> = render(&listing, &Format::Parseable)
            .lines()
            .map(|l| l.rsplit('\t').next().unwrap().to_string())
            .collect();
        assert_eq!(names, ["notes.txt", "bigfile.h5", "data/"]);

        listing.sort(SortMode::Normal(SortField::Name));
        let names: Vec<String> = render(&listing, &Format::Parseable)
            .lines()
            .map(|l| l.rsplit('\t').next().unwrap().to_string())
            .collect();
        assert_eq!(names, ["bigfile.h5", "data/", "notes.txt"]);
    }
}
