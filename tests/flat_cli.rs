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

/// Names deliberately in the reverse of size order, so a name sort and a size sort
/// cannot be mistaken for one another.
fn sort_tree(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("apple.bin"), vec![0u8; 1_000]).unwrap();
    fs::write(dir.join("middle.bin"), vec![0u8; 2_000]).unwrap();
    fs::write(dir.join("zebra.bin"), vec![0u8; 3_000]).unwrap();

    dir
}

/// A symlink to a file, one to a directory, a broken one, and -- the case that rules
/// out marking names in the parseable stream -- a symlink whose own name contains @.
fn link_tree(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("realdir")).unwrap();
    fs::write(dir.join("realfile"), vec![0u8; 5_000]).unwrap();

    let link = std::os::unix::fs::symlink;
    link("realfile", dir.join("link-to-file")).unwrap();
    link("realdir", dir.join("link-to-dir")).unwrap();
    link("/nonexistent/target", dir.join("broken-link")).unwrap();
    link("realfile", dir.join("weird@name")).unwrap();

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

/// -e prints values in full. The parseable format is already exact, so it has
/// nothing to switch there.
#[test]
fn exact_shows_values_in_full() {
    let dir = tree("exact");
    let path = path_arg(&dir);

    let units = run(&["-f", &path]);
    let exact = run(&["-f", "-e", &path]);
    assert!(exact.success, "{}", exact.stderr);

    assert!(units.stdout.contains("1.5 MB"), "{}", units.stdout);
    assert!(!units.stdout.contains("1500000"), "{}", units.stdout);

    assert!(exact.stdout.contains("1500000"), "{}", exact.stdout);
    assert!(!exact.stdout.contains("1.5 MB"), "{}", exact.stdout);
    assert!(
        !exact.stdout.contains('\t'),
        "fell back to the parseable format"
    );

    assert_eq!(
        exact.stdout,
        run(&["--flat", "--exact", &path]).stdout,
        "long forms disagree"
    );
    assert_eq!(
        run(&["-p", "-e", &path]).stdout,
        run(&["-p", &path]).stdout,
        "-e changed the parseable format, which is already exact"
    );

    let name_columns: Vec<usize> = exact
        .stdout
        .lines()
        .zip(["big.bin", "mid.bin", "notes.txt", "sub/"])
        .map(|(line, name)| line.rfind(name).unwrap())
        .collect();
    assert!(
        name_columns.windows(2).all(|w| w[0] == w[1]),
        "ragged name column:\n{}",
        exact.stdout
    );
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

/// The sort flags apply to flat mode, in the same directions as the interface keys.
#[test]
fn sort_flags_choose_the_order() {
    let dir = sort_tree("sort_order");
    let path = path_arg(&dir);

    let names = |flag: &str| -> Vec<String> {
        let args: Vec<&str> = if flag.is_empty() {
            vec!["-p", &path]
        } else {
            vec!["-p", flag, &path]
        };
        let out = run(&args);
        assert!(out.success, "{:?}: {}", args, out.stderr);
        out.stdout
            .lines()
            .map(|l| l.rsplit('\t').next().unwrap().to_string())
            .collect()
    };

    let by_name = ["apple.bin", "middle.bin", "zebra.bin"];
    let by_size = ["zebra.bin", "middle.bin", "apple.bin"];

    assert_eq!(names(""), by_size, "the default is largest first");
    assert_eq!(names("-n"), by_name);
    assert_eq!(names("-s"), by_size);
    assert_eq!(names("--name"), by_name);
    assert_eq!(names("--size"), by_size);
}

/// Every flag is accepted in both output modes and lists the same entries. Off Ceph
/// the counts and times all tie, so the orders they produce can't be told apart
/// here; tests/ceph.rs covers the counts where they have values.
#[test]
fn every_sort_flag_is_accepted() {
    let dir = sort_tree("sort_accepted");
    let path = path_arg(&dir);

    for flag in [
        "-n", "-s", "-c", "-u", "-t", "--name", "--size", "--count", "--owner", "--time",
    ] {
        for mode in ["-p", "-f"] {
            let out = run(&[mode, flag, &path]);
            assert!(out.success, "{} {}: {}", mode, flag, out.stderr);

            let mut listed: Vec<&str> = out
                .stdout
                .lines()
                .map(|l| l.split_whitespace().last().unwrap())
                .collect();
            listed.sort_unstable();
            assert_eq!(
                listed,
                ["apple.bin", "middle.bin", "zebra.bin"],
                "{} {}",
                mode,
                flag
            );
        }
    }
}

/// -r flips whichever order is in effect, and composes with the field flags rather
/// than being one of them.
#[test]
fn reverse_flips_the_order() {
    let dir = sort_tree("sort_reverse");
    let path = path_arg(&dir);

    let names = |args: &[&str]| -> Vec<String> {
        let mut argv = vec!["-p"];
        argv.extend_from_slice(args);
        argv.push(&path);
        let out = run(&argv);
        assert!(out.success, "{:?}: {}", args, out.stderr);
        out.stdout
            .lines()
            .map(|l| l.rsplit('\t').next().unwrap().to_string())
            .collect()
    };

    let ascending = ["apple.bin", "middle.bin", "zebra.bin"];
    let descending = ["zebra.bin", "middle.bin", "apple.bin"];

    // Bare -r reverses the default, which is by size.
    assert_eq!(names(&[]), descending);
    assert_eq!(names(&["-r"]), ascending);
    assert_eq!(names(&["-s", "-r"]), ascending);

    // Names run the other way, so reversing them descends.
    assert_eq!(names(&["-n"]), ascending);
    assert_eq!(names(&["-n", "-r"]), descending);

    assert_eq!(names(&["--name", "--reverse"]), descending);
    assert_eq!(names(&["-nr"]), descending, "combined short form");
}

/// -d groups directories ahead of files whatever the direction. Off Ceph this is
/// easy to see, because a directory reports no size and would otherwise sort last.
#[test]
fn dirs_first_groups_directories() {
    let dir = tree("dirs_first");
    let path = path_arg(&dir);

    let names = |args: &[&str]| -> Vec<String> {
        let mut argv = vec!["-p"];
        argv.extend_from_slice(args);
        argv.push(&path);
        let out = run(&argv);
        assert!(out.success, "{:?}: {}", args, out.stderr);
        out.stdout
            .lines()
            .map(|l| l.rsplit('\t').next().unwrap().to_string())
            .collect()
    };

    assert_eq!(names(&[]), ["big.bin", "mid.bin", "notes.txt", "sub/"]);
    assert_eq!(names(&["-d"]), ["sub/", "big.bin", "mid.bin", "notes.txt"]);
    assert_eq!(
        names(&["--dirs-first"]),
        ["sub/", "big.bin", "mid.bin", "notes.txt"]
    );

    // Reversing turns the files around but leaves the directory on top.
    assert_eq!(
        names(&["-d", "-r"]),
        ["sub/", "notes.txt", "mid.bin", "big.bin"]
    );
    assert_eq!(
        names(&["-d", "-n"]),
        ["sub/", "big.bin", "mid.bin", "notes.txt"]
    );
}

#[test]
fn sort_flags_are_mutually_exclusive() {
    let dir = sort_tree("sort_exclusive");
    let out = run(&["-p", "-n", "-s", &path_arg(&dir)]);

    assert!(!out.success, "two sort flags were accepted");
    assert!(out.stdout.is_empty(), "printed a listing anyway");
}

/// Symlinks are marked for a person and left alone for a parser.
#[test]
fn symlinks_are_marked_in_the_human_format_only() {
    let dir = link_tree("links");
    let path = path_arg(&dir);

    let human = run(&["-f", &path]);
    assert!(human.success, "{}", human.stderr);
    for name in [
        "link-to-file@",
        "link-to-dir@",
        "broken-link@",
        "weird@name@",
    ] {
        assert!(
            human.stdout.contains(name),
            "{} missing:\n{}",
            name,
            human.stdout
        );
    }
    // Regular entries keep their names, and directories their slash. The name ends
    // the line, so an unmarked one is followed by the newline.
    assert!(human.stdout.contains("realfile\n"), "{}", human.stdout);
    assert!(human.stdout.contains("realdir/"), "{}", human.stdout);
    assert!(!human.stdout.contains("realfile@"), "{}", human.stdout);

    let parseable = run(&["-p", &path]);
    let names: Vec<&str> = parseable
        .stdout
        .lines()
        .map(|l| l.rsplit('\t').next().unwrap())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        [
            "broken-link",
            "link-to-dir",
            "link-to-file",
            "realdir/",
            "realfile",
            "weird@name",
        ],
        "the parseable stream marked a name"
    );
}

