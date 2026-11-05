//! Golden tests. See "Testing" in docs/compiler.md.
//!
//! `tests/run/x.lcd` must run, print `x.out`, and exit 0.
//! `tests/fail/x.lcd` must fail under `lucid run`, print `x.err`, and exit 1.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LUCID: &str = env!("CARGO_BIN_EXE_lucid");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn lucid(dir: &Path, args: &[&str]) -> Output {
    Command::new(LUCID).args(args).current_dir(dir).output().expect("cannot start lucid")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .expect("cannot list directory")
        .map(|entry| entry.expect("cannot read entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .{extension} files in {}", dir.display());
    files
}

fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).expect("file name is not UTF-8")
}

/// Runs every test in `dir`, and reports all failures together.
fn golden(dir: &str, expected_extension: &str, check: fn(&Output, &str) -> Option<String>) {
    let dir = repo().join("tests").join(dir);
    let mut failures = Vec::new();
    for source in files(&dir, "lcd") {
        let name = file_name(&source);
        let expected = fs::read(source.with_extension(expected_extension))
            .unwrap_or_else(|e| panic!("{name}: no .{expected_extension} file: {e}"));
        let output = lucid(&dir, &["run", name]);
        if let Some(problem) = check(&output, &text(&expected)) {
            failures.push(format!("{name}: {problem}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn programs_run() {
    golden("run", "out", |output, expected| {
        let stdout = text(&output.stdout);
        let stderr = text(&output.stderr);
        (output.status.code() != Some(0) || stdout != expected || !stderr.is_empty()).then(|| {
            format!(
                "exit {:?}\nstdout: {stdout:?}\nexpected: {expected:?}\nstderr: {stderr}",
                output.status.code()
            )
        })
    });
}

#[test]
fn bad_programs_fail() {
    golden("fail", "err", |output, expected| {
        let stderr = text(&output.stderr);
        (output.status.code() != Some(1) || stderr != expected).then(|| {
            format!("exit {:?}\nstderr: {stderr:?}\nexpected: {expected:?}", output.status.code())
        })
    });
}

#[test]
fn examples_parse() {
    let dir = repo().join("examples");
    for example in files(&dir, "lcd") {
        let name = file_name(&example);
        let output = lucid(&dir, &["check", name]);
        assert!(output.status.success(), "{name}: {}", text(&output.stderr));
    }
}

#[test]
fn check_json_reports_on_stdout() {
    let dir = repo().join("tests").join("fail");
    let output = lucid(&dir, &["check", "--json", "expected_token.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stdout).starts_with("[{\"code\":\"E0201\",\"severity\":\"error\","));

    let dir = repo().join("tests").join("run");
    let output = lucid(&dir, &["check", "--json", "hello.lcd"]);
    assert_eq!((output.status.code(), text(&output.stdout).as_str()), (Some(0), "[]\n"));
}

#[test]
fn missing_file_is_reported() {
    let output = lucid(&repo(), &["run", "no_such_file.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).starts_with("no_such_file.lcd:1:1: error E0001: cannot read"));
}

#[test]
fn bad_usage_exits_2() {
    assert_eq!(lucid(&repo(), &[]).status.code(), Some(2));
    assert_eq!(lucid(&repo(), &["run", "--json", "x.lcd"]).status.code(), Some(2));
}

/// AGENTS.md: every compile error needs a stable code and a test. It must
/// also be listed in docs/agents.md.
#[test]
fn every_error_code_is_tested_and_documented() {
    let mut tested = String::from("E0001"); // missing_file_is_reported
    for err in files(&repo().join("tests").join("fail"), "err") {
        tested += &text(&fs::read(err).expect("cannot read .err file"));
    }
    let documented =
        fs::read_to_string(repo().join("docs").join("agents.md")).expect("cannot read docs");
    let mut codes = Vec::new();
    for source in files(&repo().join("compiler").join("src"), "rs") {
        let source = fs::read_to_string(source).expect("cannot read source");
        for piece in source.split("\"E").skip(1) {
            if piece.len() > 5
                && piece[..4].bytes().all(|b| b.is_ascii_digit())
                && piece[4..].starts_with('"')
            {
                codes.push(format!("E{}", &piece[..4]));
            }
        }
    }
    codes.sort();
    codes.dedup();
    assert!(codes.len() >= 10, "found only {codes:?}");
    let untested: Vec<_> = codes.iter().filter(|c| !tested.contains(*c)).collect();
    let undocumented: Vec<_> = codes.iter().filter(|c| !documented.contains(*c)).collect();
    assert!(untested.is_empty(), "no golden test for {untested:?}");
    assert!(undocumented.is_empty(), "not in docs/agents.md: {undocumented:?}");
}
