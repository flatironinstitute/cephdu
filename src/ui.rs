use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
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
use crate::app::ListingStats;
use crate::app::Message;
use crate::app::MessageKind;
use crate::format::{CTIME_FMT_WIDTH, Numbers, ctime_str};
use crate::popup::Popup;

pub const POPUP_TEXT_HEIGHT: usize = 10;

const GAUGE_WIDTH: usize = 20;
/// The colors the interface names, for their meaning rather than their shade.
/// Everything else is left to the terminal.
const ERROR_STYLE: Style = Style::new().fg(Color::White).bg(Color::Red);
const WARNING_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Yellow);
/// The cursor row names both of its colors, since with the background inherited
/// neither one alone can be relied on to contrast with it. It shades the background
/// rather than reversing the row: reversing a gauge swaps its filled and empty
/// cells, because a `█` reversed is drawn in the background color while a reversed
/// empty cell paints the foreground across its whole width, so the bar would read
/// backwards. Grey rather than blue: `Color::Blue` is ANSI 4, already the darkest
/// blue there is, so on a theme that renders it brightly there is nothing left to
/// raise the contrast against white with. `DarkGray` is the only named color darker
/// than blue that won't merge into a dark terminal's own background.
/// The row is not emboldened as well: bold brightens the text on top of the color
/// change, which reads as the row lightening rather than being marked.
const SELECTED_STYLE: Style = Style::new().bg(Color::DarkGray).fg(Color::White);
/// What is left when colors are off. crossterm drops every color sequence under
/// NO_COLOR but still sends attributes, so a band made only of colors vanishes and
/// the row would be marked by nothing but the marker.
const SELECTED_STYLE_NO_COLOR: Style = Style::new().add_modifier(Modifier::BOLD);

/// no-color.org: set and non-empty. crossterm reads the same variable to decide
/// whether to emit color sequences at all.
fn colors_disabled() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

const fn selected_style(colors_disabled: bool) -> Style {
    if colors_disabled {
        SELECTED_STYLE_NO_COLOR
    } else {
        SELECTED_STYLE
    }
}

/// Marks the selected row, and is prepended to every row by the list widget.
const HIGHLIGHT_SYMBOL: &str = "> ";
/// Minimum widths of the size and count columns. The unit forms never exceed these,
/// so only exact values widen them.
const SIZE_WIDTH: usize = 8;
const RENTRIES_WIDTH: usize = 7;

/// What every row in a frame has to agree on: the widths, measured once from the
/// whole listing since a column is only as narrow as its widest value, and the
/// number formatting.
struct Columns {
    /// How the cursor row is marked, which depends on whether colors are allowed.
    selected: Style,
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
            .reversed()
            .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        let cols = self.columns();
        let numbers = cols.numbers;

        let stats = format!(
            " {}, {} files ",
            numbers.size(Some(self.dir_listing.stats.total_size), false),
            numbers.count(Some(self.dir_listing.stats.total_rentries), false)
        );

        // Both titles and at least one border cell between them have to fit between
        // the corners; whatever is left over is the path's, and the spaces framing it
        // are part of that.
        let budget = (area.width as usize)
            .saturating_sub(2 + stats.chars().count() + 1)
            .saturating_sub(2);
        let path = truncate_start(self.cwd.to_str().unwrap_or("[invalid UTF-8]"), budget);

        let title = Line::from(format!(" {} ", path)).bold();
        let stats = Line::from(stats).bold();

        let helptitle = Line::from(" Press ? for help ").bold();

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
            .title(stats.right_aligned())
            .title_bottom(Line::from(status).bold().left_aligned())
            .title_bottom(helptitle.right_aligned())
            .border_set(border::THICK);

        let selected = self.dir_listing.selected();
        let items: Vec<ListItem> = self
            .dir_listing
            .iter_entries()
            .enumerate()
            .map(|(i, entry)| {
                entry.to_listitem(&cols, &self.dir_listing.stats).style(
                    if selected.map(|s| s == i).unwrap_or(false) {
                        cols.selected
                    } else {
                        Style::new()
                    },
                )
            })
            .collect();

