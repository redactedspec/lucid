//! Golden tests. See "Testing" in docs/compiler.md.
//!
//! `tests/run/x.lcd` must run, print `x.out`, and exit 0.
//! `tests/fail/x.lcd` must fail under `lucid run`, print `x.err`, and exit 1.
//! `tests/trap/x.lcd` must start, then trap with `x.err`, and exit 101.
//! `tests/test/x.lcd` must print `x.out` under `lucid test`, and exit 1 if a
//! test failed.
//! `tests/package/x/` must print `expected.out` and exit 0, or `expected.err`
//! and exit 1, under `lucid run x`. A directory with neither is a dependency.

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
fn packages_build_or_report() {
    let dir = repo().join("tests").join("package");
    let mut failures = Vec::new();
    let mut count = 0;
    for entry in fs::read_dir(&dir).expect("cannot list directory") {
        let package = entry.expect("cannot read entry").path();
        let name = file_name(&package);
        let good = fs::read(package.join("expected.out"));
        let bad = fs::read(package.join("expected.err"));
        for run in [lucid, naive] {
            let output = run(&dir, &["run", name]);
            let (code, found, expected) = match (&good, &bad) {
                (Ok(expected), _) => (0, text(&output.stdout), expected),
                (_, Ok(expected)) => (1, text(&output.stderr), expected),
                _ => continue,
            };
            count += 1;
            if output.status.code() != Some(code) || found != text(expected) {
                failures.push(format!("{name}: {found}{}", text(&output.stderr)));
            }
        }
    }
    assert!(count > 0, "no packages");
    assert!(
        failures.is_empty(),
        "
{}",
        failures.join(
            "
"
        )
    );
}