/// The owner costs a stat for every directory and a name lookup for every distinct
/// owner, so it is read only when asked for.
#[test]
fn the_owner_is_read_only_with_the_flag() {
    let dir = tree("owner");
    let path = path_arg(&dir);

    let plain = run(&["-p", &path]);
    assert!(plain.success, "{}", plain.stderr);
    for row in plain.rows() {
        assert_eq!(row[3], "-", "an owner was read without -l: {:?}", row);
        assert_eq!(row[4], "-", "a group was read without -l: {:?}", row);
    }

    let long = run(&["-p", "-l", &path]);
    assert!(long.success, "{}", long.stderr);
    for row in long.rows() {
        assert_ne!(row[3], "-", "-l did not read the owner: {:?}", row);
        assert_ne!(row[4], "-", "-l did not read the group: {:?}", row);
    }
    assert_eq!(
        long.stdout,
        run(&["-p", "--long", &path]).stdout,
        "long forms disagree"
    );

    // -l implies --flat: into a pipe, where the default is parseable, it still gives
    // the human format.
    assert_eq!(
        run(&["-l", &path]).stdout,
        run(&["-f", "-l", &path]).stdout,
        "-l did not imply --flat"
    );
    assert!(
        !run(&["-l", &path]).stdout.contains('\t'),
        "-l gave the parseable format"
    );

    // The human format drops the column rather than filling it with placeholders.
    let human = run(&["-f", &path]);
    let human_long = run(&["-f", "-l", &path]);
    let first = |o: &Output| o.stdout.lines().next().unwrap().len();
    assert!(
        first(&human) < first(&human_long),
        "the column was not dropped:\n{}",
        human.stdout
    );
    // Not a bare ':' check: a ctime in the current year contains one.
    let owner = long.rows()[0][3].to_string();
    assert!(
        !human.stdout.contains(&owner),
        "{} appeared without -l:\n{}",
        owner,
        human.stdout
    );
    assert!(human_long.stdout.contains(&owner), "{}", human_long.stdout);
}

/// A file's time comes from the stat it needs anyway, so it is there either way. A
/// directory's is an xattr round trip, and tests/ceph.rs covers it where there is one
/// to find.
#[test]
fn a_file_keeps_its_time_without_the_flag() {
    let dir = tree("times");
    let path = path_arg(&dir);

    for args in [vec!["-p", &path], vec!["-p", "-l", &path]] {
        let out = run(&args);
        assert!(out.success, "{}", out.stderr);
        for row in out.rows().iter().filter(|r| !r[5].ends_with('/')) {
            row[2]
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("{:?}: file has no time in {:?}", args, row));
        }
    }
}

#[test]
fn tui_conflicts_with_flat_flags() {
    let dir = tree("conflict");
    for flag in ["--flat", "--parseable", "--long"] {
        let out = run(&["--tui", flag, &path_arg(&dir)]);
        assert!(!out.success, "--tui {} was accepted", flag);
    }
}