        // Rows can be wider than the terminal, and ratatui's List has no horizontal
        // offset, so the rows are rendered to a buffer as wide as they need and a
        // window of it is copied out. That scrolls every column alike, and leaves
        // the block to draw the border and its titles in place.
        let content_width = items
            .iter()
            .map(|item| item.width() + HIGHLIGHT_SYMBOL.len())
            .max()
            .unwrap_or(0);

        let inner = block.inner(area);
        block.render(area, buf);

        let list = List::new(items)
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);

        let width = content_width.max(inner.width as usize);
        self.hscroll = self.hscroll.min(width - inner.width as usize);

        let mut rows = Buffer::empty(Rect::new(0, 0, width as u16, inner.height));
        StatefulWidget::render(list, rows.area, &mut rows, self.dir_listing.state_mut());

        for y in 0..inner.height {
            for x in 0..inner.width {
                buf[(inner.x + x, inner.y + y)] = rows[((self.hscroll as u16) + x, y)].clone();
            }
        }
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
            selected: selected_style(colors_disabled()),
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
                MessageKind::Error => ERROR_STYLE,
                MessageKind::Warning => WARNING_STYLE,
                MessageKind::Info => Style::new(),
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
        .title(format!(" {} ", popup.title))
        .borders(Borders::ALL)
        .border_set(top_border_set);

    let footer_block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_set(border::THICK);

    let paragraph = Paragraph::new(Text::from(popup.text.as_str()))
        .block(block)
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Center)
        .scroll((popup.scroll() as u16, 0));

    let footer = Paragraph::new(popup.bottom_title.clone())
        .block(footer_block)
        .centered();

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

/// Keep the end of a path when it is too long to show: the deepest components are
/// the ones that change while navigating, so the start is what gets dropped.
fn truncate_start(path: &str, budget: usize) -> String {
    const MARKER: char = '…';

    let len = path.chars().count();
    if len <= budget {
        return path.to_string();
    }
    if budget == 0 {
        return String::new();
    }

    let tail: String = path.chars().skip(len - (budget - 1)).collect();
    format!("{}{}", MARKER, tail)
}

fn safe_div(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

impl DirEntry {
    fn to_listitem(&self, cols: &Columns, listing_stats: &ListingStats) -> ListItem<'static> {
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

        let mut spans: Vec<Span> = vec![];

        spans.push(Span::raw(format!(
            "{:>width$} ┃",
            cols.numbers.size(self.size, true),
            width = cols.size
        )));

        spans.extend(gauge(size_gauge_fraction, size_gauge_percent, cols.gauge));

        spans.push(Span::raw(format!(
            "┃  {:>width$} ┃",
            cols.numbers.count(self.rentries, true),
            width = cols.rentries
        )));

        spans.extend(gauge(
            rentries_gauge_fraction,
            rentries_gauge_percent,
            cols.gauge,
        ));

        spans.push(Span::raw("┃"));

        if cols.show_owner {
            if let Some(user) = &self.user {
                spans.push(Span::raw(format!(" {:>uwidth$}", user, uwidth = cols.user)));
            }
            if let Some(group) = &self.group {
                spans.push(Span::raw(format!(
                    ":{:gwidth$}",
                    group,
                    gwidth = cols.group
                )));
            }
        }

        if cols.show_ctime
            && let Some(ctime_seconds) = self.ctime
        {
            spans.push(Span::raw(format!(
                " {:cwidth$}",
                ctime_str(ctime_seconds, cols.current_year),
                cwidth = cols.ctime
            )));
        }

        spans.push(Span::raw(format!(" {}", self.display_name())));

        let line = Line::from(spans);
        ListItem::new(line)
    }
}

