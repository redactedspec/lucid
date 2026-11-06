//! The `lucid` command. See docs/compiler.md.

#[allow(dead_code)] // Later stages read the tree that the parser builds now.
mod ast;
mod check;
mod derive;
mod diag;
mod lexer;
mod mir;
mod mono;
mod native;
mod ownership;
mod parser;

use diag::{Diagnostic, Span};
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "usage: lucid run FILE\n       lucid check [--json] [--costs | --unsafe] FILE\n       lucid test [--json] FILE";

/// Set for the child process of `lucid test`, to the place of its first test.
const TEST_FROM: &str = "LUCID_TEST_FROM";

/// What every file sees. The checker treats these like the file's own.
const PRELUDE: &str = "trait Error
    task error_message(self) -> text
";

/// Debug builds use large stack frames, and `MAX_DEPTH` levels of nesting must fit.
const STACK_BYTES: usize = 256 * 1024 * 1024;

fn main() -> ExitCode {
    // A path that is not Unicode becomes E0001 instead of a panic in `args()`.
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.display().to_string()).collect();
    match std::thread::Builder::new().stack_size(STACK_BYTES).spawn(move || cli(&args)) {
        Ok(thread) => thread.join().unwrap_or(ExitCode::from(101)),
        Err(e) => {
            emit(std::io::stderr(), &format!("lucid: cannot start compiler thread: {e}\n"));
            ExitCode::from(101)
        }
    }
}

