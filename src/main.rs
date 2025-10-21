use app::Message;
use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::backend::Backend;
use std::path::PathBuf;

mod app;
mod fs;
mod navigation;
mod popup;
mod ui;

use crate::{app::App, ui::ui};

const DEFAULT_DIR: Option<&str> = option_env!("CEPHDU_DEFAULT_DIR");

/// Display ceph space and file count (inode) usage in an interactive terminal
#[derive(Parser)]
#[clap(after_help = r#"
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
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let path_was_explicit = args.path.is_some();

    let path: PathBuf = args.path.unwrap_or_else(default_dir);

    let mut app = App::new(Some(&path)).unwrap_or_else(|e| {
        let mut app = App::new(Some(&PathBuf::from("."))).unwrap_or_else(|_| {
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
