use crossterm::event::KeyCode;

use std::path::PathBuf;

use crate::app;
use crate::app::App;
use crate::ui::POPUP_TEXT_HEIGHT;

static PAGE_BY: usize = 10;

pub const HELP: &[[&str; 2]] = &[
        ["q, Esc", "Quit"],
        ["Down, j", "Move cursor down"],
        ["Up, k", "Move cursor up"],
        ["Page Down", "Jump cursor down"],
        ["Page Up", "Jump cursor up"],
        ["Enter", "Open directory"],
        ["Backspace, h", "Go to parent directory"],
        ["n", "Sort by name"],
        ["s", "Sort by size"],
        ["c, C", "Sort by file count"],
        ["U", "Sort by owner"],
        ["u", "Toggle show owner"],
        ["?", "Show this help message"],
        ["Home, g", "Select first entry"],
        ["End, G", "Select last entry"],
    ];

impl App {
    pub fn handle_key(&mut self, key: KeyCode) {
        if self.popup.is_some() {
            match key {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')  => {
                    self.popup(None, None);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(popup) = &mut self.popup {
                        popup.scroll_by(1);
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(popup) = &mut self.popup {
                        popup.scroll_by(-1);
                    }
                }
                KeyCode::PageUp => {
                    if let Some(popup) = &mut self.popup {
                        popup.scroll_by(-(PAGE_BY as isize));
                    }
                }
                KeyCode::PageDown => {
                    if let Some(popup) = &mut self.popup {
                        popup.scroll_by(PAGE_BY as isize);
                    }
                }
                KeyCode::Home | KeyCode::Char('g') => {
                    if let Some(popup) = &mut self.popup {
                        popup.scroll_to(0);
                    }
                }
                KeyCode::End | KeyCode::Char('G') => {
                    if let Some(popup) = &mut self.popup {
                        popup.scroll_to(POPUP_TEXT_HEIGHT);
                    }
                }
                _ => {}
            }
            return;
        }

        match key {
            KeyCode::Enter => {
                if let Some(selected) = self.dir_listing.state.selected() {
                    let entry = self.dir_listing.get(selected);
                    if entry.kind == app::EntryKind::Dir {
                        self.cd(&PathBuf::from(&entry.name));
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.dir_listing.state.select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.dir_listing.state.select_previous();
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.dir_listing.state.select_first();
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.dir_listing.state.select_last();
            }
            KeyCode::PageUp => {
                let state = &mut self.dir_listing.state;
                if let Some(idx) = state.selected() {
                    let new_idx = idx.saturating_sub(PAGE_BY);
                    state.select(Some(new_idx));
                }
            }
            KeyCode::PageDown => {
                let state = &mut self.dir_listing.state;
                if let Some(idx) = state.selected() {
                    let new_idx = idx.saturating_add(PAGE_BY);
                    state.select(Some(new_idx));
                }
            }
            KeyCode::Backspace | KeyCode::Char('h') => {
                self.cd(&"..".into());
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.should_exit = true;
            }
            KeyCode::Char('n') => self.sort_or_reverse(app::SortMode::Normal(app::SortField::Name)),
            KeyCode::Char('s') => self.sort_or_reverse(app::SortMode::Reversed(app::SortField::Size)),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.sort_or_reverse(app::SortMode::Reversed(app::SortField::Rentries))
            }
            KeyCode::Char('U') => {
                self.sort_or_reverse(app::SortMode::Normal(app::SortField::Owner))
            }
            KeyCode::Char(' ') => {
                self.cd(&self.original_cwd.clone());
            }
            KeyCode::Char('u') => {
                self.show_owner = !self.show_owner;
            }
            KeyCode::Char('?') => {
                self.help();
            }
            _ => {}
        }
    }
}