fn cli(args: &[String]) -> ExitCode {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (run, json, costs, path) = match args.as_slice() {
        ["test", path] if !path.starts_with('-') => return test(path, false),
        ["check", "--unsafe", path] if !path.starts_with('-') => return unsafe_sites(path, false),
        ["check", "--json", "--unsafe", path] if !path.starts_with('-') => {
            return unsafe_sites(path, true);
        }
        ["test", "--json", path] if !path.starts_with('-') => return test(path, true),
        ["run", path] => (true, false, false, *path),
        ["check", path] => (false, false, false, *path),
        ["check", "--json", path] => (false, true, false, *path),
        ["check", "--costs", path] => (false, false, true, *path),
        ["check", "--json", "--costs", path] => (false, true, true, *path),
        _ => return usage(),
    };
    if path.starts_with('-') {
        return usage();
    }
    let (src, file, mut diags) = load(path);
    if diags.is_empty()
        && let Some(file) = &file
    {
        let (mut program, errors) = check::check(file, &src, !run);
        diags = errors;
        if diags.is_empty() {
            diags = mono::run(&mut program);
        }
        if diags.is_empty() && (run || costs) {
            ownership::run(&mut program, std::env::var_os("LUCID_NAIVE").is_none());
        }
        if costs && diags.is_empty() {
            let found = ownership::costs(&mut program);
            emit(std::io::stdout(), &ownership::render_costs(path, &src, &found, json));
            return ExitCode::SUCCESS;
        }
        if run && diags.is_empty() {
            diags = native::run(&program, path, &src, STACK_BYTES).err().unwrap_or_default();
        } else if !run && !costs {
            // `check` reports errors in the program. An unbuilt feature is not one.
            // `--costs` needs compiled code, so there it stays an error.
            diags.retain(|d| d.code != "E0901");
        }
    }
    if json {
        emit(std::io::stdout(), &diag::render_json(path, &src, &diags));
    } else {
        emit(std::io::stderr(), &diag::render_text(path, &src, &diags));
    }
    if diags.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// `lucid check --unsafe`: each `unsafe` block and `extern` task, in file order.
fn unsafe_sites(path: &str, json: bool) -> ExitCode {
    let (src, file, mut diags) = load(path);
    let mut sites = Vec::new();
    if diags.is_empty()
        && let Some(file) = &file
    {
        let (program, errors) = check::check(file, &src, true);
        diags = errors;
        sites = program.unsafe_sites;
    }
    if !diags.is_empty() {
        if json {
            emit(std::io::stdout(), &diag::render_json(path, &src, &diags));
        } else {
            emit(std::io::stderr(), &diag::render_text(path, &src, &diags));
        }
        return ExitCode::FAILURE;
    }
    sites.sort_by_key(|(_, span)| span.start);
    let lines = diag::Lines::new(&src);
    let file = diag::json_string(path);
    let items = sites.iter().map(|(kind, span)| {
        let (line, column) = lines.position(span.start);
        if json {
            format!("{{\"kind\":\"{kind}\",\"file\":{file},\"line\":{line},\"column\":{column}}}")
        } else {
            format!("{path}:{line}:{column}: unsafe: {kind}\n")
        }
    });
    let report: String =
        if json { format!("[{}]\n", items.collect::<Vec<_>>().join(",")) } else { items.collect() };
    emit(std::io::stdout(), &report);
    ExitCode::SUCCESS
}

/// The result of one test.
struct Outcome {
    failed: bool,
    /// What it printed. If it failed, also what stopped it.
    output: String,
}

/// `lucid test`. A trap ends a process, so tests run in a child, which goes on
/// after a failure in a new child. That compiles once more per failure.
fn test(path: &str, json: bool) -> ExitCode {
    let (src, file, mut diags) = load(path);
    let mut tests = Vec::new();
    if diags.is_empty()
        && let Some(file) = &file
    {
        let (mut program, errors) = check::check(file, &src, true);
        diags = errors;
        if diags.is_empty() {
            diags = mono::run(&mut program);
        }
        if diags.is_empty() {
            if let Ok(from) = std::env::var(TEST_FROM) {
                ownership::run(&mut program, std::env::var_os("LUCID_NAIVE").is_none());
                let from = from.parse().unwrap_or(usize::MAX);
                diags = native::run_tests(&program, path, &src, STACK_BYTES, from)
                    .err()
                    .unwrap_or_default();
                emit(std::io::stderr(), &diag::render_text(path, &src, &diags));
                return if diags.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE };
            }
            tests = std::mem::take(&mut program.tests);
        }
    }
    if !diags.is_empty() {
        if json {
            emit(std::io::stdout(), &diag::render_json(path, &src, &diags));
        } else {
            emit(std::io::stderr(), &diag::render_text(path, &src, &diags));
        }
        return ExitCode::FAILURE;
    }
    let outcomes = match run_tests(path, tests.len()) {
        Ok(outcomes) => outcomes,
        Err(problem) => {
            emit(std::io::stderr(), &problem);
            return ExitCode::FAILURE;
        }
    };
    let report = if json {
        render_tests_json(path, &src, &tests, &outcomes)
    } else {
        render_tests(&tests, &outcomes)
    };
    emit(std::io::stdout(), &report);
    if outcomes.iter().any(|outcome| outcome.failed) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Runs `count` tests in child processes. Each child gives a NUL before each
/// test it starts, and stops at the first that does not pass. An error means
/// that a child did not start a test, and holds what it said.
fn run_tests(path: &str, count: usize) -> Result<Vec<Outcome>, String> {
    let lucid = std::env::current_exe().map_err(|e| format!("lucid: {e}\n"))?;
    let mut outcomes = Vec::with_capacity(count);
    while outcomes.len() < count {
        let child = std::process::Command::new(&lucid)
            .args(["test", path])
            .env(TEST_FROM, outcomes.len().to_string())
            .output()
            .map_err(|e| format!("lucid: cannot start a test process: {e}\n"))?;
        let stdout = String::from_utf8_lossy(&child.stdout);
        let stderr = String::from_utf8_lossy(&child.stderr);
        let parts: Vec<&str> = stdout.split('\0').skip(1).collect();
        if parts.is_empty() {
            return Err(stderr.into_owned());
        }
        for (place, part) in parts.iter().enumerate() {
            let failed = !child.status.success() && place + 1 == parts.len();
            let printed = part.strip_prefix('\n').unwrap_or(part);
            let output = format!("{printed}{}", if failed { stderr.as_ref() } else { "" });
            outcomes.push(Outcome { failed, output });
        }
    }
    Ok(outcomes)
}

fn render_tests(tests: &[mir::TestCase], outcomes: &[Outcome]) -> String {
    let mut report = String::new();
    for (test, outcome) in tests.iter().zip(outcomes) {
        report += &format!("{} {}\n", if outcome.failed { "FAIL" } else { "ok" }, test.name);
        if outcome.failed {
            report.extend(outcome.output.lines().map(|line| format!("    {line}\n")));
        }
    }
    let failed = outcomes.iter().filter(|outcome| outcome.failed).count();
    report + &format!("{} tests, {failed} failed\n", tests.len())
}

fn render_tests_json(
    path: &str,
    src: &str,
    tests: &[mir::TestCase],
    outcomes: &[Outcome],
) -> String {
    let lines = diag::Lines::new(src);
    let file = diag::json_string(path);
    let items: Vec<String> = tests
        .iter()
        .zip(outcomes)
        .map(|(test, outcome)| {
            let name = diag::json_string(&test.name);
            let (line, column) = lines.position(test.span.start);
            let (passed, output) = (!outcome.failed, diag::json_string(&outcome.output));
            format!(
                "{{\"name\":{name},\"passed\":{passed},\"file\":{file},\"line\":{line},\"column\":{column},\"output\":{output}}}"
            )
        })
        .collect();
    format!("[{}]\n", items.join(","))
}

fn usage() -> ExitCode {
    emit(std::io::stderr(), &format!("{USAGE}\n"));
    ExitCode::from(2)
}

/// Writes output. If the reader is gone there is no one to tell, and the exit
/// code still reports the result, so a failure here is ignored.
fn emit(mut out: impl Write, text: &str) {
    let _ = out.write_all(text.as_bytes());
}

/// Reads and parses a file. Returns the text that spans refer to.
fn load(path: &str) -> (String, Option<ast::File>, Vec<Diagnostic>) {
    let start = Span { start: 0, end: 0 };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            let diag = Diagnostic::new("E0001", format!("cannot read `{path}`: {e}"), start);
            return (String::new(), None, vec![diag]);
        }
    };
    let src = match String::from_utf8(bytes) {
        Ok(src) => src,
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            let span = Span { start: valid, end: valid };
            let diag = Diagnostic::new("E0002", "file is not valid UTF-8", span);
            let prefix = String::from_utf8_lossy(&e.as_bytes()[..valid]).into_owned();
            return (prefix, None, vec![diag]);
        }
    };
    let (tokens, mut diags) = lexer::lex(&src);
    let (mut file, parse_diags) = parser::parse(tokens);
    diag::add_unless_same_line(&src, &mut diags, parse_diags);
    let (prelude, _) = parser::parse(lexer::lex(PRELUDE).0);
    file.stmts.splice(0..0, prelude.stmts);
    (src, Some(file), diags)
}
