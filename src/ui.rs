    use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{
        Color, Modifier, Style, Stylize,
        palette::tailwind::{RED, SLATE, YELLOW},
    },
    symbols::border,
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, StatefulWidget, Widget, Wrap,
    },
};

use crate::app::App;
use crate::app::DirEntry;
use crate::app::EntryKind;
use crate::app::ListingStats;
use crate::app::Message;
use crate::app::MessageKind;
use crate::popup::Popup;

const SELECTED_BG_COLOR: Color = SLATE.c700;
const SELECTED_STYLE: Style = Style::new()
    .bg(SELECTED_BG_COLOR)
    .add_modifier(Modifier::BOLD);
const TEXT_FG_COLOR: Color = SLATE.c50;
const HEADER_BG_COLOR: Color = SLATE.c800;
const DIR_TEXT_COLOR: Color = SLATE.c200;
const NONDIR_TEXT_COLOR: Color = SLATE.c200;
const LIST_BG_COLOR: Color = SLATE.c950;
const GAUGE_COLOR: Color = SLATE.c200;

const ERROR_MESSAGE_STYLE: Style = Style::new().fg(RED.c50).bg(RED.c800);
const WARNING_MESSAGE_STYLE: Style = Style::new().fg(YELLOW.c950).bg(YELLOW.c300);
const INFO_MESSAGE_STYLE: Style = Style::new().fg(SLATE.c50).bg(SLATE.c950);

const POPUP_FG_COLOR: Color = SLATE.c50;
const POPUP_BG_COLOR: Color = SLATE.c950;
pub const POPUP_TEXT_HEIGHT: usize = 10;

const GAUGE_WIDTH: usize = 20;

impl App {
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        Line::from(format!("cephdu v{} ", env!("CARGO_PKG_VERSION")).bold())
            .centered()
            .bg(TEXT_FG_COLOR)
            .fg(HEADER_BG_COLOR)
            .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        let title = Line::from(format!(
            " {} ━━ {}, {} files ",
            self.cwd.to_str().unwrap_or("[invalid UTF-8]"),
            size_str(Some(self.dir_listing.stats.total_size), false),
            rentries_str(Some(self.dir_listing.stats.total_rentries), false)
        ))
        .bold();

        let block = Block::bordered()
            .title(title.left_aligned())
            .border_set(border::THICK);

        let (user_width, group_width) = if self.show_owner {
            (
                self.dir_listing
                    .iter_entries()
                    .filter_map(|e| e.user.as_ref())
                    .map(|s| s.len())
                    .max()
                    .unwrap_or(0),
                self.dir_listing
                    .iter_entries()
                    .filter_map(|e| e.group.as_ref())
                    .map(|s| s.len())
                    .max()
                    .unwrap_or(0),
            )
        } else {
            (0, 0)
        };

        // Iterate through all elements in the `items` and stylize them.
        let selected = self.dir_listing.selected();
        let items: Vec<ListItem> = self
            .dir_listing
            .iter_entries()
            .enumerate()
            .map(|(i, entry)| {
                entry
                    .to_listitem(
                        GAUGE_WIDTH,
                        &self.dir_listing.stats,
                        user_width,
                        group_width,
                        selected.map(|s| s == i).unwrap_or(false),
                        self.show_owner,
                    )
                    .fg(TEXT_FG_COLOR)
                    .bg(if selected.map(|s| s == i).unwrap_or(false) {
                        SELECTED_BG_COLOR
                    } else {
                        LIST_BG_COLOR
                    })
            })
            .collect();

        // Create a List from all list items and highlight the currently selected one
        let list = List::new(items)
            .block(block)
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always)
            .bg(LIST_BG_COLOR);

        StatefulWidget::render(list, area, buf, self.dir_listing.state_mut());
    }

    fn render_message(&self, message: &Option<Message>, area: Rect, buf: &mut Buffer) {
        let message = message.clone().unwrap_or(Message {
            text: " ".to_string(),
            kind: MessageKind::Info,
        });
        Line::from(message.text.as_str())
            .centered()
            .style(match message.kind {
                MessageKind::Error => ERROR_MESSAGE_STYLE,
                MessageKind::Warning => WARNING_MESSAGE_STYLE,
                MessageKind::Info => INFO_MESSAGE_STYLE,
            })
            .render(area, buf);
    }
}

