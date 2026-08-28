# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```console
cargo run [-- PATH]              # build and launch the TUI
cargo run -- --flat [PATH]       # flat text mode, which is scriptable and testable
cargo build --release
cargo clippy --all-targets       # pre-commit runs fmt + cargo-check + clippy
pre-commit run --all-files

cargo test
cargo test --bin cephdu          # unit tests only (fast, no filesystem)
cargo test <substring>           # one test, e.g. cargo test renders_the_listing
cargo test -- --nocapture        # SKIP notices from tests/ceph.rs, syscall table from tests/syscalls.rs
CEPHDU_TEST_DIR=<ceph path> cargo test --test ceph
```

Compile-time configuration: `CEPHDU_DEFAULT_DIR=/mnt/ceph/users/\$USER cargo build --release` bakes in a
fallback directory (read via `option_env!` in [main.rs](src/main.rs)); the literal `$USER` is substituted at
runtime. Release binaries are built for gnu/musl × x86_64/aarch64 by
[.github/workflows/release.yml](.github/workflows/release.yml) on GitHub Release publish.

Rust edition 2024; the code uses let-chains, so a recent toolchain is required.

## Architecture

The premise: on CephFS, recursive directory size, file count, and mtime are available as extended attributes
(`ceph.dir.rbytes`, `ceph.dir.rentries`, `ceph.dir.rctime`), so disk usage needs no tree walk. Everything else
follows from that.

Layering, roughly bottom-up:

- [fs.rs](src/fs.rs) — the only `unsafe`/libc code: `lgetxattr` for the three r-attrs, `statfs` for
  filesystem-type detection, `getpwuid_r` for uid→name (memoized in a global `NAME_CACHE`). Returns `Option`
  everywhere; a missing xattr is normal, not an error.
- [app.rs](src/app.rs) — all state. `App` holds the cwd, one `DirListing`, and the machinery for the listing in
  flight (`pending`, a generation counter, the worker channel's sender); `DirListing` holds `Vec<DirEntry>`
  plus a ratatui `ListState`, sort mode, and aggregate `ListingStats`. `DirEntry::from` is where the per-entry
  stat and xattr calls happen.
- [format.rs](src/format.rs) — the base-1000 size/count units and the `ls -l`-style time format, shared by both
  output modes. Pure functions, so this is where formatting tests live.
- [navigation.rs](src/navigation.rs) — `App::handle_key`, plus the `HELP` table that is both the key mapping's
  documentation and the source of the `?` popup text. Add a keybinding *and* its `HELP` row together.
- [ui.rs](src/ui.rs) — rendering from `&mut App`. The only state it writes is ratatui's `ListState` and the
  clamp of `App::hscroll`, both of which depend on the area being drawn into.
- [flat.rs](src/flat.rs) — the non-interactive renderer, which takes a `DirListing` and a `Write`.
- [popup.rs](src/popup.rs) — scrollable modal used only for help so far.

Control flow for the TUI is a `tokio::select!` loop in `run_app` (issue #18): draw, then wait on whichever comes
first of a key event (crossterm's `EventStream`), a listing worker's answer, or — only while a read is in
flight — a 100ms tick that keeps the progress notice current. There is no free-running tick. The runtime is
current-thread and multiplexes wake-ups only: every filesystem call is a blocking syscall, so listings run on
plain `std::thread`s that `App::start_listing` spawns, not on runtime tasks — which is also why quitting never
waits on a worker stuck in a syscall (nothing joins it; process exit reaps it) and why `App` works without a
runtime, unit tests included. The moving parts and their traps:

- Cancellation is cooperative and stays so under any runtime: a thread mid-`lgetxattr` cannot be aborted, so
  `ls()` checks an atomic flag between entries (a load, not a syscall — [tests/syscalls.rs](tests/syscalls.rs)
  still holds) and returns `ErrorKind::Interrupted`. The flag lives in `ListingWatch`, shared worker/interface,
  which also carries the entries-so-far counter the progress notice reads.
- A worker's answer is applied only if its generation matches the one `pending` — superseding (a `cd` during a
  `cd`) and cancelling both orphan the old answer, which may still arrive and must be dropped, not applied.
  Anything that changes what a listing shows goes through `start_listing`/`on_listing_msg`; there is no
  synchronous cd left.
- Cancel semantics: Esc and Ctrl-C stop the read and keep the directory on screen (the old listing was never
  replaced); while idle they do nothing. Only `q` quits, deliberately: a cancel key that doubled as quit would
  make an extra press — or one landing just after the read finishes — exit unintended, which is why Esc lost its
  quit binding when it gained cancel. The Ctrl-C key arm must stay *above* the `'c'` sort arm with a `CONTROL`
  guard, or Ctrl-C sorts by count again.
- Startup goes through the same path: `App::new` reads nothing, `run_app` dispatches the first listing, and the
  old fall-back-to-`.` logic is the `OnError::Fallback` arm. `Pending::preserve_message` exists because that
  fallback sets a warning that the fallback listing's arrival must not clear.
- In unit tests, every call that can dispatch a read (`cd`, the toggles, the sorts) needs `App::pump` after it,
  which drains the channel the way the event loop would — otherwise the answer sits unapplied and the test
  asserts against the stale listing. The ui.rs tests do the opposite: they drop the receiver and must never
  dispatch, which holds because their synthetic listing already has owners and times.

### Output modes

`main` picks the mode before building an `App`: flat mode goes straight from `DirListing::from` to
`flat::write_listing`, never constructing one. The TUI draws to stdout, so it can't work when stdout is
redirected — hence flat mode is implied when stdout isn't a terminal, with `--tui` as the escape hatch for
pty-wrapping tools. Two properties of flat mode are deliberate and worth preserving:

- `--flat` (`-f`) selects the format with units and `--parseable` (`-p`) the raw one. Neither varies with the
  terminal, so parsers see the same bytes however the tool is invoked. This is also why flat mode has no
  column-visibility flags: the TUI's `u`/`t` toggles exist
  because terminal width is scarce, a pipe has no such limit, and a fixed column set keeps field offsets stable.
  It also leaves the remaining `-<key>` short flags free for the startup sort flags in issue #11.
- An implied flat listing (no flag, stdout not a terminal) is parseable rather than human, on the grounds that a
  pipe is more often read by a program.
- Flat mode does *not* fall back to the current directory when the path fails to open, unlike the TUI. A script
  needs the failure.

Adding a column to flat mode is a breaking change for anything parsing it; append rather than insert.

### Traps

The listing has two independent index transformations between storage order and display order, and both live in
`DirListing`:

1. `entries` is always sorted **ascending**. Reversed sort modes are applied at read time — `iter_entries()`
   reverses the iterator and `get()` mirrors the index. Nothing re-sorts on reversal (see `sort()`, which
   short-circuits when only the direction changed).
2. `..` is a synthetic entry in the separate `dotdot` field, displayed at index 0 and absent from `entries`.
   `len()`, `get()`, and anything consuming `selected()` must account for the +1 offset.

Both interact with `dirs_first`, which is why its grouping key in `sort()` is inverted under a reversed mode:
storage is ascending, so directories have to be stored *last* to display *first*. Two consequences that are easy
to get wrong — a change in direction alone now requires a re-sort, so `sort()` can only short-circuit while
`dirs_first` is off, and any new grouping must go through the same key rather than being applied at display time.

Any new accessor that maps a selection index to a `DirEntry` must go through `get()` rather than indexing
`entries`.

Other things worth knowing before editing:

- A sort field has three touch points: the `SortField` variant, the key arm in [navigation.rs](src/navigation.rs)
  with its `HELP` row, and the flag in `SortFlags` in [main.rs](src/main.rs). `default_mode()` and `label()` are
  exhaustive matches on the enum, so the compiler catches those two. Its starting direction lives in one
  place only, `SortField::default_mode()`, which both the key and the flag go through — that is what keeps `-s`
  and pressing `s` in agreement, so don't reintroduce a literal `SortMode::Reversed(...)` at a call site. `-r`
  applies `as_reversed()` on top and so needs nothing per field. Uppercase short flags were considered for
  reversal and rejected: `U` and `T` are already the interface's sort keys for owner and time, so `-T` would have
  meant the opposite of pressing `T`.
- Symlinks: `DirEntry::name` carries the directory `/` but *not* the symlink `@`, and the difference is
  deliberate. `/` cannot occur in a filename, so it is unambiguous in the parseable stream, which documents it.
  `@` can occur, so marking names with it there would be indistinguishable from a real character;
  `display_name()` applies it and only the TUI and the human flat format call that. The rest of #12 is untouched:
  the size shown is the link's own (the length of the path it holds), a directory symlink sorts among the files,
  and `Enter` on one does nothing, since `try_cd` canonicalizes and going back through a symlink would need that
  reworked.
- `Options` is what a read needs to know — sort mode, `dirs_first`, `owners` — and travels as one value rather
  than a run of booleans, which is what `DirListing::from` and `App::new` take.
- `DirListing::options.owners` records what that listing *was read with*, not what the next read should do. The
  distinction matters: `toggle_owner` re-reads only when the listing never had owners, so hiding the column and
  showing it again is free however many times, while `try_cd` builds the next read's options from
  `App::needs()` so leaving with a column hidden doesn't make the next directory pay. Conflating the two made the
  third press of `u` re-read; there are tests with sentinel values for both fields. `App::needs()` is also where
  *ordering* by a field counts as needing it: sorting by owner or by time without reading them silently sorted
  by nothing, which is a bug worth not reintroducing.
- `-l` implies `--flat` and conflicts with `--tui`: a flat listing is the one with no way to ask later, while
  the interface has `u` and `t`, which read on demand. That is why it selects a mode rather than being ignored
  in one.
- `owners` and `times` are off by default and are the deferred-read work for #7. They matter for very different
  reasons, and the benchmark is the reason to keep them separate from the cheap fields: on ten thousand
  directories a listing went 43s to 30s, and 84% of the remaining time is `lgetxattr` at 1.37ms a call. The
  `statx` deferral is worth only ~4%, because CephFS's `readdir` prefetches inode metadata into the client cache
  so the stat is served locally; the xattrs are not prefetched, so each is a round trip. Dropping `rctime` — one
  of the three — is what bought the 12 seconds. Anything further has to come from issuing those calls
  concurrently rather than one at a time (#18).
- A directory needs a stat for *nothing else* than the owner: its size, count and time are the r-attrs and its
  kind comes from readdir via
  `DirEntry::file_type()`, which uses `d_type` and only falls back to a stat of its own on `DT_UNKNOWN` (CephFS
  does return it — measured). A file still needs its stat for size and time, so for files the only saving is the
  name lookup, which is why `DirEntry::from` takes `owners` separately from `stat` rather than deriving the
  owner from whatever stat happens to be in hand. That saving is small: measured on Rusty, where nsswitch is
  `files sss`, a real user resolves from SSSD's cache in ~18µs against 1.37ms for one `lgetxattr`, and only
  *absent* uids are slow (~2.3ms) — a real directory has none. So the owner's cost is its per-directory stat,
  not the lookup; don't reinstate the claim that it means an LDAP round trip.
  [tests/syscalls.rs](tests/syscalls.rs) pins the resulting cost model and prices `-l`.
- `rentries` has 1 subtracted because Ceph counts the directory itself. Specifically it is `rsubdirs` that
  includes the directory, since `rentries == rfiles + rsubdirs`; the non-recursive `ceph.dir.entries` has no such
  self-count.
- Off-Ceph, directory sizes are deliberately discarded (`e.size = None`) and `total_size` forced to 0, since
  non-Ceph dir sizes are meaningless here; a warning message is shown. The app is expected to run on non-Ceph
  paths, so keep that path working.
- `select_next`/`select_prev` intentionally clamp instead of using `ListState::select_next`, because item
  highlighting is applied manually per-`ListItem` in `to_listitem` rather than via ratatui's highlight style —
  scrolling past the end would drop the highlight for one frame.
- The highlighted entry per directory is remembered in `App::highlighted` (keyed by cwd) and restored by name,
  falling back to index. On the first arrival in a directory there is nothing to restore, so
  `select_first_entry()` skips `..` and lands on the first real entry; `select_first()` is the literal top of the
  list and stays bound to Home/`g`. Refresh and Backspace go through the remembered path, so they don't jump.
- The top border puts the path on the left and the totals on the right, and the path is truncated from its
  *start* by `truncate_start` so the deepest components stay visible while navigating. The path's room is
  whatever the totals leave, computed from `area.width` rather than left to ratatui, which would otherwise let
  the two titles collide.
- The bottom border carries a status area (sort field, direction, and `dirs first` when on) sharing the border
  with the right-aligned help hint, so it costs no vertical space. `SortField::label()` is the naming authority
  for both it and the CLI flags. The golden frame in [ui.rs](src/ui.rs) includes this line, so a change to the
  status wording means regenerating it.
- Horizontal scrolling is ours, not ratatui's: `List` has no horizontal offset (only `Paragraph` does, via
  `scroll((y, x))`). `render_list` draws the block itself, renders the rows into a `Buffer` as wide as they need,
  and copies a window of it into the block's inner area. That scrolls every column identically and keeps the
  border and its titles fixed, at the cost of one buffer per frame. The window is why `hscroll` is clamped during
  rendering rather than in the key handler.
- The interface sets almost no colors: backgrounds and text are left to the terminal, so it suits a light
  terminal as well as a dark one. `ERROR_STYLE` and `WARNING_STYLE` in [ui.rs](src/ui.rs) are the only named
  colors, and they are named for their meaning. Never reach for `Color::Rgb`/`Indexed` — those don't follow the
  user's terminal theme, and `the_interface_names_no_absolute_colors` renders a frame and fails if any appear.
  A user-selectable palette was tried and removed; see the history if it comes back up.
- The cursor row is shaded with a named background pair (`SELECTED_STYLE`: grey behind white) plus the `> `
  marker, and deliberately *not* bold — bold brightens the text on top of the color change, so the row reads as
  lightening rather than as marked. Grey rather than blue because `Color::Blue` is ANSI 4, already the darkest blue in
  the 16, so on a theme that renders it brightly there is nothing left to raise the contrast with; `DarkGray` is
  the only named color darker than it that won't merge into a dark terminal's own background. Going darker
  still means leaving
  the 16 for `Indexed`/`Rgb`, defensible only here, where both halves of the pair are named and so carry their
  own contrast.
  The white foreground is for light terminals: with an inherited foreground the text would be dark on a dark
  band there. Inheriting it instead is the one-line change that keeps the text from shifting color at all, at
  that cost.
  Under `NO_COLOR` the band would simply disappear, because crossterm drops color sequences and keeps
  attributes, leaving the `> ` marker as the only cue. `selected_style(colors_disabled())` therefore falls back
  to bold, which is why the gauge still has to remove `BOLD` from its own spans. Tests that assert the row's
  colors read them from `selected_style(colors_disabled())` rather than naming them, so `NO_COLOR=1 cargo test`
  passes too.
  Two alternatives were tried and rejected. Reverse video adapts to any terminal for free, but it inverts the
  gauges: a `█` reversed is drawn in the background color while a reversed empty cell paints the foreground
  across its full width, so a bar shows the wrong value. Marker-and-bold alone, with no background, reads as too
  subtle.
- The gauges opt out of the cursor row's styling: `bar` in `gauge()` pins the foreground to `Color::Reset` and
  removes `BOLD`, so a bar is the same color on every row and the column reads as one chart. Only the background
  behind it comes from the row, which is what keeps the shading continuous across the row. The percentage over
  the bar reverses the terminal's own pair rather than the row's, since reversing the cursor row's blue would put
  blue text on the bar — fine on a dark terminal, barely legible on a light one.
- Terminal light/dark detection is deliberately absent: neither ratatui nor crossterm can do it (checked in
  0.29/0.28), and the mechanism is an OSC 11 query, which needs raw-mode care, a timeout, and tmux passthrough.
  `terminal-colorsaurus` or `terminal-light` would be the crates if it is ever wanted -- but inheriting the
  terminal's colors makes detection unnecessary in the first place.
- `POPUP_TEXT_HEIGHT` in [ui.rs](src/ui.rs) is a fixed constant that popup scroll clamping and scrollbar state
  depend on; the popup is not sized to the terminal.
- The gauge in `gauge()` draws the percentage text *inside* the bar with inverted colors on the overlapping
  region, using ⅛-block characters for sub-cell precision.
- `format::Numbers` is the single switch between unit-scaled and exact rendering, and every size or count in
  either output mode goes through it. Row widths are measured per frame in `App::columns()` rather than
  hardcoded, because exact values are wider than any unit form; `SIZE_WIDTH`/`RENTRIES_WIDTH` are the minimums
  the unit forms fit in, which is what keeps the golden frame stable when the mode is off.
- Frame rows contain three-byte box and block characters, so a test that locates a column with `str::rfind` gets
  a byte offset, not a screen column. Convert with `line[..byte].chars().count()`.
- `ceph.dir.rctime` is the newest ctime anywhere in the subtree, *including* every directory's own: `chmod` on
  the directory alone moves it, and so does `chmod` on any subdirectory (measured, not assumed — an earlier
  version of the help text claimed it excluded the directory itself). It is a *propagated* value that starts at
  zero, though: creating a directory sets its ctime but not its rctime, so one that nothing has happened in
  since it was made reports zero and renders as the epoch. That is deliberate — the epoch is how a Unix
  timestamp says "never set" — and common: a freshly created tree of empty directories shows it everywhere,
  where `ls -l` shows a date, because `ls` shows the directory's *own* mtime and creation does set that.
  Creating something *inside* a directory sets the parent's rctime, since that changes the parent's own ctime,
  so it is only the leaves of a fresh tree that read zero. There is no third option to fall back on either:
  CephFS exposes no birth time (`ls --time=birth` reports `?`).
- Times shown are `rctime` for directories and `ctime` for files — deliberately ctime, not mtime; see the
  `after_help` text in [main.rs](src/main.rs).

## Tests

Unit tests are inline `#[cfg(test)]` modules, since this is a binary crate with no lib target and integration
tests can't import it. Anything needing the built binary goes in [tests/](tests/) and finds it via
`env!("CARGO_BIN_EXE_cephdu")`; scratch trees go under `env!("CARGO_TARGET_TMPDIR")`. There is no `tempfile`
dependency, deliberately — cargo already provides both paths.

