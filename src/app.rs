use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::{fs, os::unix::fs::MetadataExt};

use ratatui::widgets::ListState;

use crate::fs::{FSType, get_fs, get_rentries, id_to_name};
use crate::navigation;
use crate::popup::Popup;

const DEFAULT_SORT_MODE: SortMode = SortMode::Reversed(SortField::Size);

pub struct App {
    pub should_exit: bool,
    pub cwd: PathBuf,
    pub dir_listing: DirListing,
    pub original_cwd: PathBuf,
    pub popup: Option<Popup>,
    pub show_owner: bool,
    pub message: Option<Message>,
}

pub struct DirListing {
    dotdot: Option<DirEntry>,
    entries: Vec<DirEntry>,
    state: ListState,
    sort_mode: SortMode,
    pub stats: ListingStats,
    pub fs: Option<FSType>,
}

pub struct ListingStats {
    pub max_rentries: usize,
    pub total_rentries: usize,
    pub max_size: usize,
    pub total_size: usize,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<usize>,
    pub rentries: Option<usize>,
    pub user: Option<String>,
    pub group: Option<String>,
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
    pub fn new(cwd: Option<&PathBuf>) -> Result<App, std::io::Error> {
        let cwd: PathBuf = if let Some(cwd) = cwd {
            cwd.clone()
        } else {
            std::env::current_dir()?
        };

        let dir_listing = DirListing::default();
        let original_cwd = cwd.clone();
        let mut app = App {
            should_exit: false,
            cwd: PathBuf::new(),
            dir_listing,
            original_cwd,
            popup: None,
            show_owner: false,
            message: None,
        };
        app.try_cd(&cwd)?;
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
}

impl DirListing {
    fn from(path: &Path, sort_mode: SortMode) -> Result<DirListing, std::io::Error> {
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

        let has_parent = path != PathBuf::from("/");
        let dotdot = has_parent.then(|| DirEntry {
            name: "..".to_string(),
            kind: EntryKind::Dir,
            size: None,
            rentries: None,
            user: None,
            group: None,
        });

        let (max_rentries, max_size) = entries.iter().fold((0, 0), |(max_r, max_s), entry| {
            let r = entry.rentries.unwrap_or(0);
            let s = entry.size.unwrap_or(0);
            (max_r.max(r), max_s.max(s))
        });
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

    fn default() -> DirListing {
        DirListing {
            dotdot: None,
            entries: Vec::new(),
            state: ListState::default(),
            sort_mode: DEFAULT_SORT_MODE,
            stats: ListingStats {
                max_rentries: 0,
                total_rentries: 0,
                max_size: 0,
                total_size: 0,
            },
            fs: None,
        }
    }

    pub fn iter_entries(&self) -> impl Iterator<Item = &DirEntry> {
        // Display ".." first if we have it, then the rest of the entries,
        // maybe in reverse order.

        let dotdot = self.dotdot.iter();

        let entries_iter: Box<dyn Iterator<Item = &DirEntry>> = if self.sort_mode.is_reversed() {
            Box::new(self.entries.iter().rev())
        } else {
            Box::new(self.entries.iter())
        };

        dotdot.chain(entries_iter)
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
        // Normally we would use select_next(), but that has a weird interaction
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

    pub fn select_first(&mut self) {
        self.state.select(Some(0));
    }

    pub fn select_last(&mut self) {
        let len = self.len();
        if len > 0 {
            self.state.select(Some(len - 1));
        }
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

fn sort(entries: &mut [DirEntry], sort_mode: SortMode) {
    match sort_mode.field() {
        SortField::Name => entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.size.cmp(&b.size))),
        SortField::Size => {
            entries.sort_by(|a, b| a.size.cmp(&b.size).then(a.rentries.cmp(&b.rentries)))
        }
        SortField::Rentries => {
            entries.sort_by(|a, b| a.rentries.cmp(&b.rentries).then(a.size.cmp(&b.size)))
        }
        SortField::Owner => entries.sort_by(|a, b| {
            a.user
                .cmp(&b.user)
                .then(a.group.cmp(&b.group))
                .then(a.size.cmp(&b.size))
        }),
    }
}

fn ls(path: &PathBuf) -> Result<(DirEntry, Vec<DirEntry>), std::io::Error> {
    let get_dent = |path: PathBuf, stat: Metadata| -> Result<DirEntry, std::io::Error> {
        let kind = if stat.is_dir() {
            EntryKind::Dir
        } else if stat.is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };

        let name_str = path.file_name().unwrap_or_default().to_string_lossy();
        let name = if kind == EntryKind::Dir {
            format!("{}/", name_str)
        } else {
            name_str.to_string()
        };

        let name_or_id = |id: u32| id_to_name(id).unwrap_or_else(|| format!("{}", id));

        let size = Some(stat.len() as usize);

        let rentries: Option<usize> = if kind == EntryKind::Dir {
            // rentries seems to include the self-count, which is confusing when there are
            // only N files but N+1 rentries.
            get_rentries(&path).map(|r| r.saturating_sub(1))
        } else {
            None
        };
        let user = Some(name_or_id(stat.uid()));
        let group = Some(name_or_id(stat.gid()));

        Ok(DirEntry {
            name,
            kind,
            size,
            rentries,
            user,
            group,
        })
    };

    let entry_cwd = get_dent(PathBuf::from(path), fs::metadata(path)?)?;
    let entries_result: Result<Vec<_>, std::io::Error> = fs::read_dir(path)?
        .map(|entry_result| -> Result<DirEntry, std::io::Error> {
            let entry = entry_result?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            get_dent(path, metadata)
        })
        .collect();
    let entries = entries_result?;

    Ok((entry_cwd, entries))
}
