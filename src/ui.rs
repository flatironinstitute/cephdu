use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{
        Color, Modifier, Style, Stylize,
        palette::tailwind::{RED, SLATE, YELLOW},
    },
    symbols::{self, border},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, StatefulWidget, Widget, Wrap,
    },
};

use chrono::{Datelike, Local};

use crate::app::App;
use crate::app::DirEntry;
use crate::app::EntryKind;
use crate::app::ListingStats;
use crate::app::Message;
use crate::app::MessageKind;
use crate::format::{CTIME_FMT_WIDTH, Numbers, ctime_str};
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
/// Minimum widths of the size and count columns. The unit forms never exceed these,
/// so only exact values widen them.
const SIZE_WIDTH: usize = 8;
const RENTRIES_WIDTH: usize = 7;

/// The widths and number formatting a frame's rows share. Measured once from the
/// whole listing, since a column is only as narrow as its widest value.
struct Columns {
    gauge: usize,
    size: usize,
    rentries: usize,
    user: usize,
    group: usize,
    ctime: usize,
    numbers: Numbers,
    current_year: isize,
    show_owner: bool,
    show_ctime: bool,
}

impl App {
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        Line::from(format!("cephdu v{} ", env!("CARGO_PKG_VERSION")).bold())
            .centered()
            .bg(TEXT_FG_COLOR)
            .fg(HEADER_BG_COLOR)
            .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        let cols = self.columns();
        let numbers = cols.numbers;

        let title = Line::from(format!(
            " {} ━━ {}, {} files ",
            self.cwd.to_str().unwrap_or("[invalid UTF-8]"),
            numbers.size(Some(self.dir_listing.stats.total_size), false),
            numbers.count(Some(self.dir_listing.stats.total_rentries), false)
        ))
        .fg(TEXT_FG_COLOR)
        .bold();

        let helptitle = Line::from(" Press ? for help ").fg(TEXT_FG_COLOR).bold();

        // Ordering that outlives a keypress needs to be visible, since the listing
        // alone doesn't always reveal it: grouping is invisible when the directories
        // happen to sort first anyway, and two fields can agree on an order.
        let sort_mode = self.dir_listing.sort_mode();
        let mut status = format!(
            " {} {}",
            sort_mode.field().label(),
            if sort_mode.is_reversed() {
                "↓"
            } else {
                "↑"
            }
        );
        if self.dir_listing.dirs_first() {
            status.push_str(" · dirs first");
        }
        status.push(' ');

        let block = Block::bordered()
            .title(title.left_aligned())
            .title_bottom(Line::from(status).fg(TEXT_FG_COLOR).bold().left_aligned())
            .title_bottom(helptitle.right_aligned())
            .border_set(border::THICK);

