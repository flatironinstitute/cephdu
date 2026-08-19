use std::collections::HashMap;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{fs, os::unix::fs::MetadataExt};

use crossterm::event::{self, Event, KeyCode, KeyModifiers, poll};

use ratatui::widgets::ListState;

use crate::fs::{FSType, get_fs, get_rbytes, get_rctime, get_rentries, id_to_name};
use crate::navigation;
use crate::popup::Popup;

pub const DEFAULT_SORT_MODE: SortMode = SortField::Size.default_mode();

pub struct App {
    pub should_exit: bool,
    pub cwd: PathBuf,
    pub dir_listing: DirListing,
    pub original_cwd: PathBuf,
    pub popup: Option<Popup>,
    pub show_owner: bool,
    pub show_ctime: bool,
    pub message: Option<Message>,
    highlighted: HashMap<PathBuf, (String, usize)>,
}

/// An encapsulation of a list of all files/dirs in a directory.
pub struct DirListing {
    dotdot: Option<DirEntry>,
    entries: Vec<DirEntry>,
    state: ListState,
    sort_mode: SortMode,
    pub stats: ListingStats,
    pub fs: Option<FSType>,
}

/// The size/rentries stats for a directory listing
pub struct ListingStats {
    pub max_rentries: usize,
    pub total_rentries: usize,
    pub max_size: usize,
    pub total_size: usize,
}

/// A single file/dir in the current directory.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<usize>,
    pub rentries: Option<usize>,
    pub ctime: Option<usize>,
    pub user: Option<String>,
    pub group: Option<String>,
}

impl DirEntry {
    fn from(path: PathBuf, stat: Metadata) -> Self {
        let kind = if stat.is_dir() {
            EntryKind::Dir
        } else if stat.is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };

        // we want to do our xattr calls asap to try and take advantage of MDS caching
        let rentries: Option<usize> = if kind == EntryKind::Dir {
            // rentries seems to include the self-count, which is confusing when there are
            // only N files but N+1 rentries.
            get_rentries(&path).map(|r| r.saturating_sub(1))
        } else {
            None
        };

        let size: Option<usize> = if kind == EntryKind::Dir {
            get_rbytes(&path)
        } else {
            Some(stat.len() as usize)
        };

        let ctime: Option<usize> = if kind == EntryKind::Dir {
            get_rctime(&path)
        } else {
            Some(stat.ctime() as usize)
        };

        let name_str = path.file_name().unwrap_or_default().to_string_lossy();
        let name = if kind == EntryKind::Dir {
            format!("{}/", name_str)
        } else {
            name_str.to_string()
        };

        let name_or_id = |id: u32| id_to_name(id).unwrap_or_else(|| format!("{}", id));

        let user = Some(name_or_id(stat.uid()));
        let group = Some(name_or_id(stat.gid()));

        DirEntry {
            name,
            kind,
            size,
            rentries,
            ctime,
            user,
            group,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Normal(SortField),
    Reversed(SortField),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Name,
    Size,
    Rentries,
    Owner,
    CTime,
}

impl SortField {
    /// The direction a field starts in when it is first chosen, by key or by flag.
    /// Sizes, counts and times read most-first; names and owners read ascending.
    pub const fn default_mode(self) -> SortMode {
        match self {
            SortField::Name | SortField::Owner => SortMode::Normal(self),
            SortField::Size | SortField::Rentries | SortField::CTime => SortMode::Reversed(self),
        }
    }
}

impl SortMode {
    pub fn field(&self) -> &SortField {
        match self {
            SortMode::Normal(field) => field,
            SortMode::Reversed(field) => field,
        }
    }

    pub fn is_reversed(&self) -> bool {
        matches!(self, SortMode::Reversed(_))
    }

    pub fn as_reversed(&self) -> SortMode {
        match self {
            SortMode::Normal(field) => SortMode::Reversed(*field),
            SortMode::Reversed(field) => SortMode::Normal(*field),
        }
    }

