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
use crate::app::Popup;

// const SELECTED_MODIFIER: Modifier = Modifier::REVERSED;
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

const MIN_RENTRIES_COL_WIDTH: usize = 5;

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

        let rentries_width = self
            .dir_listing
            .max_rentries
            .to_string()
            .len()
            .max(MIN_RENTRIES_COL_WIDTH);
        let gauge_width = 20; // TODO?

        // Iterate through all elements in the `items` and stylize them.
        let selected = self.dir_listing.state.selected();
        let items: Vec<ListItem> = self
            .dir_listing
            .iter_entries()
            .enumerate()
            .map(|(i, entry)| {
                entry
                    .to_listitem(
                        gauge_width,
                        self.dir_listing.max_size,
                        self.dir_listing.total_size,
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

impl DirEntry {
    fn to_listitem(
        &self,
        gauge_width: usize,
        max_size: usize,
        total_size: usize,
        rentries_width: usize,
        selected: bool,
    ) -> ListItem<'static> {
        // The borrow checker complains that self.dir_listing remains borrowed
        // immutably unless we insist on the static lifetime of the ListItem.
        // I'm pretty sure this a borrow checker limitation, rather than a real bug.

        let size = self.size.unwrap_or(0);
        let fraction = size as f64 / max_size as f64;
        let text_color = match self.kind {
            EntryKind::Dir => DIR_TEXT_COLOR,
            _ => NONDIR_TEXT_COLOR,
        };

        let mut spans: Vec<Span> = vec![];

        let style_selected = |span: Span<'static>| -> Span<'static> {
            if selected {
                // span.add_modifier(SELECTED_MODIFIER)
                span.style(SELECTED_STYLE)
            } else {
                span
            }
        };

        spans.push(style_selected(Span::styled(
            format!("{:>10} ┃", size_str(size)),
            text_color,
        )));

        spans.extend(gauge(
            fraction,
            self.size.map(|s| s as f64 / total_size as f64),
            gauge_width,
            selected,
        ));

        spans.push(style_selected(Span::styled(
            format!(
                "┃ {:rwidth$} {}",
                self.rentries
                    .map(|r| r.to_string())
                    .unwrap_or("".to_string()),
                self.name,
                rwidth = rentries_width,
            ),
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

fn size_str(size: usize) -> String {
    let units = [
        "  B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB",
    ];
    let base: usize = 1024;
    let i = if size > 0 {
        size.ilog2() / base.ilog2()
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
