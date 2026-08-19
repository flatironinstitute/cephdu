//! End-to-end tests of flat mode, driving the real binary.
//!
//! These run on any filesystem. The recursive values only exist on CephFS, so the
//! assertions here cover file sizes (which every filesystem reports) and treat
//! directory columns as present-or-absent, keyed off the binary's own warning.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cephdu");

struct Output {
    stdout: String,
    stderr: String,
    success: bool,
}

impl Output {
    /// Whether the directory under test is on CephFS, according to the binary.
    fn on_ceph(&self) -> bool {
        !self.stderr.contains("not a Ceph directory")
    }

    fn rows(&self) -> Vec<Vec<&str>> {
        self.stdout
            .lines()
            .map(|line| line.split('\t').collect())
            .collect()
    }
}

fn run(args: &[&str]) -> Output {
    let out = Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to run cephdu");

    Output {
        stdout: String::from_utf8(out.stdout).expect("stdout is not UTF-8"),
        stderr: String::from_utf8(out.stderr).expect("stderr is not UTF-8"),
        success: out.status.success(),
    }
}

/// A tree with distinct file sizes, so the size sort is a total order and the
/// expected row order holds whether or not directory sizes are available.
fn tree(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sub")).unwrap();

    fs::write(dir.join("big.bin"), vec![0u8; 1_500_000]).unwrap();
    fs::write(dir.join("mid.bin"), vec![0u8; 2_500]).unwrap();
    fs::write(dir.join("notes.txt"), vec![0u8; 120]).unwrap();
    fs::write(dir.join("sub").join("inner"), vec![0u8; 7]).unwrap();

    dir
}

fn path_arg(dir: &Path) -> String {
    dir.to_str().unwrap().to_string()
}

#[test]
fn parseable_rows_have_six_fields_and_stable_order() {
    let dir = tree("raw_rows");
    let out = run(&["--parseable", &path_arg(&dir)]);
    assert!(out.success, "{}", out.stderr);

    let rows = out.rows();
    assert_eq!(rows.len(), 4, "unexpected listing:\n{}", out.stdout);
    for row in &rows {
        assert_eq!(row.len(), 6, "wrong field count in {:?}", row);
    }

    // Descending by size; the directory is smallest either way.
    let names: Vec<&str> = rows.iter().map(|r| r[5]).collect();
    assert_eq!(names, ["big.bin", "mid.bin", "notes.txt", "sub/"]);

    // File sizes come from stat, so they are exact on any filesystem.
    let sizes: Vec<&str> = rows[..3].iter().map(|r| r[0]).collect();
    assert_eq!(sizes, ["1500000", "2500", "120"]);

    // Files have no recursive entry count.
    for row in &rows[..3] {
        assert_eq!(row[1], "-");
        row[2].parse::<u64>().expect("ctime is not a number");
    }

    for row in &rows {
        assert!(!row[3].is_empty(), "empty user in {:?}", row);
        assert!(!row[4].is_empty(), "empty group in {:?}", row);
    }

    if out.on_ceph() {
        rows[3][0]
            .parse::<u64>()
            .expect("dir rbytes is not a number");
        rows[3][1]
            .parse::<u64>()
            .expect("dir rentries is not a number");
    } else {
        assert_eq!(rows[3][0], "-", "dir size should be unavailable off Ceph");
        assert_eq!(rows[3][1], "-", "dir count should be unavailable off Ceph");
    }
}

/// The listing is this directory's contents, so "..' has no place in it.
#[test]
fn dotdot_is_not_listed() {
    let dir = tree("dotdot");
    for args in [
        vec!["--parseable", &path_arg(&dir)],
        vec!["--flat", &path_arg(&dir)],
    ] {
        let out = run(&args);
        assert!(!out.stdout.contains(".."), "{:?} listed '..'", args);
    }
}

/// A pipe can't display the interactive interface, so a flat listing is implied,
/// in the parseable format because the reader is more often a program. The child's
/// stdout is always a pipe here, so the bare invocation exercises it.
#[test]
fn piped_stdout_implies_parseable() {
    let dir = tree("piped");
    let implied = run(&[&path_arg(&dir)]);
    let explicit = run(&["--parseable", &path_arg(&dir)]);

    assert!(implied.success, "{}", implied.stderr);
    assert_eq!(implied.stdout, explicit.stdout);
    assert!(
        !implied.stdout.contains('\x1b'),
        "escape sequences leaked into a pipe"
    );
}

/// The parseable format must not depend on the terminal, so a parser sees the same
/// bytes no matter how it is invoked.
#[test]
fn the_two_formats_are_distinct() {
    let dir = tree("formats");
    let parseable = run(&["--parseable", &path_arg(&dir)]);
    let human = run(&["--flat", &path_arg(&dir)]);

    assert!(human.success, "{}", human.stderr);
    assert!(parseable.stdout.contains("1500000"), "{}", parseable.stdout);
    assert!(!parseable.stdout.contains("1.5 MB"), "{}", parseable.stdout);

    assert!(human.stdout.contains("1.5 MB"), "{}", human.stdout);
    assert!(!human.stdout.contains("1500000"), "{}", human.stdout);
    assert!(!human.stdout.contains('\t'), "human output has tabs");
}

/// The short and long forms mean the same thing, and -f alone means units.
#[test]
fn flag_spellings_agree() {
    let dir = tree("spellings");
    let path = path_arg(&dir);

    let parseable = run(&["--parseable", &path]);
    assert!(parseable.success, "{}", parseable.stderr);
    assert_eq!(run(&["-p", &path]).stdout, parseable.stdout);

    let human = run(&["-f", &path]);
    assert_eq!(human.stdout, run(&["--flat", &path]).stdout);
    assert!(human.stdout.contains("1.5 MB"), "{}", human.stdout);
}

#[test]
fn human_rows_are_aligned() {
    let dir = tree("aligned");
    let out = run(&["--flat", &path_arg(&dir)]);

    let name_columns: Vec<usize> = out
        .stdout
        .lines()
        .zip(["big.bin", "mid.bin", "notes.txt", "sub/"])
        .map(|(line, name)| line.rfind(name).unwrap())
        .collect();
    assert!(
        name_columns.windows(2).all(|w| w[0] == w[1]),
        "ragged name column:\n{}",
        out.stdout
    );
}

/// A script needs to see a failure, not a listing of somewhere else. This is the
/// one place flat mode deliberately differs from the interactive interface, which
/// falls back to the current directory.
#[test]
fn missing_path_is_an_error() {
    let dir = tree("missing").join("no-such-dir");
    let out = run(&["--parseable", &path_arg(&dir)]);

    assert!(!out.success, "exited successfully on a missing path");
    assert!(out.stdout.is_empty(), "printed a listing anyway");
    assert!(out.stderr.contains("no-such-dir"), "{}", out.stderr);
}

#[test]
fn a_file_argument_is_an_error() {
    let dir = tree("notadir");
    let out = run(&["--parseable", &path_arg(&dir.join("notes.txt"))]);

    assert!(!out.success, "exited successfully on a file");
    assert!(out.stdout.is_empty(), "printed a listing anyway");
}

#[test]
fn empty_directory_prints_nothing() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("empty");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let out = run(&["--parseable", &path_arg(&dir)]);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(out.stdout, "");
}

#[test]
fn tui_conflicts_with_flat_flags() {
    let dir = tree("conflict");
    for flag in ["--flat", "--parseable"] {
        let out = run(&["--tui", flag, &path_arg(&dir)]);
        assert!(!out.success, "--tui {} was accepted", flag);
    }
}
