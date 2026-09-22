use chrono::{Datelike, Local};
use clap::Parser;
use color_eyre::Result;
use crossterm::event::{Event, EventStream, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::Backend;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_stream::StreamExt;

mod app;
mod flat;
mod format;
mod fs;
mod navigation;
mod popup;
mod ui;

use crate::{
    app::{App, OnError, Options, SortField, SortMode},
    flat::Format,
    ui::ui,
};

const DEFAULT_DIR: Option<&str> = option_env!("CEPHDU_DEFAULT_DIR");

/// Display ceph space and file count (inode) usage in an interactive terminal
#[derive(Parser)]
#[clap(after_help = r#"
The interactive interface (terminal user interface, or TUI) is used when stdout is a
terminal. Otherwise, --parseable is enabled by default, so that the output can be processed
through pipes and redirects.

Without a PATH, the interactive interface starts in the current directory if it is on
Ceph. Otherwise it starts in $CEPHDU_DEFAULT_DIR if that is set, or in a default baked
into the binary when it was built; a literal $USER in either is replaced with the current
username. Setting the variable to the empty string disables the baked-in default too.
With no default configured, the current directory is used regardless. Flat listings
always use the current directory, whatever is configured.

Parseable listings have one row per entry: size, file count, change time, user,
group, name. In flat mode, columns the filesystem does not provide or were not requested
are skipped. In parseable mode, they are rendered as -. --flat writes with units for reading;
--parseable writes raw tab-separated values.

Listings are sorted by bytes unless one of the sort flags is given. Those mirror the TUI's
sort keys and apply to both TUI and flat. -r reverses whichever order is in effect, so
'cephdu -r' reads smallest first. -d groups all directories before files.

-e prints sizes and counts in full rather than scaled to a unit. The parseable
format is always exact, so -e has no effect there.

-l (implies flat) reads the two things that cost extra syscalls: the owner and a
directory's recursive time. Without it a directory needs no stat at all -- its size
and count are xattrs and its kind comes from readdir -- and makes one fewer xattr
call. A file's time comes from the stat it needs anyway for its size, so it is always
shown, and the parseable format marks what was not read '-' so that its fields never
move.

Note the following differences from 'ls -l':
  * The time shown is the time at which a file's contents *or* its metadata
    have been changed (ctime). 'ls -l' shows the time at which the contents were
    modified (mtime).
  * The time shown for a directory is the recursive ctime (rctime), including the
    directory itself. New directories do not have an rctime.
  * The size shown is recursive for directories (may also be true for
    'ls -l' depending on ceph deployment).
"#)]
struct Cli {
    /// Path to the directory to display
    path: Option<std::path::PathBuf>,

    /// Print a flat text listing, with units, instead of the interactive interface
    #[arg(short, long)]
    flat: bool,

    /// Print a flat text listing of raw values for parsing
    #[arg(short, long)]
    parseable: bool,

    /// Use the interactive interface even if stdout is not a terminal
    #[arg(long, conflicts_with_all = ["flat", "parseable", "long"])]
    tui: bool,

    #[command(flatten)]
    sort: SortFlags,

    /// Reverse the sort order
    #[arg(short, long)]
    reverse: bool,

    /// List directories before files
    #[arg(short, long)]
    dirs_first: bool,

    /// Show sizes and counts in full instead of scaled to a unit
    #[arg(short, long)]
    exact: bool,

    /// Show the owner and directory times, which cost extra syscalls. Implies -f.
    #[arg(short, long)]
    long: bool,

    /// Open the interactive interface with the info panel shown
    #[arg(short, long, conflicts_with_all = ["flat", "parseable", "long"])]
    info: bool,

    // Metadata reads in flight at once. Deliberately hidden and absent from the
    // README: it changes speed, never output, and the flag may change or vanish.
    // Keep it out of any user-facing text.
    #[arg(short = 'j', hide = true, default_value_t = 1)]
    jobs: usize,
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

    /// Sort by change time
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
    } else if args.flat || args.long {
        Some(Format::Human { exact: args.exact })
    } else if !args.tui && !std::io::stdout().is_terminal() {
        Some(Format::Parseable)
    } else {
        None
    };

    // The configured default is a convenience for the interface; a flat listing is
    // more often a script's, and a script means the directory it ran from.
    let path: PathBuf = args.path.clone().unwrap_or_else(|| match format {
        Some(_) => PathBuf::from("."),
        None => default_dir(),
    });

    let options = Options {
        sort_mode,
        dirs_first: args.dirs_first,
        // Ordering by one of these needs it read, whether or not it is shown.
        owners: args.long || *sort_mode.field() == SortField::Owner,
        times: args.long || *sort_mode.field() == SortField::CTime,
        jobs: args.jobs.max(1),
    };

    if let Some(format) = format {
        return run_flat(&path, &format, options);
    }

    color_eyre::install()?;
    // The runtime multiplexes wake-ups -- key presses, listing results, the
    // progress tick -- while the filesystem work itself runs on plain threads that
    // App manages, so a single-threaded runtime is all the loop needs.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    let mut terminal = ratatui::init();

    let res = runtime.block_on(run_app(
        &mut terminal,
        &path,
        path_was_explicit,
        options,
        &args,
    ));

    // cleanup terminal
    ratatui::restore();

    res
}

