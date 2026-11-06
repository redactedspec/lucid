//! Golden tests. See "Testing" in docs/compiler.md.
//!
//! `tests/run/x.lcd` must run, print `x.out`, and exit 0.
//! `tests/fail/x.lcd` must fail under `lucid run`, print `x.err`, and exit 1.
//! `tests/trap/x.lcd` must start, then trap with `x.err`, and exit 101.
//! `tests/test/x.lcd` must print `x.out` under `lucid test`, and exit 1 if a
//! test failed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const LUCID: &str = env!("CARGO_BIN_EXE_lucid");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn lucid(dir: &Path, args: &[&str]) -> Output {
    Command::new(LUCID).args(args).current_dir(dir).output().expect("cannot start lucid")
}

/// With ownership optimizations off. See "Testing" in docs/compiler.md.
fn naive(dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(LUCID);
    command.args(args).current_dir(dir).env("LUCID_NAIVE", "1");
    command.output().expect("cannot start lucid")
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
        for run in [lucid, naive] {
            let output = run(&dir, &["run", name]);
            if let Some(problem) = check(&output, &text(&expected)) {
                failures.push(format!("{name}: {problem}"));
            }
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
fn traps_stop_the_program() {
    golden("trap", "err", |output, expected| {
        let stderr = text(&output.stderr);
        (output.status.code() != Some(101) || stderr != expected).then(|| {
            format!("exit {:?}\nstderr: {stderr:?}\nexpected: {expected:?}", output.status.code())
        })
    });
}

#[test]
fn test_blocks_report_results() {
    let dir = repo().join("tests").join("test");
    for source in files(&dir, "lcd") {
        let name = file_name(&source);
        let expected = text(&fs::read(source.with_extension("out")).expect("no .out file"));
        for run in [lucid, naive] {
            let output = run(&dir, &["test", name]);
            assert_eq!(text(&output.stdout), expected, "{name}: {}", text(&output.stderr));
            let code = if expected.contains("FAIL ") { 1 } else { 0 };
            assert_eq!(output.status.code(), Some(code), "{name}");
        }
    }
}

#[test]
fn test_blocks_report_json() {
    let dir = repo().join("tests").join("test");
    let report = text(&lucid(&dir, &["test", "--json", "passing.lcd"]).stdout);
    let wanted = "[{\"name\":\"one\",\"passed\":true,\"file\":\"passing.lcd\",\"line\":2,";
    assert!(report.starts_with(wanted), "{report}");
    let report = text(&lucid(&dir, &["test", "--json", "results.lcd"]).stdout);
    assert!(report.contains("\"passed\":false"), "{report}");
    assert!(report.contains("expect failed: 4 == 5"), "{report}");
}

/// `lucid run` drops `test` blocks unread, so these need `lucid check`.
#[test]
fn test_names_are_plain_and_new() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let source =
        "test \"a\"\n    expect true\ntest \"a\"\n    expect true\ntest \"{1}\"\n    expect true\n";
    fs::write(dir.join("test_names.lcd"), source).expect("cannot write test file");
    let output = lucid(dir, &["check", "test_names.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    let errors = text(&output.stderr);
    assert!(errors.contains("3:6: error E0329: there are two tests named \"a\""), "{errors}");
    assert!(errors.contains("5:6: error E0329: a `test` name cannot hold"), "{errors}");
    let output = lucid(dir, &["run", "test_names.lcd"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
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
fn agent_guide_runs_and_fits() {
    let source = fs::read_to_string(repo().join("docs/agent-guide.md")).expect("no guide");
    // About 4 bytes per token. The budget is in docs/vision.md.
    assert!(source.len() / 4 < 15_000, "guide is over the token budget");

    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let blocks: Vec<&str> = source
        .split(
            "```lucid
",
        )
        .skip(1)
        .collect();
    assert!(!blocks.is_empty());
    for (i, block) in blocks.iter().enumerate() {
        let code = block.split("```").next().expect("split gives one part");
        let name = format!("guide_{i}.lcd");
        fs::write(dir.join(&name), code).expect("cannot write test file");
        let output = lucid(dir, &["run", &name]);
        assert!(output.status.success(), "block {i}: {}", text(&output.stderr));
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

/// `tests/costs/x.lcd` must report `x.out` under `lucid check --costs`.
#[test]
fn costs_are_reported() {
    let dir = repo().join("tests").join("costs");
    for source in files(&dir, "lcd") {
        let name = file_name(&source);
        let expected = fs::read(source.with_extension("out")).expect("no .out file");
        let output = lucid(&dir, &["check", "--costs", name]);
        assert_eq!(output.status.code(), Some(0), "{name}: {}", text(&output.stderr));
        assert_eq!(text(&output.stdout), text(&expected), "{name}");
    }
    // A program that cannot compile has no costs to report, which is not zero.
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(scratch.join("unbuilt.lcd"), "use foo\nprint 1\n").expect("cannot write test file");
    let output = lucid(scratch, &["check", "--costs", "unbuilt.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("E0901"), "{}", text(&output.stderr));
    let output = lucid(&dir, &["check", "--json", "--costs", "shared.lcd"]);
    let report = text(&output.stdout);
    assert!(report.starts_with("[{\"kind\":\""), "{report}");
    assert!(report.contains("\"type\":\"list[int]\",\"file\":\"shared.lcd\",\"line\":"));
}

#[test]
fn unsafe_sites_are_reported() {
    let dir = repo().join("tests").join("run");
    let output = lucid(&dir, &["check", "--unsafe", "unsafe.lcd"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let report = text(&output.stdout);
    assert!(report.starts_with("unsafe.lcd:2:1: unsafe: extern\n"), "{report}");
    assert!(report.contains("unsafe.lcd:15:1: unsafe: block\n"), "{report}");
    let output = lucid(&dir, &["check", "--json", "--unsafe", "unsafe.lcd"]);
    let wanted = "[{\"kind\":\"extern\",\"file\":\"unsafe.lcd\",\"line\":2,\"column\":1}";
    assert!(text(&output.stdout).starts_with(wanted), "{}", text(&output.stdout));
    let output = lucid(&dir, &["check", "--unsafe", "hello.lcd"]);
    assert_eq!((output.status.code(), text(&output.stdout).as_str()), (Some(0), ""));
}

#[test]
fn missing_file_is_reported() {
    let output = lucid(&repo(), &["run", "no_such_file.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).starts_with("no_such_file.lcd:1:1: error E0001: cannot read"));
}

/// Generated, since the program is too long to keep in `tests/fail/`.
#[test]
fn huge_frame_is_rejected() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let parts = "{a}".repeat(70_000);
    let source = format!("task f(a: text)\n    print \"{parts}\"\nf(\"x\")\n");
    fs::write(dir.join("huge_text.lcd"), source).expect("cannot write test file");
    let output = lucid(dir, &["run", "huge_text.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).starts_with("huge_text.lcd:1:1: error E0902:"));
}

/// Each branch adds a hidden local and a merge block. E0902 must look at the
/// real frame, and a task of this size must still compile.
#[test]
fn many_text_branches_compile() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let branches = "    if a == \"x\"\n        print \"y\"\n".repeat(300);
    let source = format!("task f(a: text)\n{branches}f(\"x\")\n");
    fs::write(dir.join("branches.lcd"), source).expect("cannot write test file");
    let output = lucid(dir, &["run", "branches.lcd"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
}

/// Generated, since the program is too long to keep in `tests/fail/`.
#[test]
fn long_alias_chain_is_rejected() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let aliases: String = (0..200)
        .map(|n| {
            format!(
                "type A{n} = A{}
",
                n + 1
            )
        })
        .collect();
    fs::write(
        dir.join("aliases.lcd"),
        format!(
            "{aliases}type A200 = int
"
        ),
    )
    .expect("cannot write test file");
    let output = lucid(dir, &["run", "aliases.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("error E0205: aliases are nested too deep"));
}

#[test]
fn bad_usage_exits_2() {
    assert_eq!(lucid(&repo(), &[]).status.code(), Some(2));
    assert_eq!(lucid(&repo(), &["check", "--json"]).status.code(), Some(2));
    assert_eq!(lucid(&repo(), &["run", "--json", "x.lcd"]).status.code(), Some(2));
}

/// Runs `lucid` with its output pipes closed, and returns the exit code.
fn exit_with_closed_pipes(dir: &Path, args: &[&str]) -> Option<i32> {
    let mut child = Command::new(LUCID)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cannot start lucid");
    drop(child.stdout.take());
    drop(child.stderr.take());
    child.wait().expect("cannot wait for lucid").code()
}

#[test]
fn closed_output_does_not_crash() {
    let fail = repo().join("tests").join("fail");
    let run = repo().join("tests").join("run");
    assert_eq!(exit_with_closed_pipes(&fail, &["check", "--json", "expected_token.lcd"]), Some(1));
    assert_eq!(exit_with_closed_pipes(&fail, &["check", "expected_token.lcd"]), Some(1));
    assert_eq!(exit_with_closed_pipes(&run, &["run", "hello.lcd"]), Some(101));
}

#[test]
fn failed_stdout_write_traps_without_a_position() {
    let mut child = Command::new(LUCID)
        .args(["run", "hello.lcd"])
        .current_dir(repo().join("tests").join("run"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cannot start lucid");
    drop(child.stdout.take());
    let output = child.wait_with_output().expect("cannot wait for lucid");
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(text(&output.stderr), "trap: cannot write to stdout\n");
}

/// AGENTS.md: every compile error needs a stable code and a test. It must
/// also be listed in docs/agents.md.
#[test]
fn every_error_code_is_tested_and_documented() {
    // missing_file_is_reported, huge_frame_is_rejected, and
    // test_names_are_plain_and_new
    let mut tested = String::from("E0001 E0902 E0329");
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
