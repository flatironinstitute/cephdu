//! Counts the syscalls a listing costs, so that #7 can be reasoned about with
//! numbers rather than guesses.
//!
//! The measurement is a *slope*, not a total: two trees differing by a known number
//! of entries are traced and the counts subtracted, which cancels the fixed cost of
//! starting a process (loading libc, resolving a uid, opening /dev/tty) and leaves
//! only what each added entry costs. That is the part that matters for a directory
//! with a million files in it, and unlike a total it doesn't move with the libc or
//! kernel underneath.
//!
//! Needs strace and permission to ptrace, so it skips when either is missing.
//!
//! What this does *not* cover: `ls()` polls for Ctrl-C once per entry, and crossterm
//! only issues a syscall for that when its event source initialises, which needs a
//! controlling terminal. Under a test runner /dev/tty fails to open with ENXIO, so
//! the poll is free here and the interface may well pay one more syscall per entry
//! than these numbers show. Measuring it needs a real terminal:
//!
//!     strace -f -c -o /tmp/tui.trace cephdu --tui <dir>   # then press q

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cephdu");

type Counts = BTreeMap<String, u64>;

fn strace_works() -> bool {
    let out = Command::new("strace")
        .args(["-c", "-o", "/dev/null", "true"])
        .output();
    matches!(out, Ok(out) if out.status.success())
}

fn tree(tag: &str, files: usize, dirs: usize) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    for i in 0..files {
        fs::write(dir.join(format!("file{}", i)), b"0123456789").unwrap();
    }
    for i in 0..dirs {
        fs::create_dir(dir.join(format!("dir{}", i))).unwrap();
    }
    dir
}

/// Syscall name to number of calls, from `strace -c`.
fn counts(dir: &Path, extra: &[&str]) -> Counts {
    let summary = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "strace-{}{}.txt",
        dir.file_name().unwrap().to_str().unwrap(),
        extra.concat()
    ));

    let out = Command::new("strace")
        .arg("-f")
        .arg("-c")
        .arg("-o")
        .arg(&summary)
        .arg(BIN)
        // The parseable listing is the deterministic path, and it exercises the same
        // ls() that the interface uses.
        .args(["-p", dir.to_str().unwrap()])
        .args(extra)
        .output()
        .expect("failed to run strace");
    assert!(
        out.status.success(),
        "cephdu under strace failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let text = fs::read_to_string(&summary).unwrap();
    let mut counts = Counts::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // "% time seconds usecs/call calls [errors] syscall", plus a total line.
        if fields.len() < 5 || fields[0].parse::<f64>().is_err() {
            continue;
        }
        let name = *fields.last().unwrap();
        if name == "total" {
            continue;
        }
        if let Ok(calls) = fields[3].parse::<u64>() {
            *counts.entry(name.to_string()).or_default() += calls;
        }
    }
    assert!(!counts.is_empty(), "no syscalls parsed from {:?}", summary);
    counts
}

/// What each of the `added` extra entries cost, as whole calls. Fractional slopes
/// would mean the cost isn't per-entry at all, so they are reported rather than
/// rounded away.
fn slope(base: &Counts, bigger: &Counts, added: u64) -> BTreeMap<String, (i64, f64)> {
    let mut out = BTreeMap::new();
    for name in base.keys().chain(bigger.keys()) {
        let delta = *bigger.get(name).unwrap_or(&0) as i64 - *base.get(name).unwrap_or(&0) as i64;
        if delta != 0 {
            out.insert(name.clone(), (delta, delta as f64 / added as f64));
        }
    }
    out
}

fn report(what: &str, slope: &BTreeMap<String, (i64, f64)>, added: u64) {
    println!("\n{} ({} added):", what, added);
    println!("  {:<14} {:>8} {:>10}", "syscall", "delta", "per entry");
    for (name, (delta, per)) in slope {
        println!("  {:<14} {:>8} {:>10.2}", name, delta, per);
    }
}

/// The cost model of a listing, one syscall at a time. A file costs a stat, for its
/// size and time. A directory costs two xattrs -- its size and its count -- and no
/// stat at all: its kind comes from readdir, its owner is not read, and neither is
/// its recursive time, which was a third of the round trips. Nothing else may scale
/// with the number of entries; that is the assertion worth having, because an
/// accidental per-entry syscall is exactly what makes a large directory slow.
#[test]
fn a_listing_costs_one_stat_per_file_and_two_xattrs_per_dir() {
    if !strace_works() {
        eprintln!("SKIP: strace unavailable or ptrace not permitted");
        return;
    }

    const FILES: u64 = 100;
    const DIRS: u64 = 20;

    let base = tree("syscalls_base", 10, 2);
    let more_files = tree("syscalls_files", 10 + FILES as usize, 2);
    let more_dirs = tree("syscalls_dirs", 10, 2 + DIRS as usize);

    // Without the extras, and then with them: -l is what puts the stat back on a
    // directory and asks for the third xattr.
    for (flags, per_file, per_dir) in [
        (
            &[][..],
            [("statx", 1)].as_slice(),
            [("lgetxattr", 2)].as_slice(),
        ),
        (
            &["-l"][..],
            [("statx", 1)].as_slice(),
            [("statx", 1), ("lgetxattr", 3)].as_slice(),
        ),
    ] {
        let base = counts(&base, flags);
        let files = slope(&base, &counts(&more_files, flags), FILES);
        let dirs = slope(&base, &counts(&more_dirs, flags), DIRS);
        report(&format!("added files {:?}", flags), &files, FILES);
        report(&format!("added dirs {:?}", flags), &dirs, DIRS);

        for (what, measured, expected, added) in [
            ("file", &files, per_file, FILES),
            ("dir", &dirs, per_dir, DIRS),
        ] {
            let expected: BTreeMap<&str, i64> = expected.iter().copied().collect();
            for (name, (delta, _)) in measured {
                let want = expected.get(name.as_str()).copied().unwrap_or(0);
                assert_eq!(
                    *delta,
                    want * added as i64,
                    "with {:?}, each {} costs {} {} calls, expected {}",
                    flags,
                    what,
                    *delta as f64 / added as f64,
                    name,
                    want
                );
            }
            for (name, want) in &expected {
                assert!(
                    measured.contains_key(*name),
                    "with {:?}, no {} calls scale with the number of {}s; expected {} each",
                    flags,
                    name,
                    what,
                    want
                );
            }
        }
    }
}

/// readdir is batched, so it must not cost a syscall per entry either. Counted
/// separately because the batch size is the kernel's business, not ours.
#[test]
fn readdir_is_batched() {
    if !strace_works() {
        eprintln!("SKIP: strace unavailable or ptrace not permitted");
        return;
    }

    let small = counts(&tree("getdents_small", 10, 0), &[]);
    let large = counts(&tree("getdents_large", 1010, 0), &[]);

    let batches = |c: &Counts| *c.get("getdents64").unwrap_or(&0);
    let total = |c: &Counts| c.values().sum::<u64>();
    println!(
        "\ngetdents64: {} for 10 entries, {} for 1010",
        batches(&small),
        batches(&large)
    );
    println!(
        "a 1010-entry listing costs {} syscalls in total, {:.2} per entry",
        total(&large),
        total(&large) as f64 / 1010.0
    );

    // A thousand more entries must not cost a thousand more calls.
    assert!(
        batches(&large) < 100,
        "readdir is not batching: {} calls for 1010 entries",
        batches(&large)
    );
}
