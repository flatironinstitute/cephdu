use std::cmp::Ordering;
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

/// How a directory is read and ordered. Carried across directory changes, and the
/// reason `DirListing::from` takes one value rather than a run of booleans.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub sort_mode: SortMode,
    pub dirs_first: bool,
    /// Whether to stat for the owner. Off by default because a directory needs no
    /// stat at all without it: its size, count and kind all come from elsewhere. The
    /// uid-to-name lookup is the cheap half -- measured at ~18us from SSSD's cache on
    /// Rusty against 1.37ms for one xattr read -- so it is the stat that is saved.
    pub owners: bool,
    /// Whether to read a directory's recursive time. Off by default because it is a
    /// third of the xattr round trips a listing makes, and measurably so: 42s to 30s
    /// on ten thousand directories. A file's time comes from the stat it needs
    /// anyway, so this only concerns directories.
    pub times: bool,
}

impl Options {
    /// Just the ordering, with the defaults for the rest.
    #[cfg(test)]
    pub fn sorted(sort_mode: SortMode) -> Options {
        Options {
            sort_mode,
            ..Options::default()
        }
    }
}

impl Default for Options {
    fn default() -> Options {
        Options {
            sort_mode: DEFAULT_SORT_MODE,
            dirs_first: false,
            owners: false,
            times: false,
        }
    }
}

pub struct App {
    pub should_exit: bool,
    pub cwd: PathBuf,
    pub dir_listing: DirListing,
    pub original_cwd: PathBuf,
    pub popup: Option<Popup>,
    pub show_owner: bool,
    pub show_ctime: bool,
    /// Show sizes and counts in full instead of scaled to a unit.
    pub exact: bool,
    /// Columns scrolled off the left of the listing. Clamped when rendered, since
    /// only the renderer knows how wide the rows came out.
    pub hscroll: usize,
    pub message: Option<Message>,
    highlighted: HashMap<PathBuf, (String, usize)>,
}

/// An encapsulation of a list of all files/dirs in a directory.
pub struct DirListing {
    dotdot: Option<DirEntry>,
    entries: Vec<DirEntry>,
    state: ListState,
    options: Options,
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
    /// The name as shown to a person, marked the way `ls -F` marks: a trailing `@`
    /// for a symlink. The directory's `/` is already part of `name` instead, because
    /// the parseable format documents it and `/` is the one byte a filename cannot
    /// contain. `@` can, so it stays out of that stream and is applied here.
    pub fn display_name(&self) -> String {
        match self.kind {
            EntryKind::Symlink => format!("{}@", self.name),
            _ => self.name.clone(),
        }
    }

    /// `stat` is absent when nothing needed it: a directory's size and count come
    /// from the xattrs and its kind from readdir, so the only thing left for a stat
    /// to answer is who owns it. What to fetch is read from `options` rather than
    /// inferred from the stat: a file is stat'd for its size regardless, so for a file
    /// the only thing `owners` saves is the uid-to-name lookup, once per distinct
    /// owner.
    fn from(path: PathBuf, kind: EntryKind, stat: Option<Metadata>, options: &Options) -> Self {
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
            stat.as_ref().map(|s| s.len() as usize)
        };

        let ctime: Option<usize> = if kind == EntryKind::Dir {
            // A third of the round trips a listing makes, and only wanted when the
            // time is on screen or being sorted by.
            options.times.then(|| get_rctime(&path)).flatten()
        } else {
            stat.as_ref().map(|s| s.ctime() as usize)
        };

        let name_str = path.file_name().unwrap_or_default().to_string_lossy();
        let name = if kind == EntryKind::Dir {
            format!("{}/", name_str)
        } else {
            name_str.to_string()
        };

        let name_or_id = |id: u32| id_to_name(id).unwrap_or_else(|| format!("{}", id));

