//! Tests of the CephFS-specific behavior: the recursive size and entry count that
//! the xattrs report, which is the whole reason this tool exists.
//!
//! These need a real CephFS mount, so they are skipped when there isn't one. Set
//! CEPHDU_TEST_DIR to choose where the scratch tree goes; otherwise
//! /mnt/ceph/users/$USER is used if it exists.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_cephdu");

/// The MDS updates recursive stats asynchronously, so a freshly written tree does
/// not report its final rbytes/rentries immediately.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

fn ceph_base() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CEPHDU_TEST_DIR") {
        let dir = PathBuf::from(dir);
        assert!(dir.is_dir(), "CEPHDU_TEST_DIR={:?} is not a directory", dir);
        return Some(dir);
    }

    let user = std::env::var("USER").ok()?;
    let dir = PathBuf::from("/mnt/ceph/users").join(user);
    dir.is_dir().then_some(dir)
}

/// Returns None when there is no CephFS to test against.
fn scratch(tag: &str) -> Option<PathBuf> {
    let dir = ceph_base()?.join(format!("cephdu-test-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    Some(dir)
}

fn write(path: PathBuf, size: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![0u8; size]).unwrap();
}

/// Rows of (size, rentries, name), in listed order.
fn listing(dir: &Path) -> Vec<(String, String, String)> {
    let out = Command::new(BIN)
        .args(["--parseable", dir.to_str().unwrap()])
        .output()
        .expect("failed to run cephdu");
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(out.status.success(), "cephdu failed: {}", stderr);
    assert!(
        !stderr.contains("not a Ceph directory"),
        "{:?} is not on CephFS; set CEPHDU_TEST_DIR to a Ceph path",
        dir
    );

    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            (f[0].to_string(), f[1].to_string(), f[5].to_string())
        })
        .collect()
}

/// Poll until the listing matches, since recursive stats settle asynchronously.
fn assert_listing_settles(dir: &Path, expected: &[(&str, &str, &str)]) {
    let expected: Vec<(String, String, String)> = expected
        .iter()
        .map(|(s, r, n)| (s.to_string(), r.to_string(), n.to_string()))
        .collect();

    let deadline = std::time::Instant::now() + SETTLE_TIMEOUT;
    let mut last = listing(dir);
    while last != expected && std::time::Instant::now() < deadline {
        std::thread::sleep(POLL_INTERVAL);
        last = listing(dir);
    }

    assert_eq!(
        last, expected,
        "recursive stats did not settle within {:?}",
        SETTLE_TIMEOUT
    );
}

/// Directory sizes are recursive, and entry counts exclude the directory itself.
#[test]
fn recursive_sizes_and_counts() {
    let Some(dir) = scratch("recursive") else {
        eprintln!("SKIP recursive_sizes_and_counts: no CephFS available");
        return;
    };

    // a/ holds 1000 + 2000 bytes directly and 3000 more in a subdirectory, so its
    // recursive size is 6000 across 4 entries (f1, f2, inner, f3).
    write(dir.join("a/f1"), 1000);
    write(dir.join("a/f2"), 2000);
    write(dir.join("a/inner/f3"), 3000);
    write(dir.join("b/f4"), 4000);
    write(dir.join("top"), 5000);

    assert_listing_settles(
        &dir,
        &[
            ("6000", "4", "a/"),
            ("5000", "-", "top"),
            ("4000", "1", "b/"),
        ],
    );

    fs::remove_dir_all(&dir).unwrap();
}

/// An empty directory reports itself as empty, not as containing itself.
#[test]
fn empty_directory_counts_zero() {
    let Some(dir) = scratch("empty") else {
        eprintln!("SKIP empty_directory_counts_zero: no CephFS available");
        return;
    };

    fs::create_dir(dir.join("nothing")).unwrap();
    assert_listing_settles(&dir, &[("0", "0", "nothing/")]);

    fs::remove_dir_all(&dir).unwrap();
}

/// Creating a directory sets its ctime but never its rctime, which the MDS only
/// fills in on the first rstat update. So a directory nothing has happened in since
/// it was made reports zero, which renders as the epoch -- idiomatic for a Unix
/// timestamp that was never set, and passed through rather than hidden.
#[test]
fn a_pristine_directory_reports_no_recursive_time() {
    let Some(dir) = scratch("pristine") else {
        eprintln!("SKIP a_pristine_directory_reports_no_recursive_time: no CephFS available");
        return;
    };

    fs::create_dir(dir.join("pristine")).unwrap();
    write(dir.join("touched/f"), 10);

    let out = Command::new(BIN)
        .args(["--parseable", "-l", dir.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();

    for line in stdout.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let (time, name) = (f[2], f[5]);
        if name == "pristine/" {
            assert_eq!(
                time, "0",
                "creation set an rctime after all, or zero was hidden: {}",
                line
            );
        } else {
            time.parse::<u64>()
                .unwrap_or_else(|_| panic!("{} has no time: {}", name, line));
        }
    }

    fs::remove_dir_all(&dir).unwrap();
}

/// Off Ceph these columns are blank; here they must carry real values. The time is
/// the exception: a directory's is a third of the xattr round trips, so it is read
/// only when asked for.
#[test]
fn human_output_shows_directory_sizes() {
    let Some(dir) = scratch("human") else {
        eprintln!("SKIP human_output_shows_directory_sizes: no CephFS available");
        return;
    };

    write(dir.join("data/big"), 2_000_000);
    assert_listing_settles(&dir, &[("2000000", "1", "data/")]);

    let human = |args: &[&str]| -> String {
        let out = Command::new(BIN)
            .args(args)
            .arg(dir.to_str().unwrap())
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };

    let plain = human(&["--flat"]);
    assert!(plain.contains("2.0 MB"), "{}", plain);
    assert!(plain.contains("data/"), "{}", plain);

    // -l reads the recursive time as well, so nothing is left unavailable.
    let long = human(&["--flat", "-l"]);
    assert!(long.contains("2.0 MB"), "{}", long);
    assert!(!long.contains('-'), "a column was unavailable: {}", long);

    fs::remove_dir_all(&dir).unwrap();
}