        let selected = self.dir_listing.selected();
        let items: Vec<ListItem> = self
            .dir_listing
            .iter_entries()
            .enumerate()
            .map(|(i, entry)| {
                entry
                    .to_listitem(
                        &cols,
                        &self.dir_listing.stats,
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

        StatefulWidget::render(list, area, buf, self.dir_listing.state_mut());
    }

    fn columns(&self) -> Columns {
        let numbers = Numbers::from_exact(self.exact);

        let widest = |f: &dyn Fn(&DirEntry) -> String| -> usize {
            self.dir_listing
                .iter_entries()
                .map(|e| f(e).len())
                .max()
                .unwrap_or(0)
        };

        let (user, group) = if self.show_owner {
            (
                widest(&|e| e.user.clone().unwrap_or_default()),
                widest(&|e| e.group.clone().unwrap_or_default()),
            )
        } else {
            (0, 0)
        };

        Columns {
            gauge: GAUGE_WIDTH,
            size: SIZE_WIDTH.max(widest(&|e| numbers.size(e.size, true))),
            rentries: RENTRIES_WIDTH.max(widest(&|e| numbers.count(e.rentries, true))),
            user,
            group,
            ctime: if self.show_ctime { CTIME_FMT_WIDTH } else { 0 },
            numbers,
            current_year: Local::now().year() as isize,
            show_owner: self.show_owner,
            show_ctime: self.show_ctime,
        }
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

fn render_popup(popup: &mut Popup, areas: [Rect; 2], buf: &mut Buffer) {
    let top_border_set = symbols::border::Set {
        // Connect the top block with the bottom block
        bottom_left: symbols::line::THICK.vertical_right,
        ..symbols::border::THICK
    };

    let block = Block::default()
        .title(Span::styled(
            format!(" {} ", popup.title),
            Style::default().fg(POPUP_FG_COLOR),
        ))
        .borders(Borders::ALL)
        .border_set(top_border_set)
        .border_style(Style::default().fg(POPUP_FG_COLOR))
        .bg(LIST_BG_COLOR);

    let footer_block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
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

    let footer = Paragraph::new(popup.bottom_title.clone())
        .block(footer_block)
        .centered()
        .fg(POPUP_FG_COLOR);

    Clear.render(areas[0], buf);
    Clear.render(areas[1], buf);

    paragraph.render(areas[0], buf);
    footer.render(areas[1], buf);

    Scrollbar::new(ScrollbarOrientation::VerticalRight).render(
        areas[0],
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
        cols: &Columns,
        listing_stats: &ListingStats,
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
            format!(
                "{:>width$} ┃",
                cols.numbers.size(self.size, true),
                width = cols.size
            ),
            text_color,
        )));

        spans.extend(gauge(
            size_gauge_fraction,
            size_gauge_percent,
            cols.gauge,
            selected,
        ));

        spans.push(style_selected(Span::styled(
            format!(
                "┃  {:>width$} ┃",
                cols.numbers.count(self.rentries, true),
                width = cols.rentries
            ),
            text_color,
        )));

        spans.extend(gauge(
            rentries_gauge_fraction,
            rentries_gauge_percent,
            cols.gauge,
            selected,
        ));

        spans.push(style_selected(Span::styled("┃", text_color)));

        if cols.show_owner {
            if let Some(user) = &self.user {
                spans.push(style_selected(Span::styled(
                    format!(" {:>uwidth$}", user, uwidth = cols.user),
                    text_color,
                )));
            }
            if let Some(group) = &self.group {
                spans.push(style_selected(Span::styled(
                    format!(":{:gwidth$}", group, gwidth = cols.group),
                    text_color,
                )));
            }
        }

        if cols.show_ctime
            && let Some(ctime_seconds) = self.ctime
        {
            spans.push(style_selected(Span::styled(
                format!(
                    " {:cwidth$}",
                    ctime_str(ctime_seconds, cols.current_year),
                    cwidth = cols.ctime
                ),
                text_color,
            )));
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

fn popup_rects(xsize: u16, ysize: u16, r: Rect) -> [Rect; 2] {
    // Cut the x axis
    let xrect = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(xsize),
            Constraint::Fill(1),
        ])
        .split(r)[1]; // Return the middle chunk

    // Cut the y axis
    let yrects = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(ysize),
            Constraint::Length(2), // popup footer
            Constraint::Fill(1),
        ])
        .split(xrect);

    [yrects[1], yrects[2]]
}

