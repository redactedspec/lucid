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
    let mut server = Command::new(dir.join("website.exe"));
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
/// foreign calls, and threads each depend on how the program starts.
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
    ];
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
    let main = "use std.env\nuse std.fs\n\n\
        task show(result: text or fs.FileError)\n    match result\n        \
        content: text -> print content\n        e: fs.FileError -> print e.error_message()\n\n\
        show(fs.read_text(\"data/a.txt\"))\nshow(fs.read_text(\"lucid.toml\"))\n\
        show(fs.read_text(\"data/../lucid.toml\"))\nprint fs.exists(\"data/a.txt\")\n\
        print fs.exists(\"lucid.toml\")\nprint env.get(\"LUCID_PERM_SET\")\n\
        print env.get(\"LUCID_PERM_OTHER\")\n";
    fs::write(dir.join("lucid.toml"), manifest).expect("cannot write");
    fs::write(dir.join("src/main.lcd"), main).expect("cannot write");
    fs::write(dir.join("data/a.txt"), "inside").expect("cannot write");
    let output = lucid(&dir, &["build"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let outside = "outside the `files` permission in `lucid.toml`";
    let expected =
        format!("inside\nlucid.toml: {outside}\ndata/../lucid.toml: {outside}\ntrue\nfalse\nyes\n");
    for built in [false, true] {
        let mut command = if built {
            Command::new(dir.join("permissions.exe"))
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
    // test_names_are_plain_and_new, web_rejects_foreign_code, and
    // web_rejects_spawn
    let mut tested = String::from("E0001 E0902 E0329 E0905 E0907");
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
    let wasm = include_bytes!(env!("LUCID_RT_WASM"));
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
/// left out: by decisions 159, 162, and 176, the web has no files,
/// arguments, environment, sleep, user `extern`, `spawn`, or sockets. See
/// "WebAssembly" in docs/compiler.md.
#[test]
fn web_programs_match_run() {
    const HOST_ONLY: [&str; 10] = [
        "channel_threads",
        "spawn",
        "spawn_overflow",
        "spawn_trap",
        "std_args_env",
        "std_fs",
        "std_http",
        "std_net",
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

/// `spawn` and `std.net` are E0907 under `--target web`, by decision 176.
#[test]
fn web_rejects_spawn() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("web_spawn");
    fs::create_dir_all(&dir).expect("cannot make directory");
    for (name, says) in [("spawn", "`spawn`"), ("std_net", "`std.net`"), ("std_http", "`std.http`")]
    {
        let file = format!("{name}.lcd");
        fs::copy(repo().join("tests/run").join(&file), dir.join(&file)).expect("cannot copy");
        let output = lucid(&dir, &["build", "--target", "web", &file]);
        assert_eq!(output.status.code(), Some(1));
        let error = text(&output.stderr);
        assert!(error.contains(&format!("error E0907: {says} cannot run")), "{error}");
    }
}
