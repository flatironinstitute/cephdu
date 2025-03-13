use ratatui::widgets::ScrollbarState;

use crate::ui::POPUP_TEXT_HEIGHT;

#[derive(Debug)]
pub struct Popup {
    pub title: String,
    pub bottom_title: String,
    pub text: String,
    pub text_width: usize,
    pub text_height: usize,
    scroll: usize,
    pub scrollbar_state: ScrollbarState,
}

impl Popup {
    pub fn new(title: &str, bottom_title: &str, text: &str) -> Self {
        let text_width = text
            .lines()
            .map(|line| line.len())
            .max()
            .unwrap_or(0)
            .max(title.len())
            .max(bottom_title.len() + 2);
        let text_height = text.lines().count();
        Popup {
            title: title.to_string(),
            bottom_title: bottom_title.to_string(),
            text: text.to_string(),
            text_width,
            text_height,
            scroll: 0,
            scrollbar_state: ScrollbarState::default()
                .position(0)
                .content_length(text_height.saturating_sub(POPUP_TEXT_HEIGHT)),
        }
    }
    pub fn scroll(&self) -> usize {
        self.scroll
    }
    pub fn scroll_by(&mut self, delta: isize) -> usize {
        let new_scroll = (self.scroll as isize + delta).max(0) as usize;
        self.scroll_to(new_scroll)
    }

    pub fn scroll_to(&mut self, line: usize) -> usize {
        self.scroll = line.min(self.max_scroll());
        self.scrollbar_state = self.scrollbar_state.position(self.scroll);
        self.scroll
    }

    fn max_scroll(&self) -> usize {
        self.text_height.saturating_sub(POPUP_TEXT_HEIGHT)
    }
}