/// Print a flat listing of `path`. Unlike the interactive interface, this doesn't
/// fall back to the current directory on failure: a script needs to see the error
/// rather than a listing of somewhere else.
fn run_flat(path: &Path, format: &Format, options: Options) -> Result<()> {
    let listing = app::DirListing::from(path, options).unwrap_or_else(|e| {
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

/// While a read is in flight, wake up this often anyway so the progress notice
/// stays current; there is no free-running tick otherwise.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    path: &Path,
    path_was_explicit: bool,
    options: Options,
    args: &Cli,
) -> Result<()> {
    let (mut app, mut listings) = App::new(options);
    app.exact = args.exact;
    if args.info {
        // Zeros until the first listing lands, which recomputes it.
        app.toggle_info();
    }
    // Purely for the frame drawn before the first listing lands, which replaces it
    // with the canonical path.
    app.cwd = path.to_path_buf();

    // The fallback App::new used to apply, now expressed as what to do when the
    // first listing fails: warn only when the user named the path themselves, and
    // don't fall back to where we already are.
    let on_error = if path == Path::new(".") {
        OnError::Message
    } else {
        OnError::Fallback {
            path: PathBuf::from("."),
            warn: path_was_explicit,
        }
    };
    app.start_listing(path.to_path_buf(), on_error, false);

    let mut events = EventStream::new();
    while !app.should_exit {
        terminal.draw(|f| ui(f, &mut app))?;

        tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) => {
                    if key.kind != KeyEventKind::Release {
                        app.handle_key(key);
                    }
                }
                // Resize and the like: fall through to redraw.
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
                None => break,
            },
            Some(msg) = listings.recv() => app.on_listing_msg(msg),
            _ = tokio::time::sleep(PROGRESS_TICK), if app.is_reading() => {}
        }
    }
    Ok(())
}

/// Where the interface starts when no PATH was given: the cwd if it is a ceph
/// dir, otherwise the configured default, otherwise the cwd anyway.
fn default_dir() -> PathBuf {
    let cwd = PathBuf::from(".");
    let var = std::env::var("CEPHDU_DEFAULT_DIR").ok();
    let username = std::env::var("USER").ok();
    let Some(dir) = configured_dir(var.as_deref(), DEFAULT_DIR, username.as_deref()) else {
        // Nothing to fall back to, so skip the statfs.
        return cwd;
    };

    if fs::get_fs(&cwd).map(fs::FSType::is_ceph).unwrap_or(false) {
        return cwd;
    }

    dir
}

/// Resolves the configured default directory, `var` being the environment
/// variable's value if it is set at all and `baked` the build-time one. A $USER
/// that cannot be expanded leaves no usable default.
fn configured_dir(
    var: Option<&str>,
    baked: Option<&str>,
    username: Option<&str>,
) -> Option<PathBuf> {
    // The variable wins over the baked-in value because it is set closer to the
    // machine: a site's modulefile against whoever built the binary. Set-but-empty
    // disables the baked-in value too, so a site can also switch the default off.
    let dir = match var {
        Some(dir) => (!dir.is_empty()).then_some(dir),
        None => baked,
    }?;

    if dir.contains("$USER") {
        Some(PathBuf::from(dir.replace("$USER", username?)))
    } else {
        Some(PathBuf::from(dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_wins_over_the_baked_in_default() {
        let dir = |s: &str| Some(PathBuf::from(s));

        assert_eq!(
            configured_dir(Some("/var"), Some("/baked"), None),
            dir("/var")
        );
        assert_eq!(configured_dir(None, Some("/baked"), None), dir("/baked"));
        assert_eq!(configured_dir(Some(""), Some("/baked"), None), None);
        assert_eq!(configured_dir(None, None, None), None);
    }

    #[test]
    fn a_configured_default_expands_user() {
        assert_eq!(
            configured_dir(Some("/ceph/users/$USER"), None, Some("ada")),
            Some(PathBuf::from("/ceph/users/ada"))
        );
        assert_eq!(
            configured_dir(None, Some("/baked/$USER"), Some("ada")),
            Some(PathBuf::from("/baked/ada"))
        );
        assert_eq!(configured_dir(Some("/ceph/users/$USER"), None, None), None);
    }
}
