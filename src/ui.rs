use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize, palette::tailwind::SLATE},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph, StatefulWidget, Widget,
        Wrap,
    },
};

use crate::app::App;
use crate::app::DirEntry;
use crate::app::EntryKind;
use crate::app::ListingStats;
use crate::app::Popup;

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

const GAUGE_WIDTH: usize = 20;

impl App {
    fn render_header(&mut self, area: Rect, buf: &mut Buffer) {
        Line::from(format!("cephdu v{} ", env!("CARGO_PKG_VERSION")).bold())
            .centered()
            .bg(TEXT_FG_COLOR)
            .fg(HEADER_BG_COLOR)
            .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        let title =
            Line::from(format!(" {} ", self.cwd.to_str().unwrap_or("[invalid UTF-8]")).bold());
        // let title = Line::from(self.cwd);

        let block = Block::bordered()
            .title(title.left_aligned())
            .border_set(border::THICK);

        let rentries_width = 7;

        // Iterate through all elements in the `items` and stylize them.
        let selected = self.dir_listing.state.selected();
        let items: Vec<ListItem> = self
            .dir_listing
            .iter_entries()
            .enumerate()
            .map(|(i, entry)| {
                entry
                    .to_listitem(
                        GAUGE_WIDTH,
                        &self.dir_listing.stats,
                        rentries_width,
                        selected.map(|s| s == i).unwrap_or(false),
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

        StatefulWidget::render(list, area, buf, &mut self.dir_listing.state);
    }

    fn render_popup(&mut self, popup: Popup, area: Rect, buf: &mut Buffer) {
        let text = Span::styled(popup.text, Style::default().fg(Color::White));

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {} ", popup.title),
                Style::default().fg(Color::White),
            ))
            .border_style(Style::default().fg(Color::White))
            .border_set(border::THICK)
            .bg(LIST_BG_COLOR);

        let paragraph = Paragraph::new(Text::from(text))
            .block(block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });

        Clear.render(area, buf);
        paragraph.render(area, buf);
    }
}

fn safe_div(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

impl DirEntry {
    fn to_listitem(
        &self,
        gauge_width: usize,
        listing_stats: &ListingStats,
        rentries_width: usize,
        selected: bool,
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
            format!("{:>8} ┃", size_str(self.size)),
            text_color,
        )));

        spans.extend(gauge(
            size_gauge_fraction,
            size_gauge_percent,
            gauge_width,
            selected,
        ));

        spans.push(style_selected(Span::styled(
            format!(
                "┃  {:>rwidth$} ┃",
                rentries_str(self.rentries),
                rwidth = rentries_width,
            ),
            text_color,
        )));

        spans.extend(gauge(
            rentries_gauge_fraction,
            rentries_gauge_percent,
            gauge_width,
            selected,
        ));

        spans.push(style_selected(Span::styled(
            format!("┃ {}", self.name),
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

fn size_str(size: Option<usize>) -> String {
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
        format!("{:.0}   {}", size, units[i as usize])
    } else {
        format!("{:.1} {}", size, units[i as usize])
    }
}

fn rentries_str(rentries: Option<usize>) -> String {
    if rentries.is_none() {
        return "".to_string();
    }
    let rentries = rentries.unwrap();
    let units = ["  ", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let base: usize = 1000;
    let i = if rentries > 0 {
        rentries.ilog10() / base.ilog10()
    } else {
        0
    };
    let rentries = rentries as f64 / base.pow(i) as f64;
    if i == 0 {
        format!("{:.0}  {}", rentries, units[i as usize])
    } else {
        format!("{:.1} {}", rentries, units[i as usize])
    }
}

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    // Cut the given rectangle into three vertical pieces
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    // Then cut the middle vertical piece into three width-wise pieces
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1] // Return the middle chunk
}

pub fn ui(frame: &mut Frame, app: &mut App) {
    let [header_area, main_area, _footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    app.render_header(header_area, frame.buffer_mut());
    app.render_list(main_area, frame.buffer_mut());
    // app.render_footer(footer_area, frame.buffer_mut());

    if let Some(popup) = &app.popup {
        let popup_area = centered_rect(50, 25, frame.area());
        app.render_popup(popup.clone(), popup_area, frame.buffer_mut());
    }
}
