# cephdu

A `ncdu`-like TUI for the Ceph File System. Uses the `rbytes` and `rentries` xattrs to display directory sizes and file counts without walking the file system.

## Usage

To build and run with a Rust toolchain [installed](https://www.rust-lang.org/tools/install), from inside the repo run:
```console
cargo run
```

To build an executable (dynamically linked by default):
```console
cargo build --release
```

To build a static executable:
```console
cargo build --release cargo build --target=x86_64-unknown-linux-musl
```

## Screenshot
![screenshot](./cephdu.png)

## License
MIT

## Author
[Lehman Garrison](https://github.com/lgarrison/)
