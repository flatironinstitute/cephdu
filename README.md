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

To build a static executable:
```console
cargo build --release cargo build --target=x86_64-unknown-linux-musl
```

## Usage
Simply run `cephdu` from the command line and an interactive terminal user interface (TUI) will be displayed. Navigate using the arrow keys and Enter. For a full list of keyboard shortcuts, press `?`.

The CLI accepts one optional argument, the initial directory:
```console
❯ cephdu --help
Display ceph space and file count (inode) usage in an interactive terminal

Usage: cephdu [OPTIONS] [PATH]

Arguments:
  [PATH]  Path to the directory to display

Options:
  -f, --flat        Print a flat text listing, with units, instead of the interactive interface
  -p, --parseable   Print a flat text listing of raw values, for parsing
      --tui         Use the interactive interface even if stdout is not a terminal
  -n, --name        Sort by name
  -s, --size        Sort by size
  -c, --count       Sort by file count
  -u, --owner       Sort by owner
  -t, --time        Sort by modification time
  -r, --reverse     Reverse the sort order
  -d, --dirs-first  List directories before files
  -e, --exact       Show sizes and counts in full instead of scaled to a unit
  -h, --help        Print help
```

### Sorting
Listings are sorted largest first unless one of `-n`, `-s`, `-c`, `-u` or `-t` is
given, which choose the field to sort on. They apply to both the interactive
interface and flat listings, and each field starts in the direction its sort key
uses in the interface: sizes, counts and times read most-first, names and owners
read ascending. In the interface, pressing the field's key reverses it from there.

`-r` reverses whichever order is in effect and combines with the field flags, so
`cephdu -r` reads smallest first and `cephdu -nr` reverses the name order.

`-d` groups directories ahead of files, whatever the field and direction, sorting
within each group by the usual rules. In the interactive interface, `d` toggles it.

`-e` prints sizes and counts in full instead of scaled to a unit, which is useful
when the rounding matters; `e` toggles it in the interface. The parseable format is
always exact, so `-e` has no effect there.

The bottom border of the interface names the sort in effect, as in `size ↓` for
largest first or `name ↑` for A to Z, and appends `· dirs first` while directories
are being grouped.

## Flat text mode
`cephdu --flat` prints the listing as text instead of drawing the interactive
interface, with the same units the interface uses:
```console
❯ cephdu --flat /mnt/ceph/users/$USER | head -3
 1.1 TB   48.2 K  Dec 11 09:15  alice:scc  data/
 2.1 GB        1  Dec 10 23:53  alice:scc  bigfile.h5
 4.1 KB       12  Dec 10 23:36  alice:scc  scripts/
```

`--parseable` prints the same listing as raw values instead, one row of six
tab-separated fields per entry — size in bytes, recursive entry count,
modification time in Unix seconds, user, group, and name — with `-` for anything
the filesystem doesn't provide (recursive values are only available on Ceph).
Directory names keep their trailing `/`, and `..` is not listed.

```console
❯ cephdu --parseable /mnt/ceph/users/$USER | head -3
1099511627776	48213	1765432100	alice	scc	data/
2147483648	1	1765400000	alice	scc	bigfile.h5
4096	12	1765399000	alice	scc	scripts/
```

The parseable format does not change based on whether stdout is a terminal, so it
stays parsable regardless of how it is invoked:
```console
❯ cephdu -p | awk -F'\t' '$1 > 1e12 {print $6}'    # directories over 1 TB
```

Because the interactive interface draws to stdout, a flat listing is printed
automatically when stdout is not a terminal, so `cephdu | ...` and `cephdu > file`
do something useful. That implied listing is parseable, on the grounds that
whatever is reading a pipe is more often a program than a person. Pass `--tui` to
override the detection, which is occasionally needed under tools that run a
program on a pseudo-terminal.

## Tests
```console
cargo test
```
The tests in `tests/ceph.rs` cover the recursive xattrs, so they need a CephFS
mount and skip themselves when there isn't one. They put a scratch tree in
`/mnt/ceph/users/$USER` by default; set `CEPHDU_TEST_DIR` to use somewhere else.

## Availability on Flatiron Institute Clusters

`cephdu` is available under the `fi-utils` module on the Flatiron Institute clusters, rusty and popeye, and is the preferred way to look at disk usage on ceph.

## License
MIT

## Author
[Lehman Garrison](https://github.com/lgarrison/)