    pub fn same_field(&self, other: &SortMode) -> bool {
        self.field() == other.field()
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub text: String,
    pub kind: MessageKind,
}

#[derive(Debug, Clone, Copy)]
pub enum MessageKind {
    Error,
    Warning,
    Info,
}

impl App {
    pub fn new(cwd: Option<&PathBuf>, sort_mode: SortMode) -> Result<App, std::io::Error> {
        let cwd: PathBuf = if let Some(cwd) = cwd {
            cwd.clone()
        } else {
            std::env::current_dir()?
        };

        let dir_listing = DirListing::empty(sort_mode);
        let original_cwd = cwd.clone();
        let mut app = App {
            should_exit: false,
            cwd: PathBuf::new(),
            dir_listing,
            original_cwd,
            popup: None,
            show_owner: false,
            show_ctime: false,
            message: None,
            highlighted: HashMap::new(),
        };
        app.try_cd(&cwd)?;

        // Save the original (resolved) dir
        app.original_cwd = app.cwd.clone();

        Ok(app)
    }

    pub fn cd(&mut self, path: &PathBuf) {
        let res = self.try_cd(path);
        if let Err(e) = res {
            self.message(Some(Message {
                text: format!("Error changing directory: {}", e),
                kind: MessageKind::Error,
            }));
        }
    }

    fn try_cd(&mut self, path: &PathBuf) -> Result<(), std::io::Error> {
        // Record which entry was highlighted in case we navigate back
        self.save_selected();

        let new = if path.is_absolute() {
            path.canonicalize()?
        } else {
            self.cwd.join(path).canonicalize()?
        };
        self.dir_listing = DirListing::from(&new, self.dir_listing.sort_mode)?;
        self.cwd = new;
        if !self.dir_listing.is_ceph() {
            self.message(Some(Message {
                text: "Warning: not a Ceph directory".to_string(),
                kind: MessageKind::Warning,
            }));
        } else {
            self.message(None);
        }

        // Restore the highlighted entry if we have one
        self.restore_selected();
        Ok(())
    }

    pub fn popup(&mut self, title: Option<&str>, bottom_title: Option<&str>, text: Option<&str>) {
        self.popup = text.map(|x| Popup::new(title.unwrap_or(""), bottom_title.unwrap_or(""), x));
    }

    pub fn message(&mut self, message: Option<Message>) {
        self.message = message;
    }

    pub fn help(&mut self) {
        let lhs_width = navigation::HELP
            .iter()
            .map(|h| h[0].len())
            .max()
            .unwrap_or(0);
        let rhs_width = navigation::HELP
            .iter()
            .map(|h| h[1].len())
            .max()
            .unwrap_or(0);

        let mut help_text = String::new();
        for h in navigation::HELP {
            help_text.push_str(&format!(
                "{:>lhs$}:  {:rhs$}\n",
                h[0],
                h[1],
                lhs = lhs_width,
                rhs = rhs_width
            ));
        }
        self.popup(
            Some("Help"),
            Some(env!("CARGO_PKG_REPOSITORY")),
            Some(&help_text),
        );
    }

    pub fn sort_or_reverse(&mut self, sort_mode: SortMode) {
        self.dir_listing.sort(
            if sort_mode.field() == self.dir_listing.sort_mode().field() {
                self.dir_listing.sort_mode().as_reversed()
            } else {
                sort_mode
            },
        )
    }

    /// Save the currently selected entry in the highlighted map.
    fn save_selected(&mut self) {
        let selected = self.dir_listing.selected();
        if let Some(selected) = selected {
            let entry = self.dir_listing.get(selected);
            self.highlighted
                .insert(self.cwd.clone(), (entry.name.clone(), selected));
        }
    }

    /// Restore the previously highlighted entry if it exists.
    /// Try to select by name, and if that fails, select by index.
    fn restore_selected(&mut self) {
        if let Some((name, idx)) = self.highlighted.get(&self.cwd) {
            if self.dir_listing.select_by_name(name).is_none() {
                self.dir_listing.saturating_select(*idx);
            }
        } else {
            self.dir_listing.select_first_entry();
        }
    }
}

impl DirListing {
    pub fn from(path: &Path, sort_mode: SortMode) -> Result<DirListing, std::io::Error> {
        let path: PathBuf = path.canonicalize()?;
        let fs = get_fs(&path);

        let (entry_cwd, mut entries): (DirEntry, Vec<DirEntry>) = ls(&path)?;

        // Don't trust dir sizes on non-ceph!
        if !fs.map(FSType::is_ceph).unwrap_or(false) {
            entries
                .iter_mut()
                .filter(|e| e.kind == EntryKind::Dir)
                .for_each(|e| {
                    e.size = None;
                });
        }
        sort(&mut entries, sort_mode);

        let has_parent = *path != *"/";
        let dotdot = has_parent.then(dotdot_entry);

        let (max_rentries, max_size) = max_stats(&entries);
        // Note a possible consistency check we're not using here:
        // that the sum of the entry sizes add up to the cwd's r-sizes.
        let total_rentries = entry_cwd.rentries.unwrap_or(0);

        // TODO: might want to display ? instead of 0 for non-ceph
        let total_size = if fs.is_some_and(FSType::is_ceph) {
            entry_cwd.size.unwrap_or(0)
        } else {
            0
        };

        let state = ListState::default().with_selected(Some(0));

        Ok(DirListing {
            entries,
            state,
            dotdot,
            sort_mode,
            stats: ListingStats {
                max_rentries,
                total_rentries,
                max_size,
                total_size,
            },
            fs,
        })
    }