pub fn ui(frame: &mut Frame, app: &mut App) {
    let [header_area, message_area, main_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(frame.area());

    app.render_header(header_area, frame.buffer_mut());
    app.render_list(main_area, frame.buffer_mut());

    app.render_message(&app.message, message_area, frame.buffer_mut());

    if let Some(popup) = &mut app.popup {
        let popup_areas = popup_rects(
            popup.text_width as u16 + 4,
            POPUP_TEXT_HEIGHT as u16 + 2,
            frame.area(),
        );
        render_popup(popup, popup_areas, frame.buffer_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, DEFAULT_SORT_MODE, DirListing, EntryKind, SortField};
    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn entry(name: &str, kind: EntryKind, size: usize, rentries: Option<usize>) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            kind,
            size: Some(size),
            rentries,
            ctime: Some(992_606_400),
            user: Some("alice".to_string()),
            group: Some("scc".to_string()),
        }
    }

    fn entries() -> Vec<DirEntry> {
        vec![
            entry("a/", EntryKind::Dir, 800_000_000_000, Some(60_000)),
            entry("b/", EntryKind::Dir, 200_000_000_000, Some(40_000)),
            entry("c.dat", EntryKind::File, 1_000_000, None),
        ]
    }

    /// An app whose listing is synthetic, so the frame doesn't depend on the
    /// filesystem the tests happen to run on. The cwd is faked for the same reason;
    /// App::new only needs a readable directory to start from.
    fn app() -> App {
        let mut app = App::new(Some(&PathBuf::from(".")), DEFAULT_SORT_MODE, false).unwrap();
        app.cwd = PathBuf::from("/ceph/users/alice");
        app.dir_listing = DirListing::from_entries(entries(), true, DEFAULT_SORT_MODE, false);
        app.message(None);
        app
    }

    fn frame(app: &mut App) -> Vec<String> {
        frame_sized(app, 80, 10)
    }

    fn frame_sized(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| ui(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Everything below the header, which carries the version number.
    const EXPECTED: &[&str] = &[
        "",
        "┏ /ceph/users/alice ━━ 1.0 TB, 100.0 K files ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓",
        "┃>          ┃                    ┃          ┃                    ┃ ..          ┃",
        "┃  800.0 GB ┃███████ 80.0%███████┃   60.0 K ┃███████ 60.0%███████┃ a/          ┃",
        "┃  200.0 GB ┃█████   20.0%       ┃   40.0 K ┃███████ 40.0%▍      ┃ b/          ┃",
        "┃    1.0 MB ┃         0.0%       ┃          ┃                    ┃ c.dat       ┃",
        "┃                                                                              ┃",
        "┃                                                                              ┃",
        "┗ size ↓ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ Press ? for help ┛",
    ];

    #[test]
    fn renders_the_listing() {
        let lines = frame(&mut app());

        assert!(lines[0].contains("cephdu v"), "{:?}", lines[0]);
        assert_eq!(&lines[1..], EXPECTED, "\n{}", lines.join("\n"));
    }

    /// The row the cursor is on is filled manually rather than by ratatui's
    /// highlight style, so the fill is worth checking directly.
    #[test]
    fn highlights_the_selected_row() {
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal.draw(|f| ui(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();

        // Row 2 is "..", the initial selection; row 3 is the first real entry.
        assert_eq!(buf[(5, 3)].bg, SELECTED_BG_COLOR);
        assert_eq!(buf[(5, 4)].bg, LIST_BG_COLOR);
    }

    #[test]
    fn moving_the_cursor_moves_the_marker() {
        let mut app = app();
        assert!(frame(&mut app)[3].starts_with("┃>"));

        app.handle_key(KeyEvent::from(KeyCode::Down));
        let lines = frame(&mut app);
        assert!(lines[3].starts_with("┃ "), "{:?}", lines[3]);
        assert!(lines[4].starts_with("┃>"), "{:?}", lines[4]);
        assert!(lines[4].contains("a/"), "{:?}", lines[4]);

        // The cursor must stop at the top rather than wrapping.
        app.handle_key(KeyEvent::from(KeyCode::Up));
        app.handle_key(KeyEvent::from(KeyCode::Up));
        assert!(frame(&mut app)[3].starts_with("┃>"));
    }

    /// Wide enough that both optional columns fit; at 80 columns the time column
    /// is truncated once the owner column is also shown.
    #[test]
    fn owner_and_time_columns_are_toggled() {
        let mut app = app();
        let wide = |app: &mut App| frame_sized(app, 120, 10);

        let hidden = wide(&mut app);
        assert!(!hidden.iter().any(|l| l.contains("alice:scc")));
        assert!(!hidden.iter().any(|l| l.contains("2001")));

        app.handle_key(KeyEvent::from(KeyCode::Char('u')));
        assert!(
            wide(&mut app).iter().any(|l| l.contains("alice:scc")),
            "owner column did not appear"
        );

        app.handle_key(KeyEvent::from(KeyCode::Char('t')));
        let both = wide(&mut app);
        assert!(
            both.iter().any(|l| l.contains("2001")),
            "time column did not appear:\n{}",
            both.join("\n")
        );
        assert!(both.iter().any(|l| l.contains("alice:scc")));

        app.handle_key(KeyEvent::from(KeyCode::Char('u')));
        app.handle_key(KeyEvent::from(KeyCode::Char('t')));
        assert_eq!(wide(&mut app), hidden, "toggling back changed the frame");
    }

    #[test]
    fn sorting_reorders_the_rows() {
        let mut app = app();
        // Rows 4..7 are the real entries; row 3 is "..".
        let names = |app: &mut App| -> Vec<String> {
            frame(app)[4..7]
                .iter()
                .map(|l| l.split('┃').nth(5).unwrap().trim().to_string())
                .collect()
        };

        // Default is descending by size; pressing 's' again reverses it.
        assert_eq!(names(&mut app), ["a/", "b/", "c.dat"]);
        app.handle_key(KeyEvent::from(KeyCode::Char('s')));
        assert_eq!(names(&mut app), ["c.dat", "b/", "a/"]);

        // The key and the -n flag resolve to the same mode.
        app.handle_key(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(app.dir_listing.sort_mode(), SortField::Name.default_mode());
        assert_eq!(names(&mut app), ["a/", "b/", "c.dat"]);
    }

    /// The status area names the field and points the way the values run.
    #[test]
    fn the_status_area_tracks_the_sort() {
        let mut app = app();
        let status = |app: &mut App| -> String {
            let border = frame(app).last().unwrap().clone();
            border
                .trim_start_matches('┗')
                .split('━')
                .next()
                .unwrap()
                .trim()
                .to_string()
        };

        assert_eq!(status(&mut app), "size ↓", "the default is largest first");

        // Same field again reverses it.
        app.handle_key(KeyEvent::from(KeyCode::Char('s')));
        assert_eq!(status(&mut app), "size ↑");

        app.handle_key(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(status(&mut app), "name ↑");

        app.handle_key(KeyEvent::from(KeyCode::Char('c')));
        assert_eq!(status(&mut app), "count ↓");

        app.handle_key(KeyEvent::from(KeyCode::Char('d')));
        assert_eq!(status(&mut app), "count ↓ · dirs first");
    }

    /// The default fixture can't show this: its directories are already the largest.
    #[test]
    fn dirs_first_key_regroups_the_rows() {
        let mut app = app();
        app.dir_listing = DirListing::from_entries(
            vec![
                entry("a/", EntryKind::Dir, 100_000, Some(1)),
                entry("big.dat", EntryKind::File, 999_000, None),
            ],
            true,
            DEFAULT_SORT_MODE,
            false,
        );

        let names = |app: &mut App| -> Vec<String> {
            frame(app)[4..6]
                .iter()
                .map(|l| l.split('┃').nth(5).unwrap().trim().to_string())
                .collect()
        };

        let shows_indicator = |app: &mut App| frame(app).join("\n").contains("dirs first");

        assert_eq!(names(&mut app), ["big.dat", "a/"]);
        assert!(!shows_indicator(&mut app), "indicator shown while off");

        app.handle_key(KeyEvent::from(KeyCode::Char('d')));
        assert_eq!(names(&mut app), ["a/", "big.dat"], "'d' did not regroup");
        assert!(shows_indicator(&mut app), "no indicator while grouping");

        app.handle_key(KeyEvent::from(KeyCode::Char('d')));
        assert_eq!(
            names(&mut app),
            ["big.dat", "a/"],
            "'d' did not toggle back"
        );
        assert!(!shows_indicator(&mut app), "indicator outlived the mode");
    }

    #[test]
    fn exact_key_switches_the_numbers() {
        let mut app = app();
        let text = |app: &mut App| frame(app).join("\n");

        let units = text(&mut app);
        assert!(units.contains("800.0 GB"), "{}", units);
        assert!(units.contains("60.0 K"), "{}", units);

        app.handle_key(KeyEvent::from(KeyCode::Char('e')));
        let exact = text(&mut app);
        assert!(exact.contains("800000000000"), "{}", exact);
        assert!(exact.contains("60000"), "{}", exact);
        assert!(!exact.contains("800.0 GB"), "{}", exact);
        // The totals in the title follow the same setting.
        assert!(exact.contains("100000 files"), "{}", exact);

        app.handle_key(KeyEvent::from(KeyCode::Char('e')));
        assert_eq!(text(&mut app), units, "'e' did not toggle back");
    }

    /// Exact values are wider than any unit form, so the column grows. The rest of
    /// the row has to stay aligned and inside the frame when it does.
    #[test]
    fn exact_widens_its_column_without_raggedness() {
        let mut app = app();
        app.exact = true;
        let lines = frame(&mut app);

        // Character columns, not byte offsets: the gauges are three bytes per cell.
        let name_columns: Vec<usize> = lines[3..7]
            .iter()
            .zip(["..", "a/", "b/", "c.dat"])
            .map(|(line, name)| {
                let byte = line.rfind(name).unwrap();
                line[..byte].chars().count()
            })
            .collect();
        assert!(
            name_columns.windows(2).all(|w| w[0] == w[1]),
            "ragged name column:\n{}",
            lines.join("\n")
        );

        // Rows 2 onwards are the bordered block, which spans the full width.
        for line in &lines[2..] {
            assert_eq!(line.chars().count(), 80, "{:?} is not full width", line);
        }
    }

    /// The status area lives in the bottom border and must not disturb the rest of it.
    #[test]
    fn dirs_first_indicator_shares_the_bottom_border() {
        let mut app = app();
        let plain = frame(&mut app);

        app.dir_listing.toggle_dirs_first();
        let grouped = frame(&mut app);

        assert_eq!(
            grouped[..grouped.len() - 1],
            plain[..plain.len() - 1],
            "the indicator changed a row other than the bottom border"
        );

        let border = grouped.last().unwrap();
        assert!(border.contains("dirs first"), "{}", border);
        assert!(border.contains("Press ? for help"), "{}", border);
        assert_eq!(border.chars().count(), 80, "{}", border);
    }

    #[test]
    fn help_popup_lists_the_keys() {
        let mut app = app();
        app.handle_key(KeyEvent::from(KeyCode::Char('?')));

        let lines = frame(&mut app);
        let text = lines.join("\n");
        assert!(text.contains("Help"), "{}", text);
        assert!(text.contains("Quit"), "{}", text);
        assert!(text.contains(env!("CARGO_PKG_REPOSITORY")), "{}", text);

        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert_eq!(frame(&mut app)[1..], *EXPECTED, "popup did not close");
    }

    /// A message replaces the blank line above the listing, and must not disturb it.
    #[test]
    fn messages_render_above_the_listing() {
        let mut app = app();
        app.message(Some(Message {
            text: "Warning: not a Ceph directory".to_string(),
            kind: MessageKind::Warning,
        }));

        let lines = frame(&mut app);
        assert!(lines[1].contains("not a Ceph directory"), "{:?}", lines[1]);
        assert_eq!(&lines[2..], &EXPECTED[1..], "\n{}", lines.join("\n"));
    }
}
