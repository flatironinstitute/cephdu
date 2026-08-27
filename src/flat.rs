use std::io::Write;

use crate::app::{DirEntry, DirListing};
use crate::format::{CTIME_FMT_WIDTH, Numbers, ctime_str};

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
    /// Aligned columns with the same units the TUI shows. `exact` shows sizes and
    /// counts in full instead; the parseable format is always exact, so it has
    /// nothing to switch.
    Human { exact: bool },
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
        Format::Human { exact } => write_human(
            &entries,
            current_year,
            Numbers::from_exact(*exact),
            listing.options().owners,
            out,
        ),
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
    numbers: Numbers,
    owners: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let width = |f: &dyn Fn(&DirEntry) -> String| -> usize {
        entries.iter().map(|e| f(e).len()).max().unwrap_or(0)
    };

    let size = |e: &DirEntry| match e.size {
        Some(_) => numbers.size(e.size, true),
        None => MISSING.to_string(),
    };
    let rentries = |e: &DirEntry| match e.rentries {
        Some(_) => numbers.count(e.rentries, true),
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
    // Omitted rather than filled with placeholders when it wasn't read: this format
    // is for reading, and a column of dashes says nothing.
    let owner_width = if owners { width(&owner) } else { 0 };

    for entry in entries {
        let ctime = entry
            .ctime
            .map(|c| ctime_str(c, current_year))
            .unwrap_or(MISSING.to_string());

        let owner = if owners {
            format!("  {:<width$}", owner(entry), width = owner_width)
        } else {
            String::new()
        };

        writeln!(
            out,
            "{:>swidth$}  {:>rwidth$}  {:>cwidth$}{}  {}",
            size(entry),
            rentries(entry),
            ctime,
            owner,
            entry.display_name(),
            swidth = size_width,
            rwidth = rentries_width,
            cwidth = CTIME_FMT_WIDTH,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{EntryKind, Options, SortField, SortMode};

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
            Options {
                sort_mode: SortMode::Reversed(SortField::Size),
                dirs_first: false,
                // These entries carry owners and times, so the listing was read
                // with both.
                owners: true,
                times: true,
            },
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
        for format in [Format::Parseable, Format::Human { exact: false }] {
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
            Options {
                sort_mode: SortMode::Reversed(SortField::Size),
                dirs_first: false,
                // These entries carry owners and times, so the listing was read
                // with both.
                owners: true,
                times: true,
            },
        );
        assert_eq!(
            render(&listing, &Format::Parseable),
            "-\t-\t-\t-\t-\topaque/\n"
        );
    }

    #[test]
    fn human_columns_are_aligned() {
        let out = render(&listing(), &Format::Human { exact: false });
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

    /// Exact numbers, but still the aligned human layout rather than the parseable
    /// one.
    #[test]
    fn human_exact_shows_values_in_full() {
        let out = render(&listing(), &Format::Human { exact: true });
        let lines: Vec<&str> = out.lines().collect();

        assert!(lines[0].contains("1099511627776"), "{}", lines[0]);
        assert!(lines[0].contains("48213"), "{}", lines[0]);
        assert!(!lines[0].contains("1.1 TB"), "{}", lines[0]);

        assert!(!out.contains('\t'), "fell back to the parseable format");
        assert!(lines[0].contains("2001"), "{}", lines[0]);
        assert!(lines[0].contains("alice:scc"), "{}", lines[0]);

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
    }

    /// The human format marks a symlink; the parseable one must not, since a name may
    /// contain `@` and a parser has no way to tell a mark from a character.
    #[test]
    fn only_the_human_format_marks_symlinks() {
        let listing = DirListing::from_entries(
            vec![
                entry("link", EntryKind::Symlink, Some(8), None),
                entry("plain", EntryKind::File, Some(9), None),
            ],
            false,
            Options {
                sort_mode: SortMode::Reversed(SortField::Size),
                dirs_first: false,
                // These entries carry owners and times, so the listing was read
                // with both.
                owners: true,
                times: true,
            },
        );

        let human = render(&listing, &Format::Human { exact: false });
        assert!(human.contains("link@"), "{}", human);
        assert!(!human.contains("plain@"), "{}", human);

        let parseable = render(&listing, &Format::Parseable);
        assert!(!parseable.contains('@'), "{}", parseable);
        assert!(parseable.contains("\tlink\n"), "{}", parseable);
    }

    /// The owner column appears only when it was read. The human format drops it
    /// rather than filling it with placeholders; the parseable one keeps its six
    /// fields and marks them unavailable, so field offsets never move.
    #[test]
    fn the_owner_column_appears_only_when_it_was_read() {
        let unread = DirListing::from_entries(
            vec![DirEntry {
                user: None,
                group: None,
                ..entry("notes.txt", EntryKind::File, Some(120), None)
            }],
            false,
            Options::default(),
        );
        let read = DirListing::from_entries(
            vec![entry("notes.txt", EntryKind::File, Some(120), None)],
            false,
            Options {
                owners: true,
                ..Options::default()
            },
        );

        let human_unread = render(&unread, &Format::Human { exact: false });
        let human_read = render(&read, &Format::Human { exact: false });
        assert!(human_read.contains("alice:scc"), "{}", human_read);
        assert!(!human_unread.contains("alice"), "{}", human_unread);
        assert!(
            human_unread.len() < human_read.len(),
            "the column was not dropped: {:?}",
            human_unread
        );

        for listing in [&unread, &read] {
            let out = render(listing, &Format::Parseable);
            assert_eq!(out.trim_end().split('\t').count(), 6, "{}", out);
        }
        assert!(
            render(&unread, &Format::Parseable).contains("\t-\t-\t"),
            "the unread owner is not marked"
        );
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