- [src/app.rs](src/app.rs) — the index-mapping traps above. `DirListing::from_entries` is a `#[cfg(test)]`
  constructor that takes entries directly, so sorting and selection can be tested without a filesystem.
- [src/ui.rs](src/ui.rs) — full-frame snapshots via ratatui's `TestBackend`. The app under test is built from a
  synthetic listing *and* a faked `cwd`, so the frame doesn't depend on the filesystem or the path the tests run
  from; only the version line is excluded from the golden. Regenerate a golden by printing the frame lines with
  `{:?}` rather than hand-editing the box-drawing characters. Both optional columns don't fit in 80 columns, so
  that one test renders at 120.
- [tests/syscalls.rs](tests/syscalls.rs) — the cost of a listing in syscalls, via `strace -c`, for #7. It
  measures a *slope*: two trees differing by a known number of entries, counts subtracted, so the fixed cost of
  starting a process cancels and the numbers don't move with the libc or kernel underneath. Today a listing costs
  one `statx` per file, two `lgetxattr` per directory and no stat for a directory at all, and nothing else
  scales — that last part is the
  assertion worth keeping, since an accidental per-entry syscall is what makes a large directory slow. Skips
  without strace or ptrace permission. The numbers hold for the interface too: the cancellation check between
  entries is an atomic load, not a syscall.
- [tests/ceph.rs](tests/ceph.rs) — the only coverage of the xattrs themselves, and the only tests that can't run
  in CI. They skip (not fail) with a `SKIP` notice when no CephFS is available, so `cargo test` is green
  everywhere. The MDS updates recursive stats asynchronously, so assertions poll until the tree settles;
  expect these to take ~10s.

Two things make the assertions filesystem-independent and are easy to get wrong when adding tests: directory
sizes and counts are `None` off Ceph (so tests either use file sizes, which `stat` always reports, or key off the
binary's own "not a Ceph directory" warning), and timestamps in fixtures are mid-year and mid-day so no timezone
can shift the year that gets rendered.

## Conventions

Comment the non-obvious *why*, invariants, and traps — not what the code says. Rationale for a change belongs in
the commit message. Commit messages in this repo are prefixed with the module (`app:`, `ui:`, `readme:`).