    /// Build a listing from entries that didn't come from a filesystem.
    #[cfg(test)]
    pub fn from_entries(
        mut entries: Vec<DirEntry>,
        has_dotdot: bool,
        sort_mode: SortMode,
    ) -> DirListing {
        sort(&mut entries, sort_mode);

        let (max_rentries, max_size) = max_stats(&entries);

        DirListing {
            dotdot: has_dotdot.then(dotdot_entry),
            stats: ListingStats {
                max_rentries,
                total_rentries: entries.iter().filter_map(|e| e.rentries).sum(),
                max_size,
                total_size: entries.iter().filter_map(|e| e.size).sum(),
            },
            entries,
            state: ListState::default().with_selected(Some(0)),
            sort_mode,
            fs: None,
        }
    }

    fn empty(sort_mode: SortMode) -> DirListing {
        DirListing {
            dotdot: None,
            entries: Vec::new(),
            state: ListState::default(),
            sort_mode,
            stats: ListingStats {
                max_rentries: 0,
                total_rentries: 0,
                max_size: 0,
                total_size: 0,
            },
            fs: None,
        }
    }

    /// Iterate the real entries in display order, without "..".
    pub fn iter_entries_sorted(&self) -> impl Iterator<Item = &DirEntry> {
        // `entries` is always sorted ascending; reversed modes are applied here
        // rather than by re-sorting.
        let entries_iter: Box<dyn Iterator<Item = &DirEntry>> = if self.sort_mode.is_reversed() {
            Box::new(self.entries.iter().rev())
        } else {
            Box::new(self.entries.iter())
        };

        entries_iter
    }

    /// Iterate the entries as displayed: ".." first if we have it, then the rest.
    pub fn iter_entries(&self) -> impl Iterator<Item = &DirEntry> {
        self.dotdot.iter().chain(self.iter_entries_sorted())
    }

    pub fn get(&self, idx: usize) -> &DirEntry {
        // idx = 0 is the ".." entry if we have one.
        // Otherwise, count from the back if we're displaying in reverse mode.

        let idx = if let Some(entry) = self.dotdot.iter().next() {
            if idx == 0 {
                return entry;
            }
            idx - 1
        } else {
            idx
        };

        if self.sort_mode.is_reversed() {
            &self.entries[self.entries.len() - idx - 1]
        } else {
            &self.entries[idx]
        }
    }

    pub fn len(&self) -> usize {
        // Count the ".." entry if we have one.
        let len = self.entries.len();
        if self.dotdot.is_some() { len + 1 } else { len }
    }

    pub fn select_next(&mut self, by: usize) {
        // Normally we would use state.select_next(), but that has a weird interaction
        // with the fact that we're manually rendering the list item highlighting.
        // Specifically, select_next() may scroll off the end of the list, so the
        // highlighting disappears. The state index is corrected after the list is
        // rendered, but then it's too late.
        let len = self.len();
        let state = &mut self.state;
        if let Some(idx) = state.selected() {
            let next = idx.saturating_add(by).min(len.saturating_sub(1));
            state.select(Some(next));
        } else {
            state.select(Some(0));
        }
    }

    pub fn select_prev(&mut self, by: usize) {
        let len = self.len();
        let state = &mut self.state;
        if let Some(idx) = state.selected() {
            let prev = idx.saturating_sub(by);
            state.select(Some(prev));
        } else {
            state.select(Some(len.saturating_sub(1)));
        }
    }

