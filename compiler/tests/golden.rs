//! Golden tests. See "Testing" in docs/compiler.md.
//!
//! `tests/run/x.lcd` must run, print `x.out`, and exit 0.
//! `tests/fail/x.lcd` must fail under `lucid run`, print `x.err`, and exit 1.
//! `tests/trap/x.lcd` must start, then trap with `x.err`, and exit 101.
//! `tests/test/x.lcd` must print `x.out` under `lucid test`, and exit 1 if a
//! test failed.
//! `tests/dom/x.lcd`, built with `--target web`, must print `x.out` under
//! `node tests/dom/page.js` with the events in `x.events`. With `x.err`, it
//! must also print that to stderr, and exit 101.
//! `tests/package/x/` must print `expected.out` and exit 0, or `expected.err`
//! and exit 1, under `lucid run x`. A directory with neither is a dependency.

use std::env::consts::EXE_SUFFIX;
use std::fs;
use std::io;
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

/// The compiler in Lucid, built once by the Rust one into its own directory,
/// by decision 214.
fn ported_exe() -> &'static Path {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lucid-stage");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("no stage dir");
        let built = lucid(&dir, &["build", repo().join("compiler/lucid").to_str().unwrap()]);
        assert!(built.status.success(), "lucid in Lucid: {}", text(&built.stderr));
        dir.join(format!("lucid{EXE_SUFFIX}"))
    })
}

