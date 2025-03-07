use clap::Parser;
use color_eyre::Result;
use crossterm::event::KeyCode;
use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::backend::Backend;
use std::path::PathBuf;

mod app;
mod ui;
mod ceph;

use crate::{app::App, ui::ui};

static PAGE_BY: usize = 10;

/// Display ceph space and file count (inode) usage in an interactive terminal
#[derive(Parser)]
struct Cli {
    /// Path to the directory to display
    path: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let args = Cli::parse();
    let path: PathBuf = args.path.clone().unwrap_or(".".into());

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
            handle_key(key.code, app);
        }
    }
    Ok(())
}

fn handle_key(key: KeyCode, app: &mut App) {
    match key {
        KeyCode::Enter | KeyCode::Right => {
            if app.popup.is_some() {
                app.popup(None);
            } else if let Some(selected) = app.dir_listing.state.selected() {
                let entry = app.dir_listing.get(selected);
                if entry.kind == app::EntryKind::Dir {
                    app.cd(&PathBuf::from(&entry.name));
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.dir_listing.state.select_next();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.dir_listing.state.select_previous();
        }
        KeyCode::PageUp => {
            let state = &mut app.dir_listing.state;
            if let Some(idx) = state.selected() {
                let new_idx = idx.saturating_sub(PAGE_BY);
                state.select(Some(new_idx));
            }
        }
        KeyCode::PageDown => {
            let state = &mut app.dir_listing.state;
            if let Some(idx) = state.selected() {
                let new_idx = idx.saturating_add(PAGE_BY);
                state.select(Some(new_idx));
            }
        }
        KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
            app.cd(&"..".into());
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            if app.popup.is_some() {
                app.popup(None);
            } else {
                app.should_exit = true;
            }
        }
        KeyCode::Char('n') => sort_or_reverse(app::SortMode::Normal(app::SortField::Name), app),
        KeyCode::Char('s') => sort_or_reverse(app::SortMode::Reversed(app::SortField::Size), app),
        KeyCode::Char('C') => {
            sort_or_reverse(app::SortMode::Reversed(app::SortField::Rentries), app)
        }
        KeyCode::Char(' ') => {
            app.cd(&app.original_cwd.clone());
        }
        _ => {}
    }
}

fn sort_or_reverse(sort_mode: app::SortMode, app: &mut App) {
    app.dir_listing.sort(
        if sort_mode.field() == app.dir_listing.sort_mode().field() {
            app.dir_listing.sort_mode().to_reversed()
        } else {
            sort_mode
        },
    )
}