    /// Select the entry at the given index, or the last entry if the index is out of bounds.
    pub fn saturating_select(&mut self, idx: usize) -> usize {
        let len = self.len();
        let state = &mut self.state;
        if idx < len {
            state.select(Some(idx));
            idx
        } else {
            let newidx = len.saturating_sub(1);
            state.select(Some(newidx));
            newidx
        }
    }

    pub fn select_first(&mut self) {
        self.state.select(Some(0));
    }

    /// Select the first entry that isn't "..", which is what a directory being
    /// entered should land on. Falls back to ".." when there is nothing else.
    pub fn select_first_entry(&mut self) {
        let dotdot_only = self.entries.is_empty();
        let first = if self.dotdot.is_some() && !dotdot_only {
            1
        } else {
            0
        };
        self.state.select(Some(first));
    }

    pub fn select_last(&mut self) {
        let len = self.len();
        if len > 0 {
            self.state.select(Some(len - 1));
        }
    }

    pub fn select_by_name(&mut self, name: &str) -> Option<usize> {
        let idx = self.iter_entries().position(|entry| entry.name == name);
        if let Some(idx) = idx {
            self.state.select(Some(idx));
        }
        idx
    }

    pub fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    pub fn state_mut(&mut self) -> &mut ListState {
        &mut self.state
    }

    pub fn sort_mode(&self) -> SortMode {
        self.sort_mode
    }

    pub fn sort(&mut self, sort_mode: SortMode) {
        if self.sort_mode.same_field(&sort_mode) {
            self.sort_mode = sort_mode;
            return;
        }

        sort(&mut self.entries, sort_mode);

        self.sort_mode = sort_mode;
    }

    pub fn is_ceph(&self) -> bool {
        self.fs.is_some_and(|fs| fs.is_ceph())
    }
}

/// The synthetic ".." entry, which has no stat of its own.
fn dotdot_entry() -> DirEntry {
    DirEntry {
        name: "..".to_string(),
        kind: EntryKind::Dir,
        size: None,
        rentries: None,
        ctime: None,
        user: None,
        group: None,
    }
}

/// The largest size and rentries in the listing, which set the gauge scales.
fn max_stats(entries: &[DirEntry]) -> (usize, usize) {
    entries.iter().fold((0, 0), |(max_r, max_s), entry| {
        let r = entry.rentries.unwrap_or(0);
        let s = entry.size.unwrap_or(0);
        (max_r.max(r), max_s.max(s))
    })
}

fn sort(entries: &mut [DirEntry], sort_mode: SortMode) {
    // Every comparison ends on the name, which is unique within a directory. That
    // makes the order total, so the listing doesn't depend on readdir order.
    let by_name = |a: &DirEntry, b: &DirEntry| a.name.cmp(&b.name);

    match sort_mode.field() {
        SortField::Name => entries.sort_by(by_name),
        SortField::Size => entries.sort_by(|a, b| {
            a.size
                .cmp(&b.size)
                .then(a.rentries.cmp(&b.rentries))
                .then(by_name(a, b))
        }),
        SortField::Rentries => entries.sort_by(|a, b| {
            a.rentries
                .cmp(&b.rentries)
                .then(a.size.cmp(&b.size))
                .then(by_name(a, b))
        }),
        SortField::CTime => entries.sort_by(|a, b| {
            a.ctime
                .cmp(&b.ctime)
                .then(a.size.cmp(&b.size))
                .then(by_name(a, b))
        }),
        SortField::Owner => entries.sort_by(|a, b| {
            a.user
                .cmp(&b.user)
                .then(a.group.cmp(&b.group))
                .then(a.size.cmp(&b.size))
                .then(by_name(a, b))
        }),
    }
}

fn ls(path: &PathBuf) -> Result<(DirEntry, Vec<DirEntry>), std::io::Error> {
    let entry_cwd = DirEntry::from(PathBuf::from(path), fs::metadata(path)?);
    let dir_iterator = fs::read_dir(path)?;
    let mut entries: Vec<DirEntry> = Vec::new();

    for entry_result in dir_iterator {
        if poll(Duration::from_secs(0)).unwrap_or(false) {
            // If the user presses Ctrl-C during this loop, interrupt.
            // TODO: this is the wrong way to do this! The whole app should use an
            // async runtime that can handle key presses and interrupts.

            if let Ok(Event::Key(key)) = event::read()
                && key.code == KeyCode::Char('c')
                && key.modifiers.contains(KeyModifiers::CONTROL)
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "Interrupted by user",
                ));
            }
        }