        let (user, group) = match stat.as_ref().filter(|_| options.owners) {
            Some(stat) => (Some(name_or_id(stat.uid())), Some(name_or_id(stat.gid()))),
            None => (None, None),
        };

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
    /// The name this field goes by in the interface and on the command line.
    pub const fn label(self) -> &'static str {
        match self {
            SortField::Name => "name",
            SortField::Size => "size",
            SortField::Rentries => "count",
            SortField::Owner => "owner",
            SortField::CTime => "time",
        }
    }

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
    pub fn new(cwd: Option<&PathBuf>, options: Options) -> Result<App, std::io::Error> {
        let cwd: PathBuf = if let Some(cwd) = cwd {
            cwd.clone()
        } else {
            std::env::current_dir()?
        };

        let dir_listing = DirListing::empty(options);
        let original_cwd = cwd.clone();
        let mut app = App {
            should_exit: false,
            cwd: PathBuf::new(),
            dir_listing,
            original_cwd,
            popup: None,
            show_owner: false,
            show_ctime: false,
            exact: false,
            hscroll: 0,
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
        // What the next read fetches follows the column, not what this listing
        // happened to be read with: leaving a directory with the owner hidden should
        // not make the next one pay for it.
        let options = self.needs();
        self.dir_listing = DirListing::from(&new, options)?;
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

    /// What a read of this directory would have to fetch: a column needs its value,
    /// and so does ordering by it, whether or not it is on screen.
    fn needs(&self) -> Options {
        let field = *self.dir_listing.sort_mode().field();
        Options {
            owners: self.show_owner || field == SortField::Owner,
            times: self.show_ctime || field == SortField::CTime,
            ..self.dir_listing.options
        }
    }

    /// Re-read when something has come to need what this listing never fetched.
    /// `options` records what the listing *has*, so one that already has it is left
    /// alone however many times a column is toggled.
    fn fetch_if_needed(&mut self) {
        let needs = self.needs();
        let has = self.dir_listing.options;
        if (needs.owners && !has.owners) || (needs.times && !has.times) {
            self.cd(&self.cwd.clone());
        }
    }

    pub fn toggle_owner(&mut self) {
        self.show_owner = !self.show_owner;
        self.fetch_if_needed();
    }

    pub fn toggle_ctime(&mut self) {
        self.show_ctime = !self.show_ctime;
        self.fetch_if_needed();
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
        );
        // Ordering by owner or by time is the other thing that needs a read, and the
        // sort mode is already in place, so the re-read lands in the right order.
        self.fetch_if_needed();
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
    pub fn from(path: &Path, options: Options) -> Result<DirListing, std::io::Error> {
        let path: PathBuf = path.canonicalize()?;
        let fs = get_fs(&path);

        let (entry_cwd, mut entries): (DirEntry, Vec<DirEntry>) = ls(&path, &options)?;

        // Don't trust dir sizes on non-ceph!
        if !fs.map(FSType::is_ceph).unwrap_or(false) {
            entries
                .iter_mut()
                .filter(|e| e.kind == EntryKind::Dir)
                .for_each(|e| {
                    e.size = None;
                });
        }
        sort(&mut entries, options.sort_mode, options.dirs_first);

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
            options,
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
        options: Options,
    ) -> DirListing {
        sort(&mut entries, options.sort_mode, options.dirs_first);

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
            options,
            fs: None,
        }
    }

    fn empty(options: Options) -> DirListing {
        DirListing {
            dotdot: None,
            entries: Vec::new(),
            state: ListState::default(),
            options,
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
        let entries_iter: Box<dyn Iterator<Item = &DirEntry>> =
            if self.options.sort_mode.is_reversed() {
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

        if self.options.sort_mode.is_reversed() {
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
        self.options.sort_mode
    }

    pub fn options(&self) -> Options {
        self.options
    }

    pub fn sort(&mut self, sort_mode: SortMode) {
        // Reversing normally needs no re-sort, but with dirs_first the stored order
        // depends on the direction, so only the plain case can short-circuit.
        if self.options.sort_mode.same_field(&sort_mode) && !self.options.dirs_first {
            self.options.sort_mode = sort_mode;
            return;
        }

        sort(&mut self.entries, sort_mode, self.options.dirs_first);

        self.options.sort_mode = sort_mode;
    }

    pub fn dirs_first(&self) -> bool {
        self.options.dirs_first
    }

    pub fn toggle_dirs_first(&mut self) {
        self.options.dirs_first = !self.options.dirs_first;
        sort(
            &mut self.entries,
            self.options.sort_mode,
            self.options.dirs_first,
        );
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

fn sort(entries: &mut [DirEntry], sort_mode: SortMode, dirs_first: bool) {
    // `entries` is stored ascending and reversed at read time, so this key has to
    // flip with the direction: under a reversed mode, directories have to be stored
    // *last* in order to be displayed first. Symlinks to directories group with the
    // files, as everywhere else (#12).
    let group = |e: &DirEntry| -> u8 {
        if !dirs_first {
            return 0;
        }
        u8::from((e.kind == EntryKind::Dir) == sort_mode.is_reversed())
    };

    let by_field = |a: &DirEntry, b: &DirEntry| match sort_mode.field() {
        // The name comparison below is the whole ordering for this field.
        SortField::Name => Ordering::Equal,
        SortField::Size => a.size.cmp(&b.size).then(a.rentries.cmp(&b.rentries)),
        SortField::Rentries => a.rentries.cmp(&b.rentries).then(a.size.cmp(&b.size)),
        SortField::CTime => a.ctime.cmp(&b.ctime).then(a.size.cmp(&b.size)),
        SortField::Owner => a
            .user
            .cmp(&b.user)
            .then(a.group.cmp(&b.group))
            .then(a.size.cmp(&b.size)),
    };

    // Every comparison ends on the name, which is unique within a directory. That
    // makes the order total, so the listing doesn't depend on readdir order.
    entries.sort_by(|a, b| {
        group(a)
            .cmp(&group(b))
            .then_with(|| by_field(a, b))
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn ls(path: &PathBuf, options: &Options) -> Result<(DirEntry, Vec<DirEntry>), std::io::Error> {
    // The cwd is only here for its totals, so it needs neither a stat nor a time.
    let totals = Options {
        owners: false,
        times: false,
        ..*options
    };
    let entry_cwd = DirEntry::from(PathBuf::from(path), EntryKind::Dir, None, &totals);
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

        // readdir already reported the type, so file_type() only falls back to a
        // stat of its own where the filesystem returned DT_UNKNOWN.
        let file_type = entry.file_type()?;
        let kind = if file_type.is_dir() {
            EntryKind::Dir
        } else if file_type.is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };

        // A file's size and time are the stat's alone; a directory needs one only to
        // say who owns it.
        let stat = if kind == EntryKind::Dir && !options.owners {
            None
        } else {
            Some(entry.metadata()?)
        };

        entries.push(DirEntry::from(path, kind, stat, options));
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

    fn file(name: &str, size: usize) -> DirEntry {
        DirEntry {
            kind: EntryKind::File,
            ..entry(name, size)
        }
    }

    /// Two directories and two files, interleaved by size so that grouping them is
    /// visible in either direction.
    fn mixed(sort_mode: SortMode, dirs_first: bool) -> DirListing {
        DirListing::from_entries(
            vec![
                entry("d_big/", 300),
                file("f_huge", 400),
                entry("d_small/", 100),
                file("f_mid", 200),
            ],
            true,
            Options {
                sort_mode,
                dirs_first,
                ..Options::default()
            },
        )
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
            Options::sorted(sort_mode),
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
        let mut listing = DirListing::from_entries(
            vec![],
            true,
            Options {
                sort_mode: DEFAULT_SORT_MODE,
                dirs_first: false,
                ..Options::default()
            },
        );
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
        let mut listing = DirListing::from_entries(
            vec![],
            true,
            Options {
                sort_mode: DEFAULT_SORT_MODE,
                dirs_first: false,
                ..Options::default()
            },
        );
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
        let mut app = App::new(Some(&PathBuf::from(".")), Options::default()).unwrap();
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

    /// Symlinks are marked where a person reads the name, not in the machine stream:
    /// a filename may contain `@`, so it cannot signal anything there.
    #[test]
    fn symlinks_are_marked_for_display_only() {
        let link = DirEntry {
            kind: EntryKind::Symlink,
            ..entry("target", 10)
        };
        assert_eq!(link.display_name(), "target@");
        assert_eq!(link.name, "target");

        // A file whose name ends in @ is left alone, and a directory keeps the `/`
        // that `from` already gave it.
        let odd = DirEntry {
            kind: EntryKind::File,
            ..entry("notes@", 10)
        };
        assert_eq!(odd.display_name(), "notes@");

        let dir = entry("data/", 10);
        assert_eq!(dir.kind, EntryKind::Dir);
        assert_eq!(dir.display_name(), "data/");
    }

    /// The owner costs a stat that nothing else needs, so it is read only when asked
    /// for -- which is also what lets a directory be listed without a stat at all.
    #[test]
    fn the_owner_is_read_only_when_asked_for() {
        let mut app = App::new(Some(&PathBuf::from(".")), Options::default()).unwrap();
        assert!(!app.dir_listing.options().owners);
        assert!(
            app.dir_listing
                .iter_entries_sorted()
                .all(|e| e.user.is_none()),
            "an owner was read without being asked for"
        );

        app.toggle_owner();
        assert!(app.show_owner);
        assert!(app.dir_listing.options().owners);
        assert!(
            app.dir_listing
                .iter_entries_sorted()
                .all(|e| e.user.is_some()),
            "asking for the owner did not read it"
        );

        // A file's size comes from the same stat either way.
        assert!(
            app.dir_listing
                .iter_entries_sorted()
                .filter(|e| e.kind == EntryKind::File)
                .all(|e| e.size.is_some()),
            "a file lost its size"
        );

        // Hiding it keeps what was already read -- see the toggling test -- and it is
        // the *next* directory that stops paying, which the following test covers.
        app.toggle_owner();
        assert!(!app.show_owner);
        assert!(
            app.dir_listing.options().owners,
            "hiding the column threw away what had been read"
        );
    }

    /// A directory is listed without a stat, so everything it shows has to come from
    /// somewhere else: the xattrs, and readdir for the kind.
    #[test]
    fn a_directory_needs_no_stat() {
        let app = App::new(Some(&PathBuf::from(".")), Options::default()).unwrap();

        let dirs: Vec<&DirEntry> = app
            .dir_listing
            .iter_entries_sorted()
            .filter(|e| e.kind == EntryKind::Dir)
            .collect();
        assert!(!dirs.is_empty(), "no directories to check");

        for dir in dirs {
            assert!(
                dir.name.ends_with('/'),
                "{} is not marked a directory",
                dir.name
            );
            assert!(dir.user.is_none(), "{} was stat'd anyway", dir.name);
        }
    }

    /// Once read, the owner stays read. Hiding the column and showing it again must
    /// not pay for it twice, however many times it is toggled.
    #[test]
    fn showing_the_owner_again_does_not_re_read_it() {
        let mut app = App::new(Some(&PathBuf::from(".")), Options::default()).unwrap();
        app.toggle_owner();
        assert!(
            app.dir_listing.options().owners,
            "the first show did not read"
        );

        // A value only a re-read would overwrite.
        app.dir_listing.entries[0].user = Some("sentinel".to_string());

        for _ in 0..3 {
            app.toggle_owner();
            app.toggle_owner();
        }

        assert!(app.show_owner);
        assert_eq!(
            app.dir_listing.entries[0].user.as_deref(),
            Some("sentinel"),
            "the directory was read again"
        );
    }

    /// Leaving with the column hidden should not make the next directory pay for the
    /// owner, and leaving with it shown should.
    #[test]
    fn the_next_directory_follows_the_column() {
        let mut app = App::new(Some(&PathBuf::from(".")), Options::default()).unwrap();

        app.cd(&PathBuf::from("src"));
        assert!(
            !app.dir_listing.options().owners,
            "read the owner with the column hidden"
        );

        app.toggle_owner();
        app.cd(&PathBuf::from(".."));
        assert!(
            app.dir_listing.options().owners,
            "did not read the owner with the column shown"
        );
        assert!(
            app.dir_listing
                .iter_entries_sorted()
                .all(|e| e.user.is_some()),
            "the owner column is shown but empty"
        );
    }

    /// Ordering by owner needs the owner read, whether or not the column is shown.
    /// Without this the sort silently had nothing to compare.
    #[test]
    fn sorting_by_owner_reads_the_owner() {
        let mut app = App::new(Some(&PathBuf::from(".")), Options::default()).unwrap();
        assert!(!app.dir_listing.options().owners);

        app.sort_or_reverse(SortField::Owner.default_mode());
        assert!(
            app.dir_listing.options().owners,
            "sorted by an owner that was never read"
        );
        assert!(
            app.dir_listing
                .iter_entries_sorted()
                .all(|e| e.user.is_some()),
            "the sort had nothing to compare"
        );
        assert_eq!(*app.dir_listing.sort_mode().field(), SortField::Owner);

        // And it stays read while that ordering is in effect, even with the column
        // hidden: the next directory has to sort by it too.
        assert!(!app.show_owner);
        app.cd(&PathBuf::from("src"));
        assert!(
            app.dir_listing.options().owners,
            "the next directory sorted by an owner it never read"
        );
    }

    /// A directory's recursive time is a third of the round trips, so it is read only
    /// when wanted -- and, like the owner, kept once read. Asserted on the intent
    /// rather than the values, since off Ceph there is no rctime to find.
    #[test]
    fn showing_the_time_reads_it_once() {
        let mut app = App::new(Some(&PathBuf::from(".")), Options::default()).unwrap();
        assert!(!app.dir_listing.options().times);

        app.toggle_ctime();
        assert!(app.show_ctime);
        assert!(
            app.dir_listing.options().times,
            "showing the time did not read it"
        );

        // A value only a re-read would overwrite.
        app.dir_listing.entries[0].ctime = Some(12_345);
        for _ in 0..3 {
            app.toggle_ctime();
            app.toggle_ctime();
        }
        assert_eq!(
            app.dir_listing.entries[0].ctime,
            Some(12_345),
            "the directory was read again"
        );
    }

    #[test]
    fn sorting_by_time_reads_it() {
        let mut app = App::new(Some(&PathBuf::from(".")), Options::default()).unwrap();

        app.sort_or_reverse(SortField::CTime.default_mode());
        assert!(
            app.dir_listing.options().times,
            "sorted by a time that was never read"
        );
        assert!(!app.show_ctime, "the column was shown as a side effect");
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
        let app = App::new(Some(&PathBuf::from(".")), Options::sorted(mode)).unwrap();
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

    #[test]
    fn dirs_first_groups_directories_ahead_of_files() {
        let interleaved = mixed(SortMode::Reversed(SortField::Size), false);
        assert_eq!(
            displayed(&interleaved),
            ["..", "f_huge", "d_big/", "f_mid", "d_small/"]
        );
        assert_get_matches_display(&interleaved);

        // Largest first within each group.
        let grouped = mixed(SortMode::Reversed(SortField::Size), true);
        assert_eq!(
            displayed(&grouped),
            ["..", "d_big/", "d_small/", "f_huge", "f_mid"]
        );
        assert_get_matches_display(&grouped);

        // Smallest first within each group, directories still on top.
        let ascending = mixed(SortMode::Normal(SortField::Size), true);
        assert_eq!(
            displayed(&ascending),
            ["..", "d_small/", "d_big/", "f_mid", "f_huge"]
        );
        assert_get_matches_display(&ascending);
    }

    /// The grouping key flips with the direction, so a name sort has to group too.
    #[test]
    fn dirs_first_groups_under_any_field() {
        let ascending = mixed(SortMode::Normal(SortField::Name), true);
        assert_eq!(
            displayed(&ascending),
            ["..", "d_big/", "d_small/", "f_huge", "f_mid"]
        );
        assert_get_matches_display(&ascending);

        let descending = mixed(SortMode::Reversed(SortField::Name), true);
        assert_eq!(
            displayed(&descending),
            ["..", "d_small/", "d_big/", "f_mid", "f_huge"]
        );
        assert_get_matches_display(&descending);
    }

    /// Reversing normally skips the re-sort, but with dirs_first the stored order
    /// depends on the direction, so skipping it would put the files on top.
    #[test]
    fn dirs_first_survives_a_direction_flip() {
        let mut listing = mixed(SortMode::Reversed(SortField::Size), true);
        assert_eq!(
            displayed(&listing),
            ["..", "d_big/", "d_small/", "f_huge", "f_mid"]
        );

        listing.sort(SortMode::Normal(SortField::Size));
        assert_eq!(
            displayed(&listing),
            ["..", "d_small/", "d_big/", "f_mid", "f_huge"],
            "the direction-only short-circuit skipped the regrouping"
        );
        assert_get_matches_display(&listing);
    }

    #[test]
    fn toggling_dirs_first_regroups() {
        let mut listing = mixed(SortMode::Reversed(SortField::Size), false);
        let interleaved = displayed(&listing);

        listing.toggle_dirs_first();
        assert_eq!(
            displayed(&listing),
            ["..", "d_big/", "d_small/", "f_huge", "f_mid"]
        );
        assert_get_matches_display(&listing);

        listing.toggle_dirs_first();
        assert_eq!(displayed(&listing), interleaved);
        assert_get_matches_display(&listing);
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
            let listing = DirListing::from_entries(
                tied(),
                false,
                Options {
                    sort_mode: SortMode::Normal(field),
                    dirs_first: false,
                    ..Options::default()
                },
            );
            assert_eq!(
                displayed(&listing),
                ["a", "b", "c"],
                "{:?} is unstable",
                field
            );
        }
    }
}