fn render_popup(popup: &mut Popup, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {} ", popup.title),
            Style::default().fg(POPUP_FG_COLOR),
        ))
        .border_style(Style::default().fg(POPUP_FG_COLOR))
        .border_set(border::THICK)
        .bg(LIST_BG_COLOR);

    let paragraph = Paragraph::new(Text::from(popup.text.as_str()))
        .block(block)
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Center)
        .bg(POPUP_BG_COLOR)
        .fg(POPUP_FG_COLOR)
        .scroll((popup.scroll() as u16, 0));

    Clear.render(area, buf);
    paragraph.render(area, buf);
    Scrollbar::new(ScrollbarOrientation::VerticalRight).render(
        area,
        buf,
        &mut popup.scrollbar_state,
    );
}

fn safe_div(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

impl DirEntry {
    fn to_listitem(
        &self,
        gauge_width: usize,
        listing_stats: &ListingStats,
        user_width: usize,
        group_width: usize,
        selected: bool,
        show_owner: bool,
    ) -> ListItem<'static> {
        // The borrow checker complains that self.dir_listing remains borrowed
        // immutably unless we insist on the static lifetime of the ListItem.
        // I'm pretty sure this a borrow checker limitation, rather than a real bug.

        let size_gauge_fraction = safe_div(self.size.unwrap_or(0), listing_stats.max_size);
        let size_gauge_percent = self.size.map(|s| safe_div(s, listing_stats.total_size));

        let rentries_gauge_fraction =
            safe_div(self.rentries.unwrap_or(0), listing_stats.max_rentries);
        let rentries_gauge_percent = self
            .rentries
            .map(|r| safe_div(r, listing_stats.total_rentries));

        let text_color = match self.kind {
            EntryKind::Dir => DIR_TEXT_COLOR,
            _ => NONDIR_TEXT_COLOR,
        };

        let mut spans: Vec<Span> = vec![];

        let style_selected = |span: Span<'static>| -> Span<'static> {
            if selected {
                span.style(SELECTED_STYLE)
            } else {
                span
            }
        };

        spans.push(style_selected(Span::styled(
            format!("{:>8} ┃", size_str(self.size, true)),
            text_color,
        )));

        spans.extend(gauge(
            size_gauge_fraction,
            size_gauge_percent,
            gauge_width,
            selected,
        ));

        spans.push(style_selected(Span::styled(
            format!("┃  {:>7} ┃", rentries_str(self.rentries, true),),
            text_color,
        )));

        spans.extend(gauge(
            rentries_gauge_fraction,
            rentries_gauge_percent,
            gauge_width,
            selected,
        ));

        spans.push(style_selected(Span::styled("┃", text_color)));

        if show_owner {
            if let Some(user) = &self.user {
                spans.push(style_selected(Span::styled(
                    format!(" {:>uwidth$}", user, uwidth = user_width),
                    text_color,
                )));
            }
            if let Some(group) = &self.group {
                spans.push(style_selected(Span::styled(
                    format!(":{:gwidth$}", group, gwidth = group_width),
                    text_color,
                )));
            }
        }

        spans.push(style_selected(Span::styled(
            format!(" {}", self.name),
            text_color,
        )));

        let line = Line::from(spans);
        ListItem::new(line)
    }
}

