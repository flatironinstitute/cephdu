# cephdu

A `ncdu`-like TUI for the Ceph File System. Uses the `rbytes` and `rentries` xattrs to display directory sizes and file counts without walking the file system.

[![Build](https://github.com/flatironinstitute/cephdu/actions/workflows/rust.yml/badge.svg)](https://github.com/flatironinstitute/cephdu/actions/workflows/rust.yml) [![Built With Ratatui](https://ratatui.rs/built-with-ratatui/badge.svg)](https://ratatui.rs/)

## Screenshot
![screenshot](./cephdu.png)

## Background

`ncdu` and similar applications that display disk usage work by crawling the filesystem (walking the directory tree) and recursively adding up file sizes and counts. On most filesystems, there's no alternative. However, the Ceph File System stores the recursive bytes and recursive counts (`ceph.dir.rbytes` and `ceph.dir.rentries`) as "extended attributes", available with the `getxattr` syscall. This means we can get disk usage info without a potentially expensive crawl.

## Installation
cephdu consists of a single binary, compiled from Rust. The binary can be downloaded from a release or built from source.

### Binaries
Binaries are attached to each GitHub Release: https://github.com/flatironinstitute/cephdu/releases

### From Source
To build and run with a Rust toolchain [installed](https://www.rust-lang.org/tools/install), from inside the repo run:
```console
cargo run
```

To build an executable (dynamically linked by default):
```console
cargo build --release
```

To build an executable that contains a default path to use if one is not given:
```console
CEPHDU_DEFAULT_DIR=/mnt/ceph/users/\$USER cargo build --release
```
The literal string `$USER` is substituted at runtime.

At runtime, the `CEPHDU_DEFAULT_DIR` environment variable overrides the baked-in
value, with the same `$USER` substitution — so a site can set the default per
cluster, in a modulefile for example, instead of per build. Setting it to the
empty string disables the baked-in default, too. Either default applies only when
no path is given and the current directory is not on Ceph; a Ceph current
directory always wins.

To build a static executable:
```console
cargo build --release cargo build --target=x86_64-unknown-linux-musl
```

## Usage
Simply run `cephdu` from the command line and an interactive terminal user interface (TUI) will be displayed. Navigate using the arrow keys and Enter. For a full list of keyboard shortcuts, press `?`. For [`flat mode`](#flat-text-mode), use `cephdu -f`.

The CLI accepts one optional argument, the initial directory. Without it, cephdu
starts in the current directory if that is on Ceph, and otherwise in
`$CEPHDU_DEFAULT_DIR` if it is set (see [Installation](#installation) for the
resolution order):
```console
❯ cephdu -h
Display ceph space and file count (inode) usage in an interactive terminal

Usage: cephdu [OPTIONS] [PATH]

Arguments:
  [PATH]  Path to the directory to display

Options:
  -f, --flat        Print a flat text listing, with units, instead of the interactive interface
  -p, --parseable   Print a flat text listing of raw values for parsing
      --tui         Use the interactive interface even if stdout is not a terminal
  -n, --name        Sort by name
  -s, --size        Sort by size
  -c, --count       Sort by file count
  -u, --owner       Sort by owner
  -t, --time        Sort by change time
  -r, --reverse     Reverse the sort order
  -d, --dirs-first  List directories before files
  -e, --exact       Show sizes and counts in full instead of scaled to a unit
  -l, --long        Show the owner and directory times, which cost extra syscalls. Implies -f
  -i, --info        Open the interactive interface with the info panel shown
  -h, --help        Print help
```

## Flat text mode
`cephdu -f/--flat` prints the listing as text instead of drawing the interactive
interface, with the same units the interface uses:
```console
❯ cephdu --flat /mnt/ceph/users/$USER | head -3
88.8 TB   95.6 K             -  Derivatives/
23.3 TB  232.8 K             -  AbacusSummit/
 1.0 GB        -  Mar 23 16:13  file.bin
```

Note that directory rctime is not displayed by default to avoid potentially expensive syscalls. Use `-l` to display.

`--parseable` prints the same listing as raw values instead, one row of six
tab-separated fields per entry — size in bytes, recursive entry count,
change time in Unix seconds, user, group, and name — with `-` for anything
the filesystem doesn't provide (recursive values are only available on Ceph).
Directory names keep their trailing `/`, and `..` is not listed. Symlinks are *not*
marked here: a filename may contain `@`, so a parser could not tell a mark from a
character. Only the two human formats mark them.

```console
❯ cephdu --parseable /mnt/ceph/users/$USER | head -3
88830194877352  95636   -       -       -       Derivatives/
23303634816992  232846  -       -       -       AbacusSummit/
1048576000      -       1774296835      -       -       test1.bin
```

Parseable is the default when stdout is not a TTY.

### Sorting
Listings are sorted by bytes (`rbytes` for directories) unless one of `-n`, `-s`, `-c`, `-u` or `-t` is
given, which choose the field to sort on. They apply to both the interactive
interface and flat listings, and each field starts in the direction its sort key
uses in the interface: sizes, counts and change times read most-first, names and
owners read ascending. In the interface, pressing the field's key reverses it from
there.

`-r` reverses whichever order is in effect and combines with the field flags, so
`cephdu -r` reads smallest first and `cephdu -nr` reverses the name order.

`-d` groups directories ahead of files, whatever the field and direction, sorting
within each group by the usual rules. In the interactive interface, `d` toggles it.

`-e` prints sizes and counts in full instead of scaled to a unit, which is useful
when the rounding matters; `e` toggles it in the interface. The parseable format is
always exact, so `-e` has no effect there.

### Colors
The interface mostly uses terminal colors, so it should adapt to dark and light themes. The program respects `NO_COLOR`. Flat mode never uses colors.

### Symlinks
Symlinks are marked the way `ls -F` marks them, with a trailing `@` beside the `/`
that directories already carry. The size shown is the symlink's own — the length of
the path it holds — rather than its target's, and a symlink to a directory is listed
among the files.


### syscalls
In a directory listing, we query the following info by default:
- For each directory: `rbytes` and `rentries`. Two `getxattr` syscalls.
- For each file: one `stat`, yielding the file size, ctime, etc.

If the user requests displaying or sorting by ctime or owner, then we issue an additional syscall (`getxattr` or `stat`) for each directory entry.

### Directory info
Every number in the listing is recursive, so the non-recursive ones live in a
panel that `i` toggles under the listing:

```
┣━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┫
┃            Recursive size:  154.0 TB                                     ┃
┃         Recursive entries:  4.4 M (4.1 M files, 325.9 K dirs)            ┃
┃  Recursive mean file size:  37.5 MB                                      ┃
┃ Recursive entries per dir:  13.5                                         ┃
┃        Size at this level:  1.1 GB (0.0% of recursive)                   ┃
┃     Entries at this level:  230 (10 files, 220 dirs, 0.0% of recursive)  ┃
┗ size ↓ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ Press ? for help ┛
```

`-i` opens the interface with the panel shown.

"At this level" means the files directly in this directory. Off Ceph the recursive lines are absent. The recursive lines
are a consistent snapshot, which can disagree slightly with the border's totals
while Ceph is still propagating recent changes; each number is individually
true.

## Tests
```console
cargo test
```
The tests in `tests/ceph.rs` cover the recursive xattrs, so they need a CephFS
mount and skip themselves when there isn't one. They put a scratch tree in
`/mnt/ceph/users/$USER` by default; set `CEPHDU_TEST_DIR` to use somewhere else.

`tests/syscalls.rs` counts how many syscalls a listing costs, using `strace`, and
skips without it. `cargo test -- --nocapture` prints the table.

## Availability on Flatiron Institute Clusters

`cephdu` is available under the `fi-utils` module on the Flatiron Institute clusters, rusty and popeye, and is the preferred way to look at disk usage on ceph.

## License
MIT

## Author
[Lehman Garrison](https://github.com/lgarrison/)
