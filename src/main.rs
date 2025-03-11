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

const CEPH_USER_DIR: &str = "/mnt/ceph/users";

/// Display ceph space and file count (inode) usage in an interactive terminal
#[derive(Parser)]
struct Cli {
    /// Path to the directory to display
    path: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let args = Cli::parse();
    let path: PathBuf = args.path.clone().unwrap_or_else(|| {
        let username = std::env::var("USER").unwrap_or_else(|_| String::from(""));
        PathBuf::from(CEPH_USER_DIR).join(&username)
    });

    let mut app = App::new(Some(&path)).unwrap_or_else(|e| {
        eprintln!("Error opening {:?}: {}", path, e);
        std::process::exit(1);
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
            app.handle_key(key.code);
        }
    }
    Ok(())
}