/// Draw a unicode gauge bar with a given percentage and width.
/// The percentage will be written as a number in the middle of the gauge.
fn gauge(fraction: f64, percent: Option<f64>, width: usize, selected: bool) -> Vec<Span<'static>> {
    let text_start = width / 2 - 3;

    let count = |filled: f64, width: usize| -> (usize, usize) {
        let whole: usize = ((filled * 8.).round().max(0.) as usize).min(8 * width);
        let eighths: usize = whole % 8;
        (whole / 8, eighths)
    };

    let bg_color: Color = if selected {
        SELECTED_BG_COLOR
    } else {
        LIST_BG_COLOR
    };

    let mut spans = vec![];

    let subgauge = |filled: f64, width: usize| -> Span {
        let eighths = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

        let (whole, remainder) = count(filled, width);

        Span::styled(
            format!(
                "{}{}{}",
                "█".repeat(whole),
                eighths[remainder],
                " ".repeat(width - whole - (remainder > 0) as usize)
            ),
            Style::default().fg(GAUGE_COLOR).bg(bg_color),
        )
    };

    let filled = fraction * width as f64;

    let first_subgauge_filled = filled.min(text_start as f64);
    spans.push(subgauge(first_subgauge_filled, text_start));

    let text_width = if let Some(percent) = percent {
        let percent_text = format!("{:>5.1}%", percent * 100.0);
        let text_width = percent_text.len();

        // If the gauge splits the text, invert the colors on the overlapping part.
        let split_char: usize = (filled - (text_start as f64)).round().max(0.) as usize;
        if split_char > 0 {
            spans.push(Span::styled(
                percent_text[..split_char.min(text_width)].to_string(),
                Style::default().bg(GAUGE_COLOR).fg(bg_color),
            ));
        }
        if split_char < text_width {
            spans.push(Span::styled(
                percent_text[split_char..].to_string(),
                Style::default().fg(GAUGE_COLOR).bg(bg_color),
            ));
        }

        text_width
    } else {
        0
    };

    let remaining_width = width.saturating_sub(text_start + text_width);
    let remaining_filled: f64 = (filled - (first_subgauge_filled + text_width as f64)).max(0.);

    spans.push(subgauge(remaining_filled, remaining_width));

    spans
}

fn size_str(size: Option<usize>, align: bool) -> String {
    if size.is_none() {
        return "".to_string();
    }
    let size = size.unwrap();
    let units = [" B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
    let base: usize = 1000;
    let i = if size > 0 {
        size.ilog10() / base.ilog10()
    } else {
        0
    };
    let size = size as f64 / base.pow(i) as f64;
    if i == 0 {
        format!(
            "{:.0}{}{}",
            size,
            if align { "  " } else { "" },
            units[i as usize]
        )
    } else {
        format!("{:.1} {}", size, units[i as usize])
    }
}

fn rentries_str(rentries: Option<usize>, align: bool) -> String {
    if rentries.is_none() {
        return "".to_string();
    }
    let rentries = rentries.unwrap();
    let units = ["", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let base: usize = 1000;
    let i = if rentries > 0 {
        rentries.ilog10() / base.ilog10()
    } else {
        0
    };
    let rentries = rentries as f64 / base.pow(i) as f64;
    if i == 0 {
        format!("{:.0}{}", rentries, if align { "    " } else { "" })
    } else {
        format!("{:.1} {}", rentries, units[i as usize])
    }
}

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(xsize: u16, ysize: u16, r: Rect) -> Rect {
    // Cut the given rectangle into three vertical pieces
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(ysize),
            Constraint::Fill(1),
        ])
        .split(r);

    // Then cut the middle vertical piece into three width-wise pieces
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(xsize),
            Constraint::Fill(1),
        ])
        .split(popup_layout[1])[1] // Return the middle chunk
}

pub fn ui(frame: &mut Frame, app: &mut App) {
    let [header_area, message_area, main_area, _footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    app.render_header(header_area, frame.buffer_mut());
    app.render_list(main_area, frame.buffer_mut());
    // app.render_footer(footer_area, frame.buffer_mut());

    app.render_message(&app.message, message_area, frame.buffer_mut());

    if let Some(popup) = &mut app.popup {
        let popup_area = centered_rect(
            popup.text_width as u16 + 4,
            POPUP_TEXT_HEIGHT as u16 + 2,
            frame.area(),
        );
        render_popup(popup, popup_area, frame.buffer_mut());
    }
}
