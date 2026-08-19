use app::Message;
use chrono::{Datelike, Local};
use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::backend::Backend;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

mod app;
mod flat;
mod format;
mod fs;
mod navigation;
mod popup;
mod ui;

use crate::{
    app::{App, SortField, SortMode},
    flat::Format,
    ui::ui,
};

const DEFAULT_DIR: Option<&str> = option_env!("CEPHDU_DEFAULT_DIR");

/// Display ceph space and file count (inode) usage in an interactive terminal
#[derive(Parser)]
#[clap(after_help = r#"
The interactive interface is used when stdout is a terminal. Otherwise a flat
listing is printed, as if --parseable had been given.

Flat listings have one row per entry: size, file count, modified time, user,
group, name, with '-' for values the filesystem does not provide. --flat writes
them with units for reading; --parseable writes raw tab-separated values in a
format that does not vary with the terminal.

Listings are sorted largest first unless one of the sort flags is given. Those
mirror the interface's sort keys, and apply to both output modes. -r reverses
whichever order is in effect, so 'cephdu -r' reads smallest first, and -d groups
directories ahead of files whatever the field and direction.

Note the following differences from 'ls -l':
  * The time shown is recursive for directories
  * The time shown is the time at which a file's contents *or* its metadata
    have been modified (ctime). This is subtly different from 'ls -l', where
    the timestamp only changes if the contents are modified (mtime)
  * The size shown is recursive for directories (may also be true for
    'ls -l' depending on ceph deployment)
"#)]
struct Cli {
    /// Path to the directory to display
    path: Option<std::path::PathBuf>,

    /// Print a flat text listing, with units, instead of the interactive interface
    #[arg(short, long)]
    flat: bool,

    /// Print a flat text listing of raw values, for parsing
    #[arg(short, long)]
    parseable: bool,

    /// Use the interactive interface even if stdout is not a terminal
    #[arg(long, conflicts_with_all = ["flat", "parseable"])]
    tui: bool,

    #[command(flatten)]
    sort: SortFlags,

    /// Reverse the sort order
    #[arg(short, long)]
    reverse: bool,

    /// List directories before files
    #[arg(short, long)]
    dirs_first: bool,
}

/// The startup sort order, mirroring the interface's sort keys. Each field starts
/// in the direction `SortField::default_mode` gives it, so `-s` reads largest
/// first, and the interface's usual keys still reverse it from there.
#[derive(clap::Args)]
#[group(multiple = false)]
struct SortFlags {
    /// Sort by name
    #[arg(short, long)]
    name: bool,

    /// Sort by size
    #[arg(short, long)]
    size: bool,

    /// Sort by file count
    #[arg(short, long)]
    count: bool,

    /// Sort by owner
    #[arg(short = 'u', long)]
    owner: bool,

    /// Sort by modification time
    #[arg(short, long)]
    time: bool,
}

impl SortFlags {
    fn mode(&self) -> SortMode {
        let field = if self.name {
            SortField::Name
        } else if self.size {
            SortField::Size
        } else if self.count {
            SortField::Rentries
        } else if self.owner {
            SortField::Owner
        } else if self.time {
            SortField::CTime
        } else {
            return app::DEFAULT_SORT_MODE;
        };
        field.default_mode()
    }
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let path_was_explicit = args.path.is_some();

    let path: PathBuf = args.path.clone().unwrap_or_else(default_dir);
    let sort_mode = if args.reverse {
        args.sort.mode().as_reversed()
    } else {
        args.sort.mode()
    };

    // The interactive interface draws to stdout, so it can only work on a terminal.
    // Whatever is reading a pipe or a file is more often a program than a person,
    // hence the parseable format there. --tui is honored anyway, for pty-wrapping
    // tools that defeat the detection.
    let format = if args.parseable {
        Some(Format::Parseable)
    } else if args.flat {
        Some(Format::Human)
    } else if !args.tui && !std::io::stdout().is_terminal() {
        Some(Format::Parseable)
    } else {
        None
    };
    if let Some(format) = format {
        return run_flat(&path, &format, sort_mode, args.dirs_first);
    }

    let mut app = App::new(Some(&path), sort_mode, args.dirs_first).unwrap_or_else(|e| {
        let mut app = App::new(Some(&PathBuf::from(".")), sort_mode, args.dirs_first)
            .unwrap_or_else(|_| {
                eprintln!("Error opening {:?}: {}", path, e);
                std::process::exit(1);
            });

        if path_was_explicit {
            app.message(Some(Message {
                text: format!("Error opening {:?}: {}", path, e),
                kind: app::MessageKind::Warning,
            }));
        }
        app
    });

    color_eyre::install()?;
    let mut terminal = ratatui::init();

    run_app(&mut terminal, &mut app)?;

    // cleanup terminal
    ratatui::restore();

    Ok(())
}

/// Print a flat listing of `path`. Unlike the interactive interface, this doesn't
/// fall back to the current directory on failure: a script needs to see the error
/// rather than a listing of somewhere else.
fn run_flat(path: &Path, format: &Format, sort_mode: SortMode, dirs_first: bool) -> Result<()> {
    let listing = app::DirListing::from(path, sort_mode, dirs_first).unwrap_or_else(|e| {
        eprintln!("Error opening {:?}: {}", path, e);
        std::process::exit(1);
    });

    if !listing.is_ceph() {
        eprintln!(
            "Warning: {:?} is not a Ceph directory; directory sizes and counts are unavailable",
            path
        );
    }

    let current_year = Local::now().year() as isize;

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let res =
        flat::write_listing(&listing, format, current_year, &mut out).and_then(|()| out.flush());

    match res {
        // Closing the pipe early, as in 'cephdu --flat | head', is not a failure.
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    while !app.should_exit {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Release {
                continue;
            }
            app.handle_key(key);
        }
    }
    Ok(())
}

/// Returns the cwd if it is a ceph dir.
/// If not, returns DEFAULT_DIR if set.
/// If not, the cwd is returned.
/// Instances of $USER in DEFAULT_DIR are replaced with the current username.
fn default_dir() -> PathBuf {
    let cwd = PathBuf::from(".");
    if DEFAULT_DIR.is_none() {
        // short-circuit testing if cwd is ceph
        return cwd;
    }

    if fs::get_fs(&cwd).map(fs::FSType::is_ceph).unwrap_or(false) {
        return cwd;
    }

    DEFAULT_DIR
        .and_then(|dir| {
            if dir.contains("$USER") {
                match std::env::var("USER") {
                    Ok(username) => Some(PathBuf::from(dir.replace("$USER", &username))),
                    Err(_) => None,
                }
            } else {
                Some(PathBuf::from(dir))
            }
        })
        .unwrap_or(PathBuf::from("."))
}
