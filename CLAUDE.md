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
cargo test -- --nocapture        # needed to see the SKIP notices from tests/ceph.rs
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
- [app.rs](src/app.rs) — all state. `App` holds the cwd and one `DirListing`; `DirListing` holds `Vec<DirEntry>`
  plus a ratatui `ListState`, sort mode, and aggregate `ListingStats`. `DirEntry::from` is where one stat +
  three xattr calls happen per entry.
- [format.rs](src/format.rs) — the base-1000 size/count units and the `ls -l`-style time format, shared by both
  output modes. Pure functions, so this is where formatting tests live.
- [navigation.rs](src/navigation.rs) — `App::handle_key`, plus the `HELP` table that is both the key mapping's
  documentation and the source of the `?` popup text. Add a keybinding *and* its `HELP` row together.
- [ui.rs](src/ui.rs) — pure rendering from `&App`; no state changes except ratatui's own `ListState`.
- [flat.rs](src/flat.rs) — the non-interactive renderer, which takes a `DirListing` and a `Write`.
- [popup.rs](src/popup.rs) — scrollable modal used only for help so far.

Control flow for the TUI is a blocking loop in `run_app`: draw, block on `event::read()`, dispatch to
`handle_key`. There is no tick, no async runtime, and no redraw except after a key press. `cd` re-reads the whole
directory synchronously, so a large directory blocks; `ls()` works around this by polling for Ctrl-C
mid-iteration, which [app.rs](src/app.rs) itself flags as the wrong fix.

### Output modes

`main` picks the mode before building an `App`: flat mode goes straight from `DirListing::from` to
`flat::write_listing`, never constructing one. The TUI draws to stdout, so it can't work when stdout is
redirected — hence flat mode is implied when stdout isn't a terminal, with `--tui` as the escape hatch for
pty-wrapping tools. Two properties of flat mode are deliberate and worth preserving:

- `--flat` (`-f`) selects the format with units and `--parseable` (`-p`) the raw one. Neither varies with the
  terminal, so parsers see the same bytes however the tool is invoked. This is also why flat mode has no column-visibility flags: the TUI's `u`/`t` toggles exist
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
- The bottom border carries a status area (sort field, direction, and `dirs first` when on) sharing the border
  with the right-aligned help hint, so it costs no vertical space. `SortField::label()` is the naming authority
  for both it and the CLI flags. The golden frame in [ui.rs](src/ui.rs) includes this line, so a change to the
  status wording means regenerating it.
- `POPUP_TEXT_HEIGHT` in [ui.rs](src/ui.rs) is a fixed constant that popup scroll clamping and scrollbar state
  depend on; the popup is not sized to the terminal.
- The gauge in `gauge()` draws the percentage text *inside* the bar with inverted colors on the overlapping
  region, using ⅛-block characters for sub-cell precision. Sizes/counts use base-1000 units.
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