/// A command takes a package, by path or from any directory inside it.
#[test]
fn packages_work_from_inside() {
    let dir = repo().join("tests").join("package").join("app");
    let expected = fs::read(dir.join("expected.out")).expect("no expected output");
    for from in [dir.clone(), dir.join("src")] {
        let output = lucid(&from, &["run"]);
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), text(&expected));
        assert!(lucid(&from, &["check"]).status.success());
    }
    let main = lucid(&dir.join("src"), &["check", "--json", "main.lcd"]);
    assert_eq!(
        text(&main.stdout),
        "[]
",
        "{}",
        text(&main.stderr)
    );
    let package = dir.join("..");
    let library = lucid(&package, &["run", "greet"]);
    assert_eq!(library.status.code(), Some(1));
    assert!(text(&library.stderr).contains("error E0339: `greet/src/lib.lcd` is a library"));
    let library = lucid(&package, &["run", "greet/src/lib.lcd"]);
    assert_eq!(library.status.code(), Some(1));
    let statement = lucid(&package, &["check", "lib_statement/src/lib.lcd"]);
    assert_eq!(statement.status.code(), Some(1));
    assert!(text(&statement.stderr).contains("E0336"), "{}", text(&statement.stderr));
    let statement = lucid(&package, &["check", "lib_statement"]);
    assert_eq!(statement.status.code(), Some(1));
    assert!(text(&statement.stderr).contains("E0336"), "{}", text(&statement.stderr));
    let missing = lucid(&repo(), &["run"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(text(&missing.stderr).contains("E0001"));
}

/// `lucid test` on a package reaches modules that nothing imports, and a test
/// in a library sees its dependencies.
#[test]
fn package_tests_run_in_every_file() {
    let dir = repo().join("tests").join("package");
    let output = lucid(&dir, &["test", "app"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "app/src/helper.lcd
ok twice
1 tests, 0 failed
"
    );
    let output = lucid(&dir, &["test", "--json", "greet"]);
    assert!(text(&output.stdout).contains("\"file\":\"greet/src/lib.lcd\""));
    assert!(text(&output.stdout).starts_with('['), "{}", text(&output.stdout));
    // `bad_later` has a good test first and a type error after it.
    let output = lucid(&dir, &["test", "bad_later"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "", "a test ran before all files compiled");
    assert!(text(&output.stderr).contains("zz.lcd:1:"), "{}", text(&output.stderr));
    let output = lucid(&dir, &["test", "both_entries"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("E0339"));
}

/// `check --unsafe` must list sites in dependencies: `docs/security.md` says so.
#[test]
fn unsafe_sites_include_dependencies() {
    let dir = repo().join("tests").join("package");
    let output = lucid(&dir, &["check", "--unsafe", "uses_unsafe"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let found = text(&output.stdout);
    assert!(found.contains("uses_unsafe/../raw/src/lib.lcd:"), "{found}");
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
    fs::write(scratch.join("unbuilt.lcd"), "use std.foo\nprint 1\n")
        .expect("cannot write test file");
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
    // An unbuilt feature does not hide a site, as it does not fail `check`.
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let source = "use std.foo\n# It runs nothing.\nunsafe\n    print 1\n";
    fs::write(scratch.join("unbuilt_unsafe.lcd"), source).expect("cannot write test file");
    let output = lucid(scratch, &["check", "--unsafe", "unbuilt_unsafe.lcd"]);
    assert_eq!(text(&output.stdout), "unbuilt_unsafe.lcd:3:1: unsafe: block\n");
    // A missing foreign task needs no compiled code to find.
    let dir = repo().join("tests").join("fail");
    let output = lucid(&dir, &["check", "extern_missing.lcd"]);
    assert!(text(&output.stderr).contains("E0904"), "{}", text(&output.stderr));
}

/// E0902 shows only in the child, and must still be data.
#[test]
fn test_compile_errors_stay_json() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let parts = "{a}".repeat(70_000);
    let source = format!("task f(a: text)\n    print \"{parts}\"\ntest \"x\"\n    f(\"x\")\n");
    fs::write(dir.join("huge_test.lcd"), source).expect("cannot write test file");
    let output = lucid(dir, &["test", "--json", "huge_test.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stdout).starts_with("[{\"code\":\"E0902\""), "{}", text(&output.stdout));
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

/// What follows the path of `lucid run` goes to the program, unread.
#[test]
fn run_passes_arguments() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::write(dir.join("args.lcd"), "use std.args\nprint args.all()\n").expect("cannot write");
    let output = lucid(dir, &["run", "args.lcd", "add", "--json", "a b"]);
    assert_eq!(text(&output.stdout), "[\"add\", \"--json\", \"a b\"]\n");
}

/// Each run reads what the last one saved. Under `lucid run`, then as a
/// built program, which is the exit of M5.
#[test]
fn todo_example_works() {
    for built in [false, true] {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("todo_{built}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("cannot make directory");
        fs::copy(repo().join("examples/todo.lcd"), dir.join("todo.lcd")).expect("cannot copy");
        if built {
            let output = lucid(&dir, &["build", "todo.lcd"]);
            assert!(output.status.success(), "{}", text(&output.stderr));
            fs::remove_file(dir.join("todo.lcd")).expect("cannot remove");
        }
        todo_works(&dir, built);
    }
}

fn todo_works(dir: &Path, built: bool) {
    let run = |args: &[&str]| {
        let output = if built {
            Command::new(dir.join("todo.exe")).args(args).current_dir(dir).output()
        } else {
            let args: Vec<&str> = ["run", "todo.lcd"].iter().chain(args).copied().collect();
            Ok(lucid(dir, &args))
        };
        let output = output.expect("cannot start the program");
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        text(&output.stdout)
    };
    assert_eq!(run(&[]), "");
    assert_eq!(run(&["add", "buy milk"]), "");
    assert_eq!(run(&["add", "call \"Ada\""]), "");
    assert_eq!(run(&["list"]), "[ ] buy milk\n[ ] call \"Ada\"\n");
    assert_eq!(run(&["add"]), "Usage: todo add <title>\n");
    assert_eq!(run(&["nope"]), "Unknown command: nope\n");
}

/// A built program gives what `lucid run` gives: traces, the stack limit, and
/// foreign calls each depend on how the program starts.
#[test]
fn built_programs_match_run() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("built");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("cannot make directory");
    let cases =
        [("trap", "trace", "err"), ("trap", "stack_overflow", "err"), ("run", "unsafe", "out")];
    for (kind, name, extension) in cases {
        let source = repo().join("tests").join(kind).join(format!("{name}.lcd"));
        fs::copy(source, dir.join(format!("{name}.lcd"))).expect("cannot copy");
        let output = lucid(&dir, &["build", &format!("{name}.lcd")]);
        assert!(output.status.success(), "{name}: {}", text(&output.stderr));
        let program = Command::new(dir.join(format!("{name}.exe"))).current_dir(&dir).output();
        let output = program.expect("cannot start the program");
        let path = repo().join("tests").join(kind).join(format!("{name}.{extension}"));
        let expected = text(&fs::read(path).expect("no expected output"));
        let (code, got) = match kind {
            "trap" => (Some(101), text(&output.stderr)),
            _ => (Some(0), text(&output.stdout)),
        };
        assert_eq!((output.status.code(), got), (code, expected), "{name}");
    }
}

/// A standard module's sites are not the program's to audit or fix.
#[test]
fn reports_leave_out_std() {
    let dir = repo().join("tests").join("run");
    let output = lucid(&dir, &["check", "--unsafe", "std_args_env.lcd"]);
    assert_eq!(text(&output.stdout), "");
    let output = lucid(&dir, &["check", "--costs", "std_args_env.lcd"]);
    assert!(!text(&output.stdout).contains("@std"), "{}", text(&output.stdout));
}

#[test]
fn bad_usage_exits_2() {
    assert_eq!(lucid(&repo(), &[]).status.code(), Some(2));
    assert_eq!(lucid(&repo(), &["check", "--nope"]).status.code(), Some(2));
    assert_eq!(lucid(&repo(), &["nope"]).status.code(), Some(2));
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
/// also have an entry in docs/errors.md.
#[test]
fn every_error_code_is_tested_and_documented() {
    // missing_file_is_reported, huge_frame_is_rejected, and
    // test_names_are_plain_and_new
    let mut tested = String::from("E0001 E0902 E0329");
    for err in files(&repo().join("tests").join("fail"), "err") {
        tested += &text(&fs::read(err).expect("cannot read .err file"));
    }
    for entry in fs::read_dir(repo().join("tests").join("package")).expect("cannot list") {
        tested += &fs::read_to_string(entry.expect("no entry").path().join("expected.err"))
            .unwrap_or_default();
    }
    let documented =
        fs::read_to_string(repo().join("docs").join("errors.md")).expect("cannot read docs");
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
    let entry = |code: &String| documented.contains(&format!("\n## {code}: "));
    let undocumented: Vec<_> = codes.iter().filter(|c| !entry(c)).collect();
    assert!(untested.is_empty(), "no golden test for {untested:?}");
    assert!(undocumented.is_empty(), "not in docs/errors.md: {undocumented:?}");
}

/// Each example in docs/errors.md gives its own code, and only that code.
/// `lucid check` leaves out what only code generation finds.
#[test]
fn error_examples_give_their_code() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("explain");
    fs::create_dir_all(&dir).expect("cannot make directory");
    let doc = fs::read_to_string(repo().join("docs/errors.md")).expect("cannot read docs");
    let mut failures = Vec::new();
    for entry in doc.split("\n## ").skip(1) {
        let code = &entry[..5];
        let Some(example) = entry.split("```lucid\n").nth(1) else { continue };
        let example = example.split("```").next().expect("split gives one part");
        fs::write(dir.join("example.lcd"), example).expect("cannot write");
        let command = if code.starts_with("E09") { "run" } else { "check" };
        let errors = text(&lucid(&dir, &[command, "example.lcd"]).stderr);
        let mut found: Vec<&str> =
            errors.split(": error ").skip(1).map(|rest| rest.get(..5).unwrap_or(rest)).collect();
        found.dedup();
        if found != [code] {
            failures.push(format!("{code}: {errors}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Lexer fixes come first, since a lexer error stops the checker. A second
/// run then makes the fixes that checking finds.
#[test]
fn fix_applies_fixes() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fix");
    fs::create_dir_all(&dir).expect("cannot make directory");
    let program = "let total = 1\r\nprint totl\r\nprint \"a}b\"\r\nif true\r\n\tprint totl\r\n";
    fs::write(dir.join("f.lcd"), program).expect("cannot write");
    let first = lucid(&dir, &["fix", "f.lcd"]);
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(
        text(&first.stdout),
        "f.lcd:3:9: fixed E0106: write `}}`\nf.lcd:5:1: fixed E0101: replace each tab with 4 spaces\n"
    );
    let second = text(&lucid(&dir, &["fix", "--json", "f.lcd"]).stdout);
    assert!(second.starts_with("[{\"code\":\"E0301\",\"description\":\"rename to `total`\""));
    let fixed = fs::read_to_string(dir.join("f.lcd")).expect("cannot read");
    assert_eq!(
        fixed,
        "let total = 1\r\nprint total\r\nprint \"a}}b\"\r\nif true\r\n    print total\r\n"
    );
    assert_eq!(text(&lucid(&dir, &["fix", "f.lcd"]).stdout), "");
}

#[test]
fn context_shows_public_api() {
    let dir = repo().join("tests/package");
    let output = lucid(&dir, &["context", "greet"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "# greet/src/deep/count.lcd\n\ntask letters(s: text) -> int\n\n\
         # greet/src/lib.lcd\n\ntype Name\n    text: text\n\ntask hello(name: Name) -> text\n"
    );
    let json = text(&lucid(&dir, &["context", "--json", "std.args"]).stdout);
    assert!(json.starts_with("[{\"module\":\"std.args\",\"line\":"), "{json}");
    assert!(json.contains("\"signature\":\"task all() -> list[text]\",\"doc\":\"Every"), "{json}");
    let missing = lucid(&dir, &["context", "std.nope"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(text(&missing.stderr), "lucid: `std` has no module `std.nope`\n");
}

#[test]
fn explain_prints_one_entry() {
    let dir = repo();
    let output = lucid(&dir, &["explain", "e0301"]);
    assert_eq!(output.status.code(), Some(0));
    let entry = text(&output.stdout);
    assert!(entry.starts_with("E0301: Name is not defined\n"), "{entry}");
    assert!(entry.contains("Fix: ") && !entry.contains("E0302"), "{entry}");
    let json = text(&lucid(&dir, &["explain", "--json", "E0314"]).stdout);
    assert!(json.starts_with("{\"code\":\"E0314\",\"title\":\"Empty list"), "{json}");
    assert!(json.contains("\"example\":\"let a = []\\n\",\"fix\":\"annotate"), "{json}");
    let missing = lucid(&dir, &["explain", "E9999"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(text(&missing.stderr), "lucid: no error code `E9999`, see docs/errors.md\n");
}
