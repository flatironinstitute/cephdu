use std::fs;
use std::path::PathBuf;

use ratatui::widgets::ListState;

use crate::ceph::{get_fs, get_rentries, FSType};

const DEFAULT_SORT_MODE: SortMode = SortMode::Reversed(SortField::Size);

pub struct App {
    pub should_exit: bool,
    pub cwd: PathBuf,
    pub dir_listing: DirListing,
    pub original_cwd: PathBuf,
    pub popup: Option<Popup>,
}

pub struct DirListing {
    dotdot: Option<DirEntry>,
    entries: Vec<DirEntry>,
    pub state: ListState,
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

    pub fn to_reversed(&self) -> SortMode {
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
pub struct Popup {
    pub title: String,
    pub text: String,
}

impl App {
    pub fn new(cwd: Option<&PathBuf>) -> Result<App, std::io::Error> {
        let cwd: PathBuf = if let Some(cwd) = cwd {
            cwd.clone()
        } else {
            std::env::current_dir()?
        }
        .canonicalize()?;

        let dir_listing = DirListing::from(&cwd, DEFAULT_SORT_MODE)?;
        let original_cwd = cwd.clone();
        Ok(App {
            should_exit: false,
            cwd: cwd,
            dir_listing: dir_listing,
            original_cwd: original_cwd,
            popup: None,
        })
    }

    pub fn cd(&mut self, path: &PathBuf) {
        let res = self.try_cd(path);
        if let Err(e) = res {
            self.popup(Some(format!("Error changing directory: {}", e)));
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
        Ok(())
    }

    pub fn popup(&mut self, text: Option<String>) {
        self.popup = text.map(|text| Popup {
            title: "Error".to_string(),
            text,
        });
    }
}

impl DirListing {
    fn from(path: &PathBuf, sort_mode: SortMode) -> Result<DirListing, std::io::Error> {
        let path: PathBuf = path.canonicalize()?;
        let fs = get_fs(&path);

        let mut entries: Vec<DirEntry> = ls(&path)?;
        if !fs.map(|f| f.is_ceph()).unwrap_or(false) {
            entries.iter_mut().filter(|e| e.kind == EntryKind::Dir).for_each(|e| {
                e.size = None;
            });
        }
        sort(&mut entries, sort_mode);

        let has_parent = path != PathBuf::from("/");
        let dotdot = has_parent.then(|| DirEntry {
            name: "..".to_string(),
            kind: EntryKind::Dir,
            size: None, // TODO
            rentries: None,
        });

        let (max_rentries, total_rentries, max_size, total_size) = entries.iter().fold(
            (0, 0, 0, 0),
            |(max_r, total_r, max_s, total_s), entry| {
                let r = entry.rentries.unwrap_or(0);
                let s = entry.size.unwrap_or(0);
                (max_r.max(r), total_r + r, max_s.max(s), total_s + s)
            },
        );
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
            fs: fs,
        })
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
        self.fs.map_or(false, |fs| fs.is_ceph())
    }
}

fn sort(entries: &mut Vec<DirEntry>, sort_mode: SortMode) {
    match sort_mode.field() {
        SortField::Name => entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.size.cmp(&b.size))),
        SortField::Size => {
            entries.sort_by(|a, b| a.size.cmp(&b.size).then(a.rentries.cmp(&b.rentries)))
        }
        SortField::Rentries => {
            entries.sort_by(|a, b| a.rentries.cmp(&b.rentries).then(a.size.cmp(&b.size)))
        }
    }
}

fn ls(path: &PathBuf) -> Result<Vec<DirEntry>, std::io::Error> {
    let entries: Result<Vec<_>, std::io::Error> = fs::read_dir(path)?
        .map(|res| -> Result<DirEntry, std::io::Error> {
            let entry = res?;

            let stat = entry.metadata()?;
            let kind = if stat.is_dir() {
                EntryKind::Dir
            } else if stat.is_symlink() {
                EntryKind::Symlink
            } else {
                EntryKind::File
            };

            let name_str = entry
                .file_name()
                .to_str()
                .unwrap_or("[invalid utf8]")
                .to_string();
            let name = if kind == EntryKind::Dir {
                format!("{}/", name_str)
            } else {
                name_str
            };

            let size = Some(stat.len() as usize);
            let rentries = get_rentries(&entry.path());

            Ok(DirEntry {
                name,
                kind,
                size,
                rentries,
            })
        })
        .collect();

    entries
}