        let entry = entry_result?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        entries.push(DirEntry::from(path, metadata));
    }

    Ok((entry_cwd, entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: usize) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            kind: EntryKind::Dir,
            size: Some(size),
            rentries: Some(size),
            ctime: Some(size),
            user: Some("alice".to_string()),
            group: Some("scc".to_string()),
        }
    }

    /// Ascending by size: small, medium, large.
    fn listing(has_dotdot: bool, sort_mode: SortMode) -> DirListing {
        DirListing::from_entries(
            vec![
                entry("large", 300),
                entry("small", 100),
                entry("medium", 200),
            ],
            has_dotdot,
            sort_mode,
        )
    }

    fn displayed(listing: &DirListing) -> Vec<String> {
        listing.iter_entries().map(|e| e.name.clone()).collect()
    }

    /// `get()` maps a selection index to what the same index displays. This is the
    /// pairing that the reversal and the ".." offset can each break.
    fn assert_get_matches_display(listing: &DirListing) {
        let names = displayed(listing);
        assert_eq!(names.len(), listing.len());
        for (i, name) in names.iter().enumerate() {
            assert_eq!(&listing.get(i).name, name, "get({}) disagrees", i);
        }
    }

    #[test]
    fn display_order_follows_direction() {
        let normal = listing(false, SortMode::Normal(SortField::Size));
        assert_eq!(displayed(&normal), ["small", "medium", "large"]);

        let reversed = listing(false, SortMode::Reversed(SortField::Size));
        assert_eq!(displayed(&reversed), ["large", "medium", "small"]);
    }

    #[test]
    fn dotdot_displays_first_in_both_directions() {
        for sort_mode in [
            SortMode::Normal(SortField::Size),
            SortMode::Reversed(SortField::Size),
        ] {
            let listing = listing(true, sort_mode);
            assert_eq!(displayed(&listing)[0], "..");
            assert_eq!(listing.len(), 4);
            assert_eq!(listing.iter_entries_sorted().count(), 3);
        }
    }

    #[test]
    fn get_agrees_with_display_order() {
        for has_dotdot in [false, true] {
            for field in [SortField::Name, SortField::Size, SortField::Rentries] {
                for sort_mode in [SortMode::Normal(field), SortMode::Reversed(field)] {
                    assert_get_matches_display(&listing(has_dotdot, sort_mode));
                }
            }
        }
    }

    /// Reversing must flip the display without disturbing the stored order, and
    /// `get()` has to keep up.
    #[test]
    fn reversing_in_place_keeps_get_consistent() {
        let mut listing = listing(true, SortMode::Normal(SortField::Size));
        assert_eq!(displayed(&listing), ["..", "small", "medium", "large"]);
        assert_get_matches_display(&listing);

        listing.sort(SortMode::Reversed(SortField::Size));
        assert_eq!(displayed(&listing), ["..", "large", "medium", "small"]);
        assert_get_matches_display(&listing);
    }

    #[test]
    fn selection_is_clamped_to_the_listing() {
        let mut listing = listing(true, DEFAULT_SORT_MODE);

        listing.select_last();
        assert_eq!(listing.selected(), Some(3));
        listing.select_next(1);
        assert_eq!(listing.selected(), Some(3), "ran off the end");
        listing.select_next(100);
        assert_eq!(listing.selected(), Some(3), "ran off the end");

        listing.select_prev(100);
        assert_eq!(listing.selected(), Some(0), "ran off the start");

        assert_eq!(listing.saturating_select(2), 2);
        assert_eq!(listing.saturating_select(99), 3);
    }

    /// An empty directory still has "..", and must not be indexed out of bounds.
    #[test]
    fn empty_listing_has_only_dotdot() {
        let mut listing = DirListing::from_entries(vec![], true, DEFAULT_SORT_MODE);
        assert_eq!(displayed(&listing), [".."]);
        assert_get_matches_display(&listing);

        listing.select_last();
        listing.select_next(1);
        assert_eq!(listing.selected(), Some(0));
    }

    #[test]
    fn select_by_name_uses_display_indices() {
        let mut listing = listing(true, SortMode::Reversed(SortField::Size));

        assert_eq!(listing.select_by_name("large"), Some(1));
        assert_eq!(listing.selected(), Some(1));
        assert_eq!(listing.get(1).name, "large");

        assert_eq!(listing.select_by_name(".."), Some(0));
        assert_eq!(listing.select_by_name("nonexistent"), None);
        assert_eq!(
            listing.selected(),
            Some(0),
            "failed lookup moved the cursor"
        );
    }

    #[test]
    fn arriving_selects_the_first_real_entry() {
        let mut listing = listing(true, DEFAULT_SORT_MODE);

        listing.select_first_entry();
        assert_eq!(listing.selected(), Some(1));
        assert_ne!(listing.get(1).name, "..");
    }

    /// ".." is the only thing left to select in an empty directory.
    #[test]
    fn arriving_in_an_empty_directory_selects_dotdot() {
        let mut listing = DirListing::from_entries(vec![], true, DEFAULT_SORT_MODE);
        listing.select_first_entry();
        assert_eq!(listing.selected(), Some(0));
        assert_eq!(listing.get(0).name, "..");
    }

    /// The root has no "..", so its first entry is at index 0.
    #[test]
    fn arriving_without_dotdot_selects_index_zero() {
        let mut listing = listing(false, DEFAULT_SORT_MODE);
        listing.select_first_entry();
        assert_eq!(listing.selected(), Some(0));
    }

    /// Entering a directory skips "..", but returning to one restores what was
    /// highlighted there. Uses the crate's own tree, which cargo makes the cwd.
    #[test]
    fn cd_selects_the_first_real_entry_then_remembers() {
        let mut app = App::new(Some(&PathBuf::from(".")), DEFAULT_SORT_MODE).unwrap();
        assert_eq!(app.dir_listing.selected(), Some(1));
        assert_ne!(app.dir_listing.get(1).name, "..");

        app.dir_listing
            .select_by_name("src/")
            .expect("src/ should be listed");
        app.cd(&PathBuf::from("src"));
        assert_eq!(app.dir_listing.selected(), Some(1), "did not skip '..'");

        app.cd(&PathBuf::from(".."));
        let selected = app.dir_listing.selected().unwrap();
        assert_eq!(app.dir_listing.get(selected).name, "src/");
    }

    /// The sort keys and the CLI sort flags share these directions, so they cannot
    /// drift apart.
    #[test]
    fn sort_fields_have_natural_directions() {
        for field in [SortField::Name, SortField::Owner] {
            assert!(
                !field.default_mode().is_reversed(),
                "{:?} should read ascending",
                field
            );
        }
        for field in [SortField::Size, SortField::Rentries, SortField::CTime] {
            assert!(
                field.default_mode().is_reversed(),
                "{:?} should read most-first",
                field
            );
        }
        for field in [
            SortField::Name,
            SortField::Size,
            SortField::Rentries,
            SortField::Owner,
            SortField::CTime,
        ] {
            assert_eq!(*field.default_mode().field(), field);
        }
    }

    /// A startup sort mode has to reach the listing App::new builds.
    #[test]
    fn new_honors_the_startup_sort_mode() {
        let mode = SortMode::Normal(SortField::Name);
        let app = App::new(Some(&PathBuf::from(".")), mode).unwrap();
        assert_eq!(app.dir_listing.sort_mode(), mode);

        let names: Vec<String> = app
            .dir_listing
            .iter_entries_sorted()
            .map(|e| e.name.clone())
            .collect();
        let mut ascending = names.clone();
        ascending.sort();
        assert_eq!(names, ascending, "listing is not in name order");
    }

    /// Names break ties so that the listing doesn't depend on readdir order.
    #[test]
    fn equal_entries_are_ordered_by_name() {
        let tied = || vec![entry("c", 1), entry("a", 1), entry("b", 1)];

        for field in [
            SortField::Size,
            SortField::Rentries,
            SortField::CTime,
            SortField::Owner,
        ] {
            let listing = DirListing::from_entries(tied(), false, SortMode::Normal(field));
            assert_eq!(
                displayed(&listing),
                ["a", "b", "c"],
                "{:?} is unstable",
                field
            );
        }
    }
}