/// Draw a unicode gauge bar with a given percentage and width.
/// The percentage will be written as a number in the middle of the gauge.
fn gauge(fraction: f64, percent: Option<f64>, width: usize) -> Vec<Span<'static>> {
    let text_start = width / 2 - 3;

    let count = |filled: f64, width: usize| -> (usize, usize) {
        let whole: usize = ((filled * 8.).round().max(0.) as usize).min(8 * width);
        let eighths: usize = whole % 8;
        (whole / 8, eighths)
    };

    // The bar is data, so it keeps the terminal's own rendering on every row: taking
    // the cursor row's foreground, or the bold it falls back to when colors are off,
    // would leave one bar in the column looking different from its neighbours. Only
    // the background behind it comes from the row.
    let bar: Style = Style::new()
        .fg(Color::Reset)
        .remove_modifier(Modifier::BOLD);
    // The percentage over the bar swaps the terminal's own pair, not the row's, for
    // the same reason: reversing the cursor row's blue would put blue text on the
    // bar, which is only legible on some terminals.
    let over_bar: Style = bar.bg(Color::Reset).add_modifier(Modifier::REVERSED);

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
            bar,
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
                over_bar,
            ));
        }
        if split_char < text_width {
            spans.push(Span::styled(
                percent_text[split_char..].to_string(),
                Style::default(),
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

    // A read in progress outranks whatever message was up before it.
    let message = app.progress().or_else(|| app.message.clone());
    app.render_message(&message, message_area, frame.buffer_mut());

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
    use crate::app::{App, DEFAULT_SORT_MODE, DirListing, EntryKind, Options, SortField};
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

    /// An app whose listing is synthetic and whose cwd is faked, so the frame
    /// doesn't depend on the filesystem the tests happen to run on -- App::new
    /// itself reads nothing. The receiver is dropped, which is fine because these
    /// tests never dispatch a read: the listing already has owners and times, so
    /// even `u` and `t` have nothing to fetch.
    fn app() -> App {
        let (mut app, _listings) = App::new(Options::default());
        app.cwd = PathBuf::from("/ceph/users/alice");
        app.dir_listing = DirListing::from_entries(
            entries(),
            true,
            Options {
                sort_mode: DEFAULT_SORT_MODE,
                dirs_first: false,
                owners: true,
                times: true,
                jobs: 1,
            },
        );
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
        "┏ /ceph/users/alice ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 1.0 TB, 100.0 K files ┓",
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

    /// The cursor row is shaded and names both of its colors, and the marker still
    /// points at it.
    #[test]
    fn highlights_the_selected_row() {
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal.draw(|f| ui(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();

        // Row 3 is "..", the initial selection; row 4 is the first real entry. The
        // expected style is read rather than named, so the test holds under NO_COLOR.
        let expected = selected_style(colors_disabled());
        assert_eq!(buf[(5, 3)].bg, expected.bg.unwrap_or(Color::Reset));
        assert_eq!(buf[(5, 3)].fg, expected.fg.unwrap_or(Color::Reset));
        assert!(!buf[(5, 3)].modifier.contains(Modifier::REVERSED));
        // The band and the marker are the cue; the row is not emboldened as well,
        // since bold brightens the text on top of the color change. With colors off
        // that is inverted: bold is all that is left.
        assert_eq!(
            buf[(5, 3)].modifier.contains(Modifier::BOLD),
            expected.add_modifier.contains(Modifier::BOLD),
        );

        assert_eq!(buf[(5, 4)].bg, Color::Reset);
        assert_eq!(buf[(5, 4)].fg, Color::Reset);

        assert!(frame(&mut app)[3].starts_with("┃>"));
    }

    /// The percentage sits legibly on the bar by swapping the row's own two colors.
    /// The bar itself must never be reversed: a reversed `█` is drawn in the
    /// background color and a reversed empty cell paints the foreground, so the bar
    /// would show the wrong value.
    #[test]
    fn the_gauge_reverses_only_its_percentage() {
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal.draw(|f| ui(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();

        let reversed = |x: u16, y: u16| buf[(x, y)].modifier.contains(Modifier::REVERSED);

        // x=15 is bar with no text over it; x=22 falls inside the percentage. Row 4
        // is a/, whose bar is full; row 3 is the selected ".." row.
        for row in [3, 4] {
            assert!(
                !reversed(15, row),
                "the bar itself is reversed on row {}",
                row
            );
        }
        assert!(
            reversed(22, 4),
            "the percentage over the bar is not reversed"
        );
    }

    /// With colors off, the band would be dropped by crossterm and the row left with
    /// nothing but the marker, so it falls back to an attribute instead.
    #[test]
    fn the_cursor_row_survives_no_color() {
        let colored = selected_style(false);
        assert_eq!(colored.bg, Some(Color::DarkGray));
        assert_eq!(colored.fg, Some(Color::White));

        let plain = selected_style(true);
        assert_eq!(plain.bg, None, "a color that NO_COLOR would drop");
        assert_eq!(plain.fg, None, "a color that NO_COLOR would drop");
        assert!(plain.add_modifier.contains(Modifier::BOLD));
    }

    /// Nothing names an absolute color: every cell is either the terminal's own or
    /// one of the 16 it maps itself, which is what lets the interface suit a light
    /// terminal as well as a dark one.
    #[test]
    fn the_interface_names_no_absolute_colors() {
        let mut app = app();
        app.message(Some(Message {
            text: "Warning: not a Ceph directory".to_string(),
            kind: MessageKind::Warning,
        }));
        app.handle_key(KeyEvent::from(KeyCode::Char('?')));

        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal.draw(|f| ui(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();

        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                for color in [cell.fg, cell.bg] {
                    assert!(
                        !matches!(color, Color::Rgb(..) | Color::Indexed(_)),
                        "{:?} at {},{} is absolute",
                        color,
                        x,
                        y
                    );
                }
            }
        }
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
            Options::default(),
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

    /// A wide listing, so that there is something to scroll.
    fn wide_app() -> App {
        let mut app = app();
        app.show_owner = true;
        app.show_ctime = true;
        app
    }

    /// The inside of a row, without the border cell at either end.
    fn row_inside(app: &mut App, row: usize) -> Vec<char> {
        let chars: Vec<char> = frame(app)[row].chars().collect();
        chars[1..chars.len() - 1].to_vec()
    }

    /// The whole listing scrolls, every column alike; the border does not.
    #[test]
    fn scrolling_shifts_the_columns_not_the_border() {
        let mut app = wide_app();

        let before = frame(&mut app);
        let row_before = row_inside(&mut app, 4);

        app.handle_key(KeyEvent::from(KeyCode::Right));
        app.handle_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(app.hscroll, 8);

        let after = frame(&mut app);
        let row_after = row_inside(&mut app, 4);

        let shifted: Vec<char> = row_before.into_iter().skip(8).collect();
        assert_eq!(
            row_after[..shifted.len()],
            shifted[..],
            "the row did not shift by 8 columns"
        );

        // The border and its titles are drawn outside the scrolled window.
        assert_eq!(before[2], after[2], "the top border moved");
        assert_eq!(before[9], after[9], "the bottom border moved");
    }

    /// Rendering clamps the offset, because only the renderer knows how wide the
    /// rows came out.
    #[test]
    fn scrolling_stops_at_the_right_edge() {
        let mut app = wide_app();

        app.hscroll = 10_000;
        let clamped = frame(&mut app);
        let max = app.hscroll;
        assert!(max > 0 && max < 10_000, "offset was not clamped: {}", max);

        app.handle_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(frame(&mut app), clamped, "scrolled past the end");
        assert_eq!(app.hscroll, max);

        app.handle_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(app.hscroll, max - 4);
        assert_ne!(frame(&mut app), clamped);
    }

    #[test]
    fn scrolling_stops_at_the_left_edge() {
        let mut app = wide_app();
        let home = frame(&mut app);

        app.handle_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(app.hscroll, 0);
        assert_eq!(frame(&mut app), home);
    }

    /// Rows that already fit have nothing to scroll.
    #[test]
    fn a_narrow_listing_does_not_scroll() {
        let mut app = app();
        let before = frame(&mut app);

        app.handle_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(frame(&mut app), before, "scrolled a listing that fits");
        assert_eq!(app.hscroll, 0);
    }

    /// The path is what runs out of room, so it loses its start rather than pushing
    /// the totals off the border.
    #[test]
    fn a_long_path_keeps_its_tail() {
        let mut app = app();
        app.cwd =
            PathBuf::from("/ceph/users/alice/projects/simulations/run-0042/outputs/snapshots");

        let border = frame(&mut app)[2].clone();

        assert!(border.contains('…'), "{}", border);
        assert!(border.contains("outputs/snapshots"), "{}", border);
        assert!(
            !border.contains("/ceph/users"),
            "kept the start: {}",
            border
        );
        assert!(border.ends_with("1.0 TB, 100.0 K files ┓"), "{}", border);
        assert_eq!(border.chars().count(), 80, "{}", border);
    }

    /// Widening the totals takes room from the path, not from the border.
    #[test]
    fn the_totals_hold_the_right_edge() {
        let mut app = app();
        app.cwd =
            PathBuf::from("/ceph/users/alice/projects/simulations/run-0042/outputs/snapshots");

        for exact in [false, true] {
            app.exact = exact;
            let border = frame(&mut app)[2].clone();
            assert!(border.ends_with("files ┓"), "exact={}: {}", exact, border);
            assert_eq!(border.chars().count(), 80, "exact={}: {}", exact, border);
        }
    }

    #[test]
    fn truncate_start_keeps_the_tail() {
        // Short enough to show whole, including exactly at the budget.
        assert_eq!(truncate_start("/a/b", 10), "/a/b");
        assert_eq!(truncate_start("/a/b", 4), "/a/b");

        assert_eq!(truncate_start("/aaa/bbb/ccc", 8), "…bbb/ccc");
        assert_eq!(truncate_start("/aaa/bbb/ccc", 1), "…");
        assert_eq!(truncate_start("/aaa/bbb/ccc", 0), "");
    }

    /// The budget is in cells, so a multi-byte path must not overrun it.
    #[test]
    fn truncate_start_counts_characters() {
        let path = "/données/été";
        for budget in 1..=path.chars().count() {
            let shown = truncate_start(path, budget);
            assert_eq!(
                shown.chars().count(),
                budget,
                "{:?} does not fill {} cells",
                shown,
                budget
            );
        }
    }

    /// A bar is data, so it looks the same whichever row the cursor is on: only the
    /// background behind it comes from the row.
    #[test]
    fn the_gauge_keeps_its_colors_on_the_cursor_row() {
        let mut app = app();
        app.handle_key(KeyEvent::from(KeyCode::Down));

        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal.draw(|f| ui(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();

        // Row 4 is a/, now the cursor row; row 5 is b/. Both have bar under x=46 and
        // under the percentage at x=52, in the second gauge.
        for x in [46, 52] {
            assert_eq!(
                buf[(x, 4)].fg,
                buf[(x, 5)].fg,
                "x={} is a different color on the cursor row",
                x
            );
            assert_eq!(buf[(x, 4)].modifier, buf[(x, 5)].modifier, "x={}", x);
        }

        // The row still shades behind the bar, but not behind the percentage, which
        // reverses the terminal's own pair so it reads on any of them.
        let band = selected_style(colors_disabled()).bg.unwrap_or(Color::Reset);
        assert_eq!(buf[(46, 4)].bg, band);
        assert_eq!(buf[(46, 5)].bg, Color::Reset);
        assert_eq!(buf[(52, 4)].bg, Color::Reset);
        assert_eq!(buf[(52, 5)].bg, Color::Reset);
    }

    /// The listing marks a symlink the way `ls -F` does.
    #[test]
    fn symlinks_are_marked_in_the_listing() {
        let mut app = app();
        app.dir_listing = DirListing::from_entries(
            vec![
                entry("link", EntryKind::Symlink, 8, None),
                entry("plain", EntryKind::File, 9, None),
            ],
            true,
            Options::default(),
        );

        let names: Vec<String> = frame(&mut app)[4..6]
            .iter()
            .map(|l| l.split('┃').nth(5).unwrap().trim().to_string())
            .collect();
        assert_eq!(names, ["plain", "link@"]);
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