/// The compiler in Lucid. It reads `std` from `LUCID_STD`, by decision 226.
fn ported(dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(ported_exe());
    command.args(args).current_dir(dir).env("LUCID_STD", repo().join("std"));
    command.output().expect("cannot start lucid in Lucid")
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

/// A way to start a compiler: `lucid`, `naive`, or `ported`.
type Run = fn(&Path, &[&str]) -> Output;

/// Runs every test in `dir` with each compiler, and reports all failures
/// together.
fn golden(dir: &str, expected_extension: &str, check: fn(&Output, &str) -> Option<String>) {
    let dir = repo().join("tests").join(dir);
    let mut failures = Vec::new();
    for source in files(&dir, "lcd") {
        let name = file_name(&source);
        let expected = fs::read(source.with_extension(expected_extension))
            .unwrap_or_else(|e| panic!("{name}: no .{expected_extension} file: {e}"));
        for (run, by) in [(lucid as Run, ""), (naive, " naive"), (ported, " in Lucid")] {
            let output = run(&dir, &["run", name]);
            if let Some(problem) = check(&output, &text(&expected)) {
                failures.push(format!("{name}{by}: {problem}"));
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

/// A deadlock among threads traps, whichever thread waits last, so only the
/// message is fixed. A wait at the end of a task counts too, by decision 174.
#[test]
fn deadlocks_across_threads_trap() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("deadlock");
    fs::create_dir_all(&dir).expect("cannot make test dir");
    let listen = "task listen(c: channel[int])
    let x = c.receive()
    print x

";
    let programs = [
        (
            "waits.lcd",
            "let c = channel[int](1)
let t = spawn listen(c)
t.wait()
",
        ),
        (
            "joins.lcd",
            "let c = channel[int](1)
spawn listen(c)
",
        ),
        (
            "ends.lcd",
            "let c = channel[int](1)
let t = spawn listen(c)
let y = c.receive()
",
        ),
    ];
    for (name, body) in programs {
        fs::write(dir.join(name), format!("{listen}{body}")).expect("cannot write test file");
        let output = lucid(&dir, &["run", name]);
        let stderr = text(&output.stderr);
        assert_eq!(output.status.code(), Some(101), "{name}: {stderr}");
        assert!(stderr.contains("trap: deadlock: every thread waits"), "{name}: {stderr}");
    }
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
        for run in [lucid as Run, naive, ported] {
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

/// Every `.lcd` file under `dir`, at any depth, as a path from the repository.
fn all_sources(dir: &str, out: &mut Vec<String>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(repo().join(dir))
        .expect("cannot list directory")
        .map(|entry| entry.expect("cannot read entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let name = format!("{dir}/{}", file_name(&path));
        if path.is_dir() {
            all_sources(&name, out);
        } else if name.ends_with(".lcd") {
            out.push(name);
        }
    }
}

/// `LUCID_DUMP` prints each stage of every file in the repository the same way
/// twice, and never panics, by decision 215. The Lucid compiler is compared
/// with these dumps as each of its stages lands. `naive` is `owned` with
/// `LUCID_NAIVE`, and `costs` and `unsafe` are those `check` flags.
#[test]
fn stage_dumps_are_stable() {
    let mut sources = Vec::new();
    for dir in ["tests", "examples", "std", "tools", "compiler/lucid"] {
        all_sources(dir, &mut sources);
    }
    // The second run of each stage uses the Lucid compiler, so it must print
    // what the Rust one does.
    let ported = ported_exe();
    let jobs: Vec<(&str, &str)> = sources
        .iter()
        .flat_map(|file| {
            ["tokens", "ast", "resolved", "checked", "mir", "owned", "naive", "costs", "unsafe"]
                .map(|stage| (file.as_str(), stage))
        })
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures = std::sync::Mutex::new(Vec::new());
    let dump = |file: &str, stage: &str, exe: &Path| {
        let mut command = Command::new(exe);
        command.current_dir(repo());
        match stage {
            "costs" | "unsafe" => {
                _ = command.args(["check", "--json", &format!("--{stage}"), file])
            }
            "naive" => {
                _ = command
                    .args(["check", "--json", file])
                    .envs([("LUCID_DUMP", "owned"), ("LUCID_NAIVE", "1")])
            }
            _ => _ = command.args(["check", "--json", file]).env("LUCID_DUMP", stage),
        }
        command.output().expect("cannot start lucid")
    };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some(&(file, stage)) =
                    jobs.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                {
                    let (first, second) =
                        (dump(file, stage, Path::new(LUCID)), dump(file, stage, ported));
                    let problem = if !matches!(first.status.code(), Some(0 | 1)) {
                        Some(format!("exit {:?}: {}", first.status.code(), text(&first.stderr)))
                    } else if first.stdout.is_empty() {
                        Some("printed nothing".to_string())
                    } else if first.stdout != second.stdout {
                        Some("differs between two runs".to_string())
                    } else if matches!(stage, "owned" | "naive") && first.status.success() {
                        read_back(&text(&first.stdout))
                    } else {
                        None
                    };
                    if let Some(problem) = problem {
                        failures.lock().unwrap().push(format!("{file} {stage}: {problem}"));
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
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

/// What the backend reads from an `owned` dump, by decision 216, written
/// again, must be the dump. `stdout` ends with the `--json` diagnostics.
fn read_back(stdout: &str) -> Option<String> {
    let dump = stdout.strip_suffix("[]\n").expect("a dump with no diagnostics");
    // E0901, which `check` leaves out, stops the dump.
    if dump.is_empty() {
        return None;
    }
    match lucid_backend::dump::read(dump) {
        Ok((program, source)) if lucid_backend::dump::program(&program, Some(&source)) == dump => {
            None
        }
        Ok(_) => Some("reads back as a different program".to_string()),
        Err(problem) => Some(format!("does not read back: {problem}")),
    }
}

/// The browser examples build for the web. The build goes to a temporary
/// directory, so the example directory stays clean.
#[test]
fn web_examples_build() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("web-examples");
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&repo().join("examples").join("web"), &dir);
    for example in files(&dir, "lcd") {
        let name = file_name(&example);
        let output = lucid(&dir, &["build", "--target", "web", name]);
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
    fs::write(scratch.join("unbuilt.lcd"), "use std.process\nprint 1\n")
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
    let source = "use std.process\n# It runs nothing.\nunsafe\n    print 1\n";
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

/// Only reachable tasks compile, by decision 190, so a task that nothing
/// calls cannot fail as E0902.
#[test]
fn unreachable_tasks_do_not_compile() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let parts = "{a}".repeat(70_000);
    let source = format!("task f(a: text)\n    print \"{parts}\"\nprint \"ok\"\n");
    fs::write(dir.join("unused_huge.lcd"), source).expect("cannot write test file");
    let output = lucid(dir, &["run", "unused_huge.lcd"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "ok\n");
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
            Command::new(dir.join(format!("todo{EXE_SUFFIX}"))).args(args).current_dir(dir).output()
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

/// The exit of M7: the built website answers 64 clients at once, each
/// with requests on one kept-alive connection, while one more client holds
/// a request half sent.
#[test]
fn website_serves_under_load() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
        }
    }
    // One response: the status line, and the body by its Content-Length.
    fn answer(reader: &mut BufReader<TcpStream>) -> (String, String) {
        let mut status = String::new();
        reader.read_line(&mut status).expect("no status line");
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("no header line");
            if line == "\r\n" {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length: ") {
                length = n.trim().parse().expect("bad length");
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).expect("short body");
        (status.trim_end().to_string(), String::from_utf8(body).expect("body is not UTF-8"))
    }

    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("website");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("cannot make directory");
    fs::copy(repo().join("examples/website.lcd"), dir.join("website.lcd")).expect("cannot copy");
    let output = lucid(&dir, &["build", "website.lcd"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    // A free port, given back so that the server can take it.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut server = Command::new(dir.join(format!("website{EXE_SUFFIX}")));
    server.arg(port.to_string()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut server = Kill(server.spawn().expect("cannot start the website"));
    let address = format!("127.0.0.1:{port}");
    let started = Instant::now();
    let connect = || loop {
        match TcpStream::connect(&address) {
            Ok(stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
                return stream;
            }
            Err(e) if started.elapsed() > Duration::from_secs(20) => panic!("cannot connect: {e}"),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let mut slow = connect();
    slow.write_all(b"GET /status HTTP/1.1\r\nHost: x\r\n").unwrap();
    let clients: Vec<_> = (0..64)
        .map(|i| {
            let stream = connect();
            std::thread::spawn(move || {
                let mut writer = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                for n in 0..20 {
                    let path = if (i + n) % 2 == 0 { "/" } else { "/status" };
                    let request = format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n");
                    writer.write_all(request.as_bytes()).unwrap();
                    let (status, body) = answer(&mut reader);
                    assert_eq!(status, "HTTP/1.1 200 OK", "{path}");
                    let wanted = if path == "/" { "Hello from Lucid" } else { "\"ok\":true" };
                    assert!(body.contains(wanted), "{path}: {body}");
                }
            })
        })
        .collect();
    let failed = clients.into_iter().map(|c| c.join()).filter(|r| r.is_err()).count();
    if failed == 0 {
        slow.write_all(b"\r\n").unwrap();
        let (status, body) = answer(&mut BufReader::new(slow));
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert!(body.contains("\"version\":\"0.1.0\""), "{body}");
    }
    let exited = server.0.try_wait().unwrap();
    server.0.kill().unwrap();
    let mut stderr = String::new();
    server.0.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    assert_eq!((failed, exited, stderr.as_str()), (0, None, ""), "clients failed, exit, stderr");
}

/// A built program gives what `lucid run` gives: traces, the stack limit,
/// foreign calls, threads, and signals each depend on how the program starts.
/// `std_http_client` writes to sockets that the other end closed, which on
/// Linux ends a program that does not ignore SIGPIPE.
#[test]
fn built_programs_match_run() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("built");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("cannot make directory");
    let cases = [
        ("trap", "trace", "err"),
        ("trap", "stack_overflow", "err"),
        ("trap", "spawn_overflow", "err"),
        ("run", "unsafe", "out"),
        ("run", "spawn", "out"),
        ("run", "std_http_client", "out"),
    ];
    for (kind, name, extension) in cases {
        let source = repo().join("tests").join(kind).join(format!("{name}.lcd"));
        fs::copy(source, dir.join(format!("{name}.lcd"))).expect("cannot copy");
        for build in [lucid as Run, ported] {
            let output = build(&dir, &["build", &format!("{name}.lcd")]);
            assert!(output.status.success(), "{name}: {}", text(&output.stderr));
            let program =
                Command::new(dir.join(format!("{name}{EXE_SUFFIX}"))).current_dir(&dir).output();
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
}

/// The root package's `files` and `env` lists hold at run time, under
/// `lucid run` and in a built program. See "Package permissions" in
/// docs/security.md.
#[test]
fn permissions_hold_at_run_time() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("permissions");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).expect("cannot make directory");
    fs::create_dir_all(dir.join("data")).expect("cannot make directory");
    let manifest = "[package]\nname = \"permissions\"\nversion = \"1.0.0\"\n\n\
        [permissions]\nfiles = [\"data\"]\nenv = [\"LUCID_PERM_SET\"]\n";
    let main = "use std.env\nuse std.fs\nuse std.sql\n\n\
        task show(result: text or fs.FileError)\n    match result\n        \
        content: text -> print content\n        e: fs.FileError -> print e.error_message()\n\n\
        task opened(path: text)\n    match sql.open(path)\n        \
        db: sql.Db -> print \"opened {path}\"\n        \
        e: sql.SqlError -> print \"{path}: {e.error_message()}\"\n\n\
        show(fs.read_text(\"data/a.txt\"))\nshow(fs.read_text(\"lucid.toml\"))\n\
        show(fs.read_text(\"data/../lucid.toml\"))\nprint fs.exists(\"data/a.txt\")\n\
        print fs.exists(\"lucid.toml\")\nopened(\"data/app.db\")\nopened(\"app.db\")\n\
        opened(\"file:data/app.db\")\nopened(\":memory:\")\nprint env.get(\"LUCID_PERM_SET\")\n\
        print env.get(\"LUCID_PERM_OTHER\")\n";
    fs::write(dir.join("lucid.toml"), manifest).expect("cannot write");
    fs::write(dir.join("src/main.lcd"), main).expect("cannot write");
    fs::write(dir.join("data/a.txt"), "inside").expect("cannot write");
    let output = lucid(&dir, &["build"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let outside = "outside the `files` permission in `lucid.toml`";
    let expected = format!(
        "inside\nlucid.toml: {outside}\ndata/../lucid.toml: {outside}\ntrue\nfalse\n\
        opened data/app.db\napp.db: {outside}\nfile:data/app.db: {outside}\nopened :memory:\nyes\n"
    );
    for built in [false, true] {
        let mut command = if built {
            Command::new(dir.join(format!("permissions{EXE_SUFFIX}")))
        } else {
            let mut command = Command::new(LUCID);
            command.arg("run");
            command
        };
        let output = command.current_dir(&dir).env("LUCID_PERM_SET", "yes").output();
        let output = output.expect("cannot start the program");
        assert_eq!(text(&output.stdout), expected, "built: {built}");
        assert_eq!(output.status.code(), Some(101), "built: {built}");
        let trap = "trap: `env.get(\"LUCID_PERM_OTHER\")`: the `env` permission does not list it";
        assert!(text(&output.stderr).starts_with(trap), "{}", text(&output.stderr));
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
    // missing_file_is_reported, huge_frame_is_rejected,
    // test_names_are_plain_and_new, web_rejects_foreign_code,
    // web_rejects_spawn, and registry_fetches_and_locks
    let mut tested = String::from("E0001 E0902 E0329 E0905 E0907 E0343 E0344 E0345");
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
    // No rename to the same name, or to a name that does not work there.
    let unfixable = "print count\nlet count = 1\nvar total = 0\ntask f() -> int\n    return totl\n";
    fs::write(dir.join("g.lcd"), unfixable).expect("cannot write");
    assert_eq!(text(&lucid(&dir, &["fix", "g.lcd"]).stdout), "");
    // In a package the manifest comes first, so `main.lcd` does not start
    // at offset 0, and its fixes must still land in it.
    let package = dir.join("package");
    fs::create_dir_all(package.join("src")).expect("cannot make directory");
    let manifest = "[package]\nname = \"package\"\nversion = \"1.0.0\"\n";
    fs::write(package.join("lucid.toml"), manifest).expect("cannot write");
    fs::write(package.join("src/main.lcd"), "if true\n\tprint \"a}b\"\n").expect("cannot write");
    let output = lucid(&dir, &["fix", "package"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let read = |path: &str| fs::read_to_string(package.join(path)).expect("cannot read");
    assert_eq!(read("lucid.toml"), manifest);
    assert_eq!(read("src/main.lcd"), "if true\n    print \"a}}b\"\n");
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

/// `tests/fmt/x.lcd` must format to `x.out`, and `x.out` must stay as it is.
#[test]
fn fmt_gives_the_layout() {
    let dir = repo().join("tests").join("fmt");
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fmt");
    fs::create_dir_all(&scratch).expect("cannot make directory");
    let mut failures = Vec::new();
    for source in files(&dir, "lcd") {
        let name = file_name(&source);
        let expected = fs::read(source.with_extension("out")).expect("no .out file");
        fs::copy(&source, scratch.join(name)).expect("cannot copy");
        let output = lucid(&scratch, &["fmt", name]);
        let got = fs::read(scratch.join(name)).expect("cannot read");
        if !output.status.success() || got != expected {
            failures.push(format!("{name}: {}\n{}", text(&got), text(&output.stderr)));
        }
        fs::write(scratch.join(name), &expected).expect("cannot write");
        if !lucid(&scratch, &["fmt", "--check", name]).status.success() {
            failures.push(format!("{name}: the .out file is not stable"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Generated, since git may change line breaks in a file it holds.
#[test]
fn fmt_reports_and_rewrites() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fmt_cases");
    fs::create_dir_all(&dir).expect("cannot make directory");
    let raw = "\u{feff}if a\r\n\t# tab\r\n    print  1   \r\n\r\n";
    fs::write(dir.join("crlf.lcd"), raw).expect("cannot write");
    let check = lucid(&dir, &["fmt", "--check", "crlf.lcd"]);
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(text(&check.stdout), "crlf.lcd\n");
    assert_eq!(fs::read_to_string(dir.join("crlf.lcd")).expect("cannot read"), raw);
    assert!(lucid(&dir, &["fmt", "crlf.lcd"]).status.success());
    let formatted = fs::read_to_string(dir.join("crlf.lcd")).expect("cannot read");
    assert_eq!(formatted, "if a\n    # tab\n    print 1\n");
    // A lexer error is reported, and the file stays as it was.
    fs::write(dir.join("bad.lcd"), "print  \"a}\"\n").expect("cannot write");
    let bad = lucid(&dir, &["fmt", "bad.lcd"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(text(&bad.stderr).contains("error E0106"), "{}", text(&bad.stderr));
    assert_eq!(fs::read_to_string(dir.join("bad.lcd")).expect("cannot read"), "print  \"a}\"\n");
}

/// Lucid code that ships with the compiler must already be formatted.
#[test]
fn shipped_code_is_formatted() {
    for dir in ["std", "tools", "examples", "examples/web"] {
        let dir = repo().join(dir);
        for file in files(&dir, "lcd") {
            let name = file_name(&file);
            let output = lucid(&dir, &["fmt", "--check", name]);
            assert!(output.status.success(), "{name}: {}", text(&output.stdout));
        }
    }
}

/// Loads the WebAssembly runtime under Node, with the imports the loader
/// gives, by decisions 156 and 159. Checks its imports and exports, that its
/// headers sit at the host's offsets, and each import it uses.
#[test]
fn web_runtime_loads() {
    let script = r#"
        const [len, count, tag] = process.argv.slice(1).map(BigInt);
        const bytes = require("fs").readFileSync(0);
        const module = new WebAssembly.Module(bytes);
        const say = (...words) => console.log(words.join(" "));
        say("imports", WebAssembly.Module.imports(module).map(i => i.module + "." + i.name).sort());
        let memory, written = ["", "", ""], random = 0;
        const text = (at, n) => Buffer.from(memory.buffer, at, n).toString();
        const js = {
            write: (stream, at, n) => { written[stream] += text(at, n); },
            trap: (at, n) => { throw new Error(text(at, n)); },
            exit: code => { throw new Error("exit " + code); },
            now: () => Date.now(),
            clock: () => performance.now(),
            random: (at, n) => { random += n; crypto.getRandomValues(new Uint8Array(memory.buffer, at, n)); },
        };
        // The page's tasks are tested in `dom_pages_match`.
        for (const { name } of WebAssembly.Module.imports(module)) {
            js[name] ??= () => 0;
        }
        const run = new WebAssembly.Instance(module, { js }).exports;
        memory = run.memory;
        const words = at => new BigUint64Array(memory.buffer.slice(at, at + 8))[0];
        say("table grows", run.__indirect_function_table.grow(1) > 0);
        const truth = run.lucid_bool_text(1);
        say("text", words(truth + Number(len)), words(truth + Number(count)));
        say("tag", words(run.lucid_record_new(7n, 0, 0) + Number(tag)));
        run.lucid_print_int(42n);
        say("stdout", JSON.stringify(written[1]), "stderr", JSON.stringify(written[2]));
        say("now", run.lucid_time_now() > 1_700_000_000_000_000_000n, run.lucid_time_clock() >= 0n);
        run.lucid_hash_int(0n, 1n);
        say("random", random);
        try { run.lucid_time_sleep(1n); } catch (error) { say(error.message); }
    "#;
    let mut node = Command::new("node")
        .args(["-e", script])
        .args(
            [lucid_rt::LEN_OFFSET, lucid_rt::COUNT_OFFSET, lucid_rt::TAG_OFFSET]
                .map(|at| at.to_string()),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cannot start node, which web tests need");
    let wasm = lucid_backend::wasm::RUNTIME;
    io::Write::write_all(&mut node.stdin.take().expect("piped"), wasm).expect("cannot write");
    let output = node.wait_with_output().expect("node failed");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let expected = "\
imports js.clock,js.dom_append,js.dom_create,js.dom_off,js.dom_on,js.dom_remove,js.dom_replace,js.dom_set,js.dom_set_text,js.dom_text,js.dom_unset,js.dom_value,js.dom_value_len,js.exit,js.now,js.random,js.trap,js.write
table grows true
text 4 1
tag 7
stdout \"42\\n\" stderr \"\"
now true true
random 16
trap: time.sleep is not available on the web
";
    assert_eq!(text(&output.stdout), expected);
}

/// Copies a directory and what it holds.
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("cannot make directory");
    for entry in fs::read_dir(from).expect("cannot list directory") {
        let path = entry.expect("cannot read entry").path();
        let target = to.join(path.file_name().expect("an entry has a name"));
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).expect("cannot copy");
        }
    }
}

/// Builds each run and trap test with `--target web`, and runs it under
/// `node`, against the same golden files. Only the tests in `HOST_ONLY` are
/// left out: by decisions 159, 162, 176, 193, and 194, the web has no
/// files, arguments, environment, sleep, user `extern`, `spawn`, sockets,
/// `std.crypto`, or `std.sql`. See "WebAssembly" in docs/compiler.md.
#[test]
fn web_programs_match_run() {
    const HOST_ONLY: [&str; 14] = [
        "channel_threads",
        "spawn",
        "spawn_overflow",
        "spawn_trap",
        "sql_marks",
        "std_args_env",
        "std_crypto",
        "std_fs",
        "std_http",
        "std_http_client",
        "std_net",
        "std_sql",
        "std_time",
        "unsafe",
    ];
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("web");
    let _ = fs::remove_dir_all(&root);
    let mut failures = Vec::new();
    let mut ran = 0;
    for (kind, code, extension) in [("run", 0, "out"), ("trap", 101, "err")] {
        let dir = root.join(kind);
        copy_dir(&repo().join("tests").join(kind), &dir);
        for source in files(&dir, "lcd") {
            let name = file_name(&source);
            if HOST_ONLY.contains(&name.trim_end_matches(".lcd")) {
                continue;
            }
            let expected = text(&fs::read(source.with_extension(extension)).expect("no golden"));
            let output = lucid(&dir, &["build", "--target", "web", name]);
            if !output.status.success() {
                failures.push(format!("{name}: build failed: {}", text(&output.stderr)));
                continue;
            }
            let script = source.with_extension("js");
            let output = Command::new("node").arg(&script).current_dir(&dir).output();
            let output = output.expect("cannot start node, which web tests need");
            let got = match kind {
                "run" => text(&output.stdout),
                _ => text(&output.stderr),
            };
            if output.status.code() != Some(code) || got != expected {
                let stderr = text(&output.stderr);
                failures.push(format!(
                    "{name}: exit {:?}
got: {got:?}
expected: {expected:?}
stderr: {stderr}",
                    output.status.code()
                ));
            }
            ran += 1;
        }
    }
    assert!(
        failures.is_empty(),
        "
{}",
        failures.join(
            "
"
        )
    );
    assert!(ran > 90, "only {ran} web tests ran");
}

/// Runs each page in `tests/dom` on the fake `document` of `page.js`, by
/// decision 164.
#[test]
fn dom_pages_match() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("dom");
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&repo().join("tests").join("dom"), &dir);
    let mut failures = Vec::new();
    for source in files(&dir, "lcd") {
        let name = file_name(&source);
        let output = lucid(&dir, &["build", "--target", "web", name]);
        if !output.status.success() {
            failures.push(format!("{name}: build failed: {}", text(&output.stderr)));
            continue;
        }
        let output = Command::new("node")
            .args(["page.js", file_name(&source.with_extension("js"))])
            .arg(file_name(&source.with_extension("events")))
            .current_dir(&dir)
            .output()
            .expect("cannot start node, which web tests need");
        let expected = text(&fs::read(source.with_extension("out")).expect("no .out file"));
        let errors = fs::read(source.with_extension("err")).map(|e| text(&e)).unwrap_or_default();
        let code = if errors.is_empty() { 0 } else { 101 };
        let (got, got_errors) = (text(&output.stdout), text(&output.stderr));
        if output.status.code() != Some(code) || got != expected || got_errors != errors {
            failures.push(format!(
                "{name}: exit {:?}
got: {got}
stderr: {got_errors}",
                output.status.code()
            ));
        }
    }
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

/// By decision 162, a user `extern` cannot run on the web.
#[test]
fn web_rejects_foreign_code() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("foreign");
    fs::create_dir_all(&dir).expect("cannot make directory");
    fs::copy(repo().join("tests/run/unsafe.lcd"), dir.join("unsafe.lcd")).expect("cannot copy");
    let output = lucid(&dir, &["build", "--target", "web", "unsafe.lcd"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("error E0905"), "{}", text(&output.stderr));
}

/// `spawn`, `std.net`, `std.http`, `std.crypto`, and `std.sql` are E0907
/// under `--target web`, by decisions 176, 193, and 194.
#[test]
fn web_rejects_spawn() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("web_spawn");
    fs::create_dir_all(&dir).expect("cannot make directory");
    for (name, says) in [
        ("spawn", "`spawn`"),
        ("std_net", "`std.net`"),
        ("std_http", "`std.http`"),
        ("std_crypto", "`std.crypto`"),
        ("std_sql", "`std.sql`"),
    ] {
        let file = format!("{name}.lcd");
        fs::copy(repo().join("tests/run").join(&file), dir.join(&file)).expect("cannot copy");
        let output = lucid(&dir, &["build", "--target", "web", &file]);
        assert_eq!(output.status.code(), Some(1));
        let error = text(&output.stderr);
        assert!(error.contains(&format!("error E0907: {says} cannot run")), "{error}");
    }
}
/// The registry, by decisions 197 to 201, from a local directory. `lucid
/// package` writes an archive and its index line. `lucid run` picks versions
/// by minimal version selection, fetches into `LUCID_CACHE`, and writes
/// `lucid.lock`. A changed archive or pin is E0345, two majors E0344, and what
/// the registry lacks E0343.
#[test]
fn registry_fetches_and_locks() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("registry");
    let _ = fs::remove_dir_all(&dir);
    let cache = dir.join("cache");
    let write = |path: &str, content: &str| {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().expect("a parent")).expect("cannot make directory");
        fs::write(path, content).expect("cannot write");
    };
    let run = |cwd: &str, args: &[&str]| {
        let mut command = Command::new(LUCID);
        command.args(args).current_dir(dir.join(cwd)).env("LUCID_CACHE", &cache);
        command.output().expect("cannot start lucid")
    };
    let head = |name: &str, version: &str| {
        format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\n\n\
             [registry]\nurl = \"../registry\"\n\n[dependencies]\n"
        )
    };
    let add_to_index = |name: &str, line: &str| {
        let index = format!("registry/index/{name}");
        let old = fs::read_to_string(dir.join(&index)).unwrap_or_default();
        write(&index, &format!("{old}{line}"));
    };
    let publish = |name: &str, version: &str, dependencies: &str, lib: &str| {
        let source = format!("{name}-{version}");
        write(&format!("{source}/lucid.toml"), &(head(name, version) + dependencies));
        write(&format!("{source}/src/lib.lcd"), lib);
        let output = run(".", &["package", &source]);
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        let archive = fs::read(dir.join(format!("{source}.lpk"))).expect("an archive");
        write(&format!("registry/files/{name}/{version}.lpk"), &text(&archive));
        add_to_index(name, &text(&output.stdout));
    };
    // An archive written by hand, with its hash from `std.crypto`.
    let publish_raw = |name: &str, version: &str, archive: &str| {
        write(&format!("registry/files/{name}/{version}.lpk"), archive);
        let hash = "use std.crypto\nuse std.fs\nuse std.args\n\
            let data = fs.read_bytes(args.get(0) else \"\") else []\n\
            print crypto.hex(crypto.sha256(data))\n";
        write("hash.lcd", hash);
        let path = format!("registry/files/{name}/{version}.lpk");
        let sha = text(&run(".", &["run", "hash.lcd", &path]).stdout).trim().to_string();
        let line = format!("{{\"version\":\"{version}\",\"sha256\":\"{sha}\"}}\n");
        add_to_index(name, &line);
    };
    let says_version =
        |version: &str| format!("public task version() -> text\n    return \"{version}\"\n");
    publish("util", "1.1.0", "", &says_version("1.1.0"));
    publish("util", "1.2.0", "", &says_version("1.2.0"));
    publish("util", "1.3.0", "", &says_version("1.3.0"));
    publish("util", "2.0.0", "", &says_version("2.0.0"));
    let md =
        "use util\n\npublic task render() -> text\n    return \"md on util {util.version()}\"\n";
    publish("md", "1.0.0", "util = { version = \"1.1.0\" }\n", md);

    // `md` asks for util 1.1.0 and the app for 1.2.0, so both get 1.2.0, not
    // the newest 1.3.0.
    let main = "use md\nuse util\n\nprint md.render()\nprint util.version()\n";
    let app = |name: &str, dependencies: &str| {
        write(&format!("{name}/lucid.toml"), &(head(name, "0.1.0") + dependencies));
        write(&format!("{name}/src/main.lcd"), main);
    };
    app("app", "md = { version = \"1.0.0\" }\nutil = { version = \"1.2.0\" }\n");
    let output = run(".", &["run", "app"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "md on util 1.2.0\n1.2.0\n");
    let stderr = text(&output.stderr);
    assert!(stderr.contains("lucid: fetching md 1.0.0\n"), "{stderr}");
    assert!(stderr.contains("lucid: fetching util 1.2.0\n"), "{stderr}");
    let lock = fs::read_to_string(dir.join("app/lucid.lock")).expect("a lock");
    let names: Vec<&str> =
        lock.lines().skip(1).map(|line| &line[..line.rfind(' ').unwrap()]).collect();
    assert_eq!(names, ["md 1.0.0", "util 1.2.0"], "{lock}");
    let again = run(".", &["run", "app"]);
    assert_eq!((text(&again.stdout), text(&again.stderr)), (text(&output.stdout), String::new()));

    // A pin in the lock that the index does not give.
    let util = lock.lines().find(|line| line.starts_with("util")).expect("a line").to_string();
    let other = format!("util 1.2.0 {}", "0".repeat(64));
    fs::write(dir.join("app/lucid.lock"), lock.replace(&util, &other)).expect("cannot write");
    let pinned = run(".", &["run", "app"]);
    assert_eq!(pinned.status.code(), Some(1));
    assert!(
        text(&pinned.stderr).contains("error E0345: `lucid.lock` gives `util` 1.2.0"),
        "{}",
        text(&pinned.stderr)
    );
    fs::write(dir.join("app/lucid.lock"), "util 1.2.0\n").expect("cannot write");
    let bad_lock = run(".", &["run", "app"]);
    assert!(
        text(&bad_lock.stderr).contains("app/lucid.lock:1:1: error E0338"),
        "{}",
        text(&bad_lock.stderr)
    );
    fs::write(dir.join("app/lucid.lock"), &lock).expect("cannot write");

    // An archive that changed after the index gave its hash.
    let archive = dir.join("registry/files/util/1.2.0.lpk");
    let original = fs::read(&archive).expect("an archive");
    let cached = fs::read_dir(&cache).expect("a cache").flatten().map(|entry| entry.path());
    for entry in cached.filter(|path| file_name(path).starts_with("util-1.2.0-")) {
        fs::remove_dir_all(entry).expect("cannot remove");
    }
    fs::write(&archive, text(&original).replace("1.2.0", "6.6.6")).expect("cannot write");
    let changed = run(".", &["run", "app"]);
    assert_eq!(changed.status.code(), Some(1));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("error E0345: the archive of `util` 1.2.0 has the SHA-256"),
        "{stderr}"
    );
    fs::write(&archive, original).expect("cannot write");

    app("majors", "md = { version = \"1.0.0\" }\nutil = { version = \"2.0.0\" }\n");
    let majors = run(".", &["run", "majors"]);
    assert_eq!(majors.status.code(), Some(1));
    let stderr = text(&majors.stderr);
    assert!(stderr.contains("error E0344: two major versions of `util`"), "{stderr}");

    // A version that MVS passes over may want another major of a package
    // than the chosen version does. Only the chosen one counts.
    publish("y", "1.0.0", "", &says_version("y1"));
    publish("y", "2.0.0", "", &says_version("y2"));
    publish("util", "1.4.0", "y = { version = \"1.0.0\" }\n", &says_version("1.4.0"));
    publish("util", "1.5.0", "y = { version = \"2.0.0\" }\n", &says_version("1.5.0"));
    write("side/lucid.toml", &(head("side", "1.0.0") + "util = { version = \"1.4.0\" }\n"));
    write("side/src/lib.lcd", "public task nothing() -> int\n    return 0\n");
    for name in ["passed_over", "passed_under"] {
        let side = "side = { path = \"../side\" }\n";
        let util = "util = { version = \"1.5.0\" }\n";
        let order =
            if name == "passed_over" { format!("{side}{util}") } else { format!("{util}{side}") };
        write(&format!("{name}/lucid.toml"), &(head(name, "0.1.0") + &order));
        write(&format!("{name}/src/main.lcd"), "use util\n\nprint util.version()\n");
        let output = run(".", &["run", name]);
        assert_eq!(text(&output.stdout), "1.5.0\n", "{name}: {}", text(&output.stderr));
    }

    app(
        "absent",
        "md = { version = \"1.0.0\" }\nutil = { version = \"1.2.0\" }\nnothing = { version = \"1.0.0\" }\n",
    );
    let absent = run(".", &["run", "absent"]);
    let stderr = text(&absent.stderr);
    assert!(
        stderr.contains("absent/lucid.toml:11:1: error E0343: the registry has no"),
        "{stderr}"
    );

    write(
        "no_registry/lucid.toml",
        "[package]\nname = \"x\"\nversion = \"1.0.0\"\n\n[dependencies]\nmd = { version = \"1.0.0\" }\n",
    );
    write("no_registry/src/main.lcd", "print 1\n");
    let stderr = text(&run(".", &["run", "no_registry"]).stderr);
    assert!(
        stderr.contains("error E0338: a dependency with a `version` needs a registry"),
        "{stderr}"
    );

    // Archives that break the rules of decision 200. None of their files
    // may land outside the cache.
    let manifest = |version: &str, deps: &str| {
        let toml = format!(
            "[package]\nname = \"evil\"\nversion = \"{version}\"\n\n[dependencies]\n{deps}"
        );
        json_text(&toml)
    };
    let lib = json_text("public task version() -> text\n    return \"evil\"\n");
    let escape = format!(
        "{{\"lucid.toml\":{},\"src/lib.lcd\":{lib},\"src/../../../escaped.lcd\":{lib}}}",
        manifest("1.0.0", "")
    );
    publish_raw("evil", "1.0.0", &escape);
    let path_dependency = format!(
        "{{\"lucid.toml\":{},\"src/lib.lcd\":{lib}}}",
        manifest("1.1.0", "util = { path = \"../util\" }\n")
    );
    publish_raw("evil", "1.1.0", &path_dependency);
    let liar = format!(
        "{{\"lucid.toml\":{},\"src/lib.lcd\":{lib}}}",
        manifest("1.2.0", "util = { version = \"1.1.0\" }\n")
    );
    publish_raw("evil", "1.2.0", &liar);
    for (version, says) in [
        ("1.0.0", "error E0343: the archive of `evil` 1.0.0 holds a bad path"),
        ("1.1.0", "error E0339: a package from the registry cannot have a `path` dependency"),
        ("1.2.0", "error E0343: `util` 1.1.0 is not in `lucid.lock` after a fetch"),
    ] {
        let name = format!("evil{}", version.replace('.', ""));
        write(
            &format!("{name}/lucid.toml"),
            &(head(&name, "0.1.0") + &format!("evil = {{ version = \"{version}\" }}\n")),
        );
        write(&format!("{name}/src/main.lcd"), "use evil\n\nprint evil.version()\n");
        let output = run(".", &["run", &name]);
        assert_eq!(output.status.code(), Some(1), "{version}");
        assert!(text(&output.stderr).contains(says), "{version}: {}", text(&output.stderr));
    }
    assert!(!dir.join("escaped.lcd").exists() && !cache.join("escaped.lcd").exists());

    // `lucid package` takes only what an archive may hold.
    write("loud/lucid.toml", &head("loud", "1.0.0"));
    write("loud/src/lib.lcd", "use ./Shout\n");
    write("loud/src/Shout.lcd", "public task x() -> int\n    return 1\n");
    let output = run(".", &["package", "loud"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("cannot go in an archive"), "{}", text(&output.stderr));

    // The M8 exit: a package uses a registry package built on `std.sql` and
    // `std.crypto`, and passes `lucid test`.
    let vault = "use std.crypto\nuse std.sql\nuse std.sql.Db\nuse std.sql.SqlError\n\n\
        public task open() -> Db or SqlError\n\
        \x20   let db = try sql.open(\":memory:\")\n\
        \x20   try db.run(\"CREATE TABLE users (name TEXT PRIMARY KEY, hash TEXT NOT NULL)\", [])\n\
        \x20   return db\n\n\
        public task add(db: Db, name: text, password: text) -> int or SqlError\n\
        \x20   let hash = crypto.hash_password(password)\n\
        \x20   return try db.run(\"INSERT INTO users VALUES (?, ?)\", [sql.Text(value: name), sql.Text(value: hash)])\n\n\
        public task login(db: Db, name: text, password: text) -> bool or SqlError\n\
        \x20   let rows = try db.query(\"SELECT hash FROM users WHERE name = ?\", [sql.Text(value: name)])\n\
        \x20   if rows.len() == 0\n\
        \x20       return false\n\
        \x20   return match rows[0].values[0]\n\
        \x20       Text(value) -> crypto.check_password(password, value)\n\
        \x20       _ -> false\n";
    let files = "\n[permissions]\nfiles = [\"./data\"]\n";
    publish("vault", "1.0.0", files, vault);
    let dependency = "vault = { version = \"1.0.0\", allow = [\"files\"] }\n";
    write("keeper/lucid.toml", &(head("keeper", "0.1.0") + dependency + files));
    let keeper = "use vault\n\nprint \"keeper\"\n\n\
        test \"logs in with the right password only\"\n\
        \x20   let db = try vault.open()\n\
        \x20   expect try vault.add(db, \"ada\", \"correct horse\") == 1\n\
        \x20   expect try vault.login(db, \"ada\", \"correct horse\")\n\
        \x20   expect not try vault.login(db, \"ada\", \"wrong\")\n\
        \x20   expect not try vault.login(db, \"bob\", \"correct horse\")\n";
    write("keeper/src/main.lcd", keeper);
    let output = run(".", &["test", "keeper"]);
    assert_eq!(output.status.code(), Some(0), "{}{}", text(&output.stdout), text(&output.stderr));
    assert!(text(&output.stdout).contains("1 tests, 0 failed"), "{}", text(&output.stdout));
}

/// A string in JSON, for an archive written by hand.
fn json_text(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            c if c < ' ' => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

/// `lucid lsp` driven over pipes, as an editor drives it, by decision 202.
#[test]
fn language_server_answers_an_editor() {
    use io::{BufRead, Read, Write};
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lsp");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).expect("cannot make directory");
    let write =
        |file: &str, content: &str| fs::write(dir.join(file), content).expect("cannot write");
    write("lucid.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n");
    write(
        "src/util.lcd",
        "## Adds one to a number.\npublic task add_one(n: int) -> int\n    return n + 1\n",
    );
    // The file on disk is empty, so only the editor's text can give the results.
    write("src/main.lcd", "");
    let real = fs::canonicalize(dir.join("src/main.lcd")).expect("cannot find main.lcd");
    let real = real.display().to_string();
    let real = real.strip_prefix(r"\\?\").unwrap_or(&real).replace('\\', "/");
    let uri =
        if real.starts_with('/') { format!("file://{real}") } else { format!("file:///{real}") };

    let mut server = Command::new(LUCID)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("cannot start lucid lsp");
    let input = std::cell::RefCell::new(server.stdin.take().expect("a piped stdin"));
    let mut output = io::BufReader::new(server.stdout.take().expect("a piped stdout"));
    let send_raw = |raw: &str| {
        let mut input = input.borrow_mut();
        input.write_all(raw.as_bytes()).expect("cannot write");
        input.flush().expect("cannot flush");
    };
    let send = |body: String| send_raw(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    // Reads messages until one holds `wanted`.
    let mut receive = |wanted: &str| loop {
        let mut length = 0;
        loop {
            let mut line = String::new();
            assert_ne!(output.read_line(&mut line).expect("cannot read"), 0, "no `{wanted}`");
            match line.trim_end().split_once(": ") {
                Some(("Content-Length", n)) => length = n.parse().expect("a length"),
                None => break,
                _ => {}
            }
        }
        let mut body = vec![0; length];
        output.read_exact(&mut body).expect("cannot read a message");
        let body = String::from_utf8(body).expect("UTF-8");
        if body.contains(wanted) {
            return body;
        }
    };
    let quoted = json_text;
    let change = |version: usize, text: &str| {
        format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{uri}","version":{version}}},"contentChanges":[{{"text":{}}}]}}}}"#,
            quoted(text)
        )
    };
    let at = |id: usize, method: &str, line: usize, character: usize| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/{method}","params":{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":{line},"character":{character}}}}}}}"#
        )
    };

    send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#.into());
    let reply = receive(r#""id":1"#);
    assert!(reply.contains(r#""positionEncoding":"utf-16""#), "{reply}");
    send(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#.into());

    let text = "use ./util\n\nlet x = util.add_one(2)\nprint x\nprint missing\n";
    send(format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"lucid","version":1,"text":{}}}}}}}"#,
        quoted(text)
    ));
    let found = receive("publishDiagnostics");
    assert!(found.contains(&uri), "{found}");
    assert!(found.contains(r#""code":"E0301""#), "{found}");
    assert!(found.contains(r#""start":{"line":4,"character":6}"#), "{found}");

    send(change(2, "use ./util\n\nlet x = util.add_one(2)\nprint x\n"));
    let cleared = receive("publishDiagnostics");
    assert!(cleared.contains(r#""diagnostics":[]"#), "{cleared}");

    // `add_one` in `util.add_one(2)`, then `x` in `print x`.
    send(at(2, "definition", 2, 15));
    let place = receive(r#""id":2"#);
    assert!(place.contains("util.lcd"), "{place}");
    assert!(place.contains(r#""start":{"line":1,"character":12}"#), "{place}");
    send(at(3, "hover", 2, 15));
    let shown = receive(r#""id":3"#);
    assert!(shown.contains("public task add_one(n: int) -> int"), "{shown}");
    assert!(shown.contains("Adds one to a number."), "{shown}");
    send(at(4, "hover", 3, 6));
    let shown = receive(r#""id":4"#);
    assert!(shown.contains("x: int"), "{shown}");

    send(change(3, "use ./util\n\nlet x  =  util.add_one(2)\nprint x\n"));
    receive("publishDiagnostics");
    send(format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"textDocument/formatting","params":{{"textDocument":{{"uri":"{uri}"}},"options":{{"tabSize":4,"insertSpaces":true}}}}}}"#
    ));
    let edits = receive(r#""id":5"#);
    assert!(
        edits.contains(r#""newText":"use ./util\n\nlet x = util.add_one(2)\nprint x\n""#),
        "{edits}"
    );

    // A parameter, in a task after one with a `let` in the same local slot.
    send(change(
        4,
        "task a() -> int\n    let y = 1\n    return y\n\ntask b(n: int) -> int\n    return n\n\nprint b(a())\n",
    ));
    receive("publishDiagnostics");
    send(at(7, "definition", 5, 11));
    let place = receive(r#""id":7"#);
    assert!(place.contains(r#""start":{"line":4,"character":7}"#), "{place}");
    send(at(8, "hover", 5, 11));
    let shown = receive(r#""id":8"#);
    assert!(shown.contains("n: int"), "{shown}");

    // A frame with no length gets an error, and the server goes on.
    send_raw("X-Nothing: 1\r\n\r\n");
    receive(r#""code":-32700"#);

    send(r#"{"jsonrpc":"2.0","id":6,"method":"shutdown"}"#.into());
    receive(r#""id":6"#);
    send(r#"{"jsonrpc":"2.0","method":"exit"}"#.into());
    assert_eq!(server.wait().expect("cannot wait").code(), Some(0));
}
