use ratatui::widgets::ScrollbarState;

#[derive(Debug)]
pub struct Popup {
    pub title: String,
    pub bottom_title: String,
    pub text: String,
    pub text_width: usize,
    pub text_height: usize,
    scroll: usize,
    /// Rows of text on screen, which only the renderer knows -- it depends on the
    /// terminal. Zero until the first frame, and everything that depends on it is
    /// re-clamped when it changes, so a key pressed before that frame can't leave
    /// the scroll out of range.
    view_height: usize,
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
            view_height: 0,
            scrollbar_state: ScrollbarState::default().position(0),
        }
    }

    /// Tell the popup how many rows of text it is being drawn into.
    pub fn set_view_height(&mut self, view_height: usize) {
        if view_height == self.view_height {
            return;
        }
        self.view_height = view_height;
        // A taller terminal can leave the scroll past the new end.
        self.scroll_to(self.scroll);
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
        self.scrollbar_state = self
            .scrollbar_state
            .position(self.scroll)
            .content_length(self.max_scroll());
        self.scroll
    }

    pub fn scroll_to_end(&mut self) -> usize {
        self.scroll_to(usize::MAX)
    }

    /// The first line that can be shown at the top with text still filling the
    /// view. Zero when the whole text fits, which is also when the scrollbar has
    /// nothing to say.
    pub fn max_scroll(&self) -> usize {
        self.text_height.saturating_sub(self.view_height)
    }
}
