use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use std::path::PathBuf;

use crate::app;
use crate::app::App;
use crate::ui::POPUP_TEXT_HEIGHT;

static PAGE_BY: usize = 10;
/// Columns per horizontal scroll step.
static SCROLL_BY: usize = 4;

pub const HELP: &[[&str; 2]] = &[
    ["q", "Quit"],
    ["Down, j", "Move cursor down"],
    ["Up, k", "Move cursor up"],
    ["Page Down", "Jump cursor down"],
    ["Left", "Scroll left"],
    ["Right", "Scroll right"],
    ["Page Up", "Jump cursor up"],
    ["Enter", "Open directory"],
    ["Backspace", "Go to parent directory"],
    ["n", "Sort by name"],
    ["s", "Sort by size"],
    ["c, C", "Sort by file count"],
    ["U", "Sort by owner"],
    ["u", "Toggle show owner (reads it if needed)"],
    ["T", "Sort by change time"],
    ["t", "Toggle show change time (reads it if needed)"],
    ["d", "Toggle listing directories first"],
    ["e", "Toggle exact sizes and counts"],
    ["?, h", "Show this help message"],
    ["Home, g", "Select first entry"],
    ["End, G", "Select last entry"],
    ["r, F5", "Refresh"],
    ["Esc, Ctrl-C", "Cancel reading a directory"],
    ["Space", "Go to original directory"],
];

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.popup.is_some() {
            match key.code {
                KeyCode::Esc
                | KeyCode::Enter
                | KeyCode::Char('q')
                | KeyCode::Char('?')
                | KeyCode::Char('h') => {
                    self.popup(None, None, None);
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

        match key.code {
            KeyCode::Enter => {
                if let Some(selected) = self.dir_listing.selected() {
                    let entry = self.dir_listing.get(selected);
                    if entry.kind == app::EntryKind::Dir {
                        self.cd(&PathBuf::from(&entry.name));
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.dir_listing.select_next(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.dir_listing.select_prev(1);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.dir_listing.select_first();
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.dir_listing.select_last();
            }
            KeyCode::PageUp => {
                self.dir_listing.select_prev(PAGE_BY);
            }
            KeyCode::PageDown => {
                self.dir_listing.select_next(PAGE_BY);
            }
            KeyCode::Left => {
                self.hscroll = self.hscroll.saturating_sub(SCROLL_BY);
            }
            KeyCode::Right => {
                self.hscroll = self.hscroll.saturating_add(SCROLL_BY);
            }
            KeyCode::Backspace => {
                self.cd(&"..".into());
            }
            KeyCode::Char('q') => {
                self.should_exit = true;
            }
            // A cancel key must not double as a quit key: an extra press, or one
            // landing just after the read finishes, would exit unintended. Hence
            // Esc cancels and only q quits.
            KeyCode::Esc => {
                self.cancel_listing();
            }
            // Before the sort arm below, which must not swallow Ctrl-C as a 'c'.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cancel_listing();
            }
            KeyCode::Char('n') => self.sort_or_reverse(app::SortField::Name.default_mode()),
            KeyCode::Char('s') => self.sort_or_reverse(app::SortField::Size.default_mode()),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.sort_or_reverse(app::SortField::Rentries.default_mode())
            }
            KeyCode::Char('U') => self.sort_or_reverse(app::SortField::Owner.default_mode()),
            KeyCode::Char('T') => self.sort_or_reverse(app::SortField::CTime.default_mode()),
            KeyCode::Char(' ') => {
                // None only until the first listing lands; nowhere to go back to.
                if let Some(original) = self.original_cwd.clone() {
                    self.cd(&original);
                }
            }
            KeyCode::Char('u') => {
                self.toggle_owner();
            }
            KeyCode::Char('t') => {
                self.toggle_ctime();
            }
            KeyCode::Char('d') => {
                self.dir_listing.toggle_dirs_first();
            }
            KeyCode::Char('e') => {
                self.exact = !self.exact;
            }
            KeyCode::Char('r') | KeyCode::F(5) => {
                self.cd(&self.cwd.clone());
            }
            KeyCode::Char('?') | KeyCode::Char('h') => {
                self.help();
            }
            _ => {}
        }
    }
}
