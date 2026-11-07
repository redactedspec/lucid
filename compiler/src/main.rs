//! The `lucid` command. See docs/compiler.md.

#[allow(dead_code)] // Later stages read the tree that the parser builds now.
mod ast;
mod check;
mod context;
mod derive;
mod diag;
#[cfg(test)]
mod fuzz;
mod lexer;
mod manifest;
mod mir;
mod mono;
mod native;
mod ownership;
mod parser;
mod resolve;
mod wasm;

use diag::{Diagnostic, Source};
use resolve::{load, package_files, package_root};
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "usage: lucid run [PATH [ARGUMENTS]]\n       lucid build [--target web] [--json] [PATH]\n       lucid check [--json] [--costs | --unsafe] [PATH]\n       lucid test [--json] [PATH]\n       lucid fix [--json] [PATH]\n       lucid explain [--json] CODE\n       lucid context [--json] MODULE\n       lucid fmt [--check] [PATH]\nPATH is a file or a package directory, by default the current directory.";

/// Set for the child process of `lucid test`, to the place of its first test.
const TEST_FROM: &str = "LUCID_TEST_FROM";

/// Set for the child process of `lucid fmt`, which runs the formatter.
const FMT_CHILD: &str = "LUCID_FMT_CHILD";

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
    let Some((&command, mut rest)) = args.split_first() else { return usage() };
    match (command, rest) {
        ("explain", [code]) if !code.starts_with('-') => return explain(code, false),
        ("explain", ["--json", code]) => return explain(code, true),
        ("explain", _) => return usage(),
        ("context", [module]) if !module.starts_with('-') => return context(module, false),
        ("context", ["--json", module]) => return context(module, true),
        ("context", _) => return usage(),
        ("fmt", _) => return fmt(rest),
        _ => {}
    }
    // What follows the path of `lucid run` belongs to the program.
    if command == "run" && rest.first().is_some_and(|path| !path.starts_with('-')) {
        lucid_rt::set_args(rest[1..].iter().map(|arg| (*arg).to_string()).collect());
        rest = &rest[..1];
    }
    let (mut rest, path) = match rest.split_last() {
        Some((path, flags)) if !path.starts_with('-') => (flags, *path),
        _ => (rest, "."),
    };
    let build = command == "build";
    // `--target web` comes first, so the flags below match as they do without it.
    let web = build && rest.starts_with(&["--target", "web"]);
    if web {
        rest = &rest[2..];
    }
    let (run, json, costs) = match (command, rest) {
        ("test", []) => return test(path, false),
        ("test", ["--json"]) => return test(path, true),
        ("fix", []) => return fix(path, false),
        ("fix", ["--json"]) => return fix(path, true),
        ("check", ["--unsafe"]) => return unsafe_sites(path, false),
        ("check", ["--json", "--unsafe"]) => return unsafe_sites(path, true),
        ("run" | "build", []) => (true, false, false),
        ("build", ["--json"]) => (true, true, false),
        ("check", []) => (false, false, false),
        ("check", ["--json"]) => (false, true, false),
        ("check", ["--costs"]) => (false, false, true),
        ("check", ["--json", "--costs"]) => (false, true, true),
        _ => return usage(),
    };
    let (source, file, mut diags) = load(path, run);
    if diags.is_empty()
        && let Some(file) = &file
    {
        let (mut program, errors) = check::check(file, &source, !run);
        diags = errors;
        if diags.is_empty() {
            diags = mono::run(&mut program);
        }
        if diags.is_empty() && (run || costs) {
            ownership::run(&mut program, std::env::var_os("LUCID_NAIVE").is_none());
        }
        if costs && diags.is_empty() {
            let found = ownership::costs(&mut program);
            emit(std::io::stdout(), &ownership::render_costs(&source, &found, json));
            return ExitCode::SUCCESS;
        }
        if web && diags.is_empty() {
            match wasm::script(&program, &source) {
                Ok(script) => return write_script(&script, &out_name(path, "js")),
                Err(errors) => diags = errors,
            }
        } else if build && diags.is_empty() {
            match native::object(&program, &source) {
                Ok(object) => return link(&object, &out_name(path, "exe")),
                Err(errors) => diags = errors,
            }
        } else if run && diags.is_empty() {
            diags = native::run(&program, &source, STACK_BYTES).err().unwrap_or_default();
        } else if !run && !costs {
            // `check` reports errors in the program. An unbuilt feature is not one.
            // `--costs` needs compiled code, so there it stays an error.
            diags.retain(|d| d.code != "E0901");
        }
    }
    report_errors(&source, &diags, json);
    if diags.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// What `lucid build` writes to the current directory: named after the file,
/// or after the directory of the package, with this extension.
fn out_name(path: &str, extension: &str) -> String {
    let path = std::path::Path::new(path);
    let stem = if path.is_dir() {
        let root = std::fs::canonicalize(format!("{}.", package_root(&path.display().to_string())));
        root.ok().and_then(|root| root.file_name().map(|name| name.to_os_string()))
    } else {
        path.file_stem().map(|name| name.to_os_string())
    };
    format!("{}.{extension}", stem.unwrap_or("main".into()).display())
}

/// Writes the output of `lucid build --target web`. As in `link`, a failure
/// is in the system, so it has no code.
fn write_script(script: &str, name: &str) -> ExitCode {
    match std::fs::write(name, script) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            emit(std::io::stderr(), &format!("lucid: cannot write `{name}`: {e}\n"));
            ExitCode::FAILURE
        }
    }
}

/// Links an object file from `native::object` with the embedded runtime, into
/// `exe`. A failure here is in the system, not the program, so it has no code.
#[cfg(windows)]
fn link(object: &[u8], exe: &str) -> ExitCode {
    const RUNTIME: &[u8] = include_bytes!(env!("LUCID_RT_LIB"));
    let fail = |problem: String| {
        emit(std::io::stderr(), &format!("lucid: {problem}\n"));
        ExitCode::FAILURE
    };
    let Some(linker) = find_msvc_tools::find_tool("x86_64-pc-windows-msvc", "link.exe") else {
        return fail(
            "cannot find the MSVC linker: install Visual Studio Build Tools, with the C++ tools"
                .into(),
        );
    };
    let dir = std::env::temp_dir().join(format!("lucid-build-{}", std::process::id()));
    let linked = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join("main.obj"), object))
        .and_then(|()| std::fs::write(dir.join("lucid_rt.lib"), RUNTIME))
        .and_then(|()| {
            let mut command = linker.to_command();
            command.args(["/NOLOGO", "/SUBSYSTEM:CONSOLE"]);
            command.arg(format!("/STACK:{}", lucid_rt::EXE_STACK_BYTES));
            command.arg(format!("/OUT:{exe}"));
            command.arg(dir.join("main.obj")).arg(dir.join("lucid_rt.lib"));
            command.args(env!("LUCID_RT_LIBS").split_whitespace()).output()
        });
    let _ = std::fs::remove_dir_all(&dir);
    match linked {
        Ok(output) if output.status.success() => ExitCode::SUCCESS,
        Ok(output) => {
            let said =
                String::from_utf8_lossy(&output.stdout) + String::from_utf8_lossy(&output.stderr);
            fail(format!("linking failed:\n{said}"))
        }
        Err(e) => fail(format!("cannot link: {e}")),
    }
}

#[cfg(not(windows))]
fn link(_object: &[u8], _exe: &str) -> ExitCode {
    emit(std::io::stderr(), "lucid: `lucid build` works only on Windows until M8\n");
    ExitCode::FAILURE
}

/// Reports compile errors, as data on stdout with `json`.
fn report_errors(source: &Source, diags: &[Diagnostic], json: bool) {
    if json {
        emit(std::io::stdout(), &diag::render_json(source, diags));
    } else {
        emit(std::io::stderr(), &diag::render_text(source, diags));
    }
}

/// `lucid check --unsafe`: each `unsafe` block and `extern` task, in file order.
fn unsafe_sites(path: &str, json: bool) -> ExitCode {
    let (source, file, mut diags) = load(path, false);
    let mut sites = Vec::new();
    if diags.is_empty()
        && let Some(file) = &file
    {
        let (program, errors) = check::check(file, &source, true);
        diags = errors;
        // As in `check`: an unbuilt feature is not an error in the program.
        diags.retain(|d| d.code != "E0901");
        sites = program.unsafe_sites;
    }
    if !diags.is_empty() {
        report_errors(&source, &diags, json);
        return ExitCode::FAILURE;
    }
    sites.sort_by_key(|(_, span)| span.start);
    let lines = diag::Lines::new(&source);
    // `std` ships with the compiler, so its sites are not the program's to audit.
    sites.retain(|(_, span)| !lines.place(span.start).0.starts_with("@std/"));
    let items = sites.iter().map(|(kind, span)| {
        let (file, line, column) = lines.place(span.start);
        if json {
            let file = diag::json_string(file);
            format!("{{\"kind\":\"{kind}\",\"file\":{file},\"line\":{line},\"column\":{column}}}")
        } else {
            format!("{file}:{line}:{column}: unsafe: {kind}\n")
        }
    });
    let report: String =
        if json { format!("[{}]\n", items.collect::<Vec<_>>().join(",")) } else { items.collect() };
    emit(std::io::stdout(), &report);
    ExitCode::SUCCESS
}

/// Every error code, with its cause, an example, and a fix.
const ERRORS: &str = include_str!("../../docs/errors.md");

/// `lucid explain`: the entry of one error code in docs/errors.md.
fn explain(code: &str, json: bool) -> ExitCode {
    let code = code.to_ascii_uppercase();
    let entries = ERRORS.split("\n## ").skip(1);
    let entry = entries.map(str::trim_end).find_map(|entry| {
        let (heading, body) = entry.split_once('\n').unwrap_or((entry, ""));
        Some((heading.strip_prefix(code.as_str())?.strip_prefix(": ")?, entry, body))
    });
    let Some((title, entry, body)) = entry else {
        emit(std::io::stderr(), &format!("lucid: no error code `{code}`, see docs/errors.md\n"));
        return ExitCode::FAILURE;
    };
    if !json {
        emit(std::io::stdout(), &format!("{entry}\n"));
        return ExitCode::SUCCESS;
    }
    let cause = body.split("```lucid").next().and_then(|cause| cause.split("\nFix: ").next());
    let example = body.split("```lucid\n").nth(1).and_then(|block| block.split("```").next());
    let fix = body.lines().find_map(|line| line.strip_prefix("Fix: "));
    let field = |text: Option<&str>| diag::json_string(text.unwrap_or("").trim_matches(' '));
    let cause = diag::json_string(cause.unwrap_or("").trim());
    emit(
        std::io::stdout(),
        &format!(
            "{{\"code\":{},\"title\":{},\"cause\":{cause},\"example\":{},\"fix\":{}}}\n",
            diag::json_string(&code),
            diag::json_string(title),
            field(example),
            field(fix),
        ),
    );
    ExitCode::SUCCESS
}

/// `lucid fix`: makes the fix of every error that has one, and lists them. Two
/// fixes that touch the same text would conflict, so only the first is made,
/// and another run makes the next. `std` belongs to the compiler, so it stays.
fn fix(path: &str, json: bool) -> ExitCode {
    let (source, file, mut diags) = load(path, false);
    if diags.is_empty()
        && let Some(file) = &file
    {
        let (mut program, errors) = check::check(file, &source, true);
        diags = errors;
        if diags.is_empty() {
            diags = mono::run(&mut program);
        }
    }
    diags.sort_by_key(|d| d.span.start);
    let files = &source.files;
    let mut taken: Vec<diag::Span> = Vec::new();
    let mut made = Vec::new();
    for d in &diags {
        let Some(fix) = d.fixes.first() else { continue };
        let file = diag::file_index(files, fix.edits[0].0.start);
        let clash = |span: &diag::Span| {
            let overlap = |other: &diag::Span| {
                span.start == other.start || (span.start < other.end && other.start < span.end)
            };
            taken.iter().any(overlap)
        };
        if files[file].path.starts_with("@std/") || fix.edits.iter().any(|(span, _)| clash(span)) {
            continue;
        }
        taken.extend(fix.edits.iter().map(|(span, _)| *span));
        made.push((d, fix, file));
    }
    // Each file's text ends with the line break that `Source::add` put after it.
    let mut edits: Vec<(usize, diag::Span, &str)> = made
        .iter()
        .flat_map(|(_, fix, file)| {
            fix.edits.iter().map(|(span, text)| (*file, *span, text.as_str()))
        })
        .collect();
    edits.sort_by_key(|&(file, span, _)| (file, std::cmp::Reverse(span.start)));
    let mut at = 0;
    while at < edits.len() {
        let file = edits[at].0;
        let start = files[file].start;
        let end = files.get(file + 1).map_or(source.text.len(), |next| next.start) - 1;
        let mut text = source.text[start..end].to_string();
        while let Some(&(_, span, new)) = edits.get(at).filter(|edit| edit.0 == file) {
            text.replace_range(span.start - start..span.end - start, new);
            at += 1;
        }
        if let Err(e) = std::fs::write(files[file].path, text) {
            emit(std::io::stderr(), &format!("lucid: cannot write `{}`: {e}\n", files[file].path));
            return ExitCode::FAILURE;
        }
    }
    let lines = diag::Lines::new(&source);
    let items = made.iter().map(|(d, fix, _)| {
        let (file, line, column) = lines.place(d.span.start);
        let description = diag::plain(&fix.description);
        if json {
            let (file, description) = (diag::json_string(file), diag::json_string(&description));
            format!(
                "{{\"code\":\"{}\",\"description\":{description},\"file\":{file},\"line\":{line},\"column\":{column}}}",
                d.code
            )
        } else {
            format!("{file}:{line}:{column}: fixed {}: {description}\n", d.code)
        }
    });
    let report: String =
        if json { format!("[{}]\n", items.collect::<Vec<_>>().join(",")) } else { items.collect() };
    emit(std::io::stdout(), &report);
    ExitCode::SUCCESS
}

/// `lucid context`: the public API of a file, of every file of a package, or of
/// a `std` module, as `std.fs`.
fn context(name: &str, json: bool) -> ExitCode {
    let std = name.strip_prefix("std.").map(resolve::std_module);
    let files = match std {
        Some(Some(text)) => vec![(name.to_string(), Ok(text.to_string()))],
        Some(None) => {
            emit(std::io::stderr(), &format!("lucid: `std` has no module `{name}`\n"));
            return ExitCode::FAILURE;
        }
        None if std::path::Path::new(name).is_dir() => {
            let read = |file: String| (file.clone(), std::fs::read_to_string(&file));
            package_files(name).into_iter().map(read).collect()
        }
        None => vec![(name.to_string(), std::fs::read_to_string(name))],
    };
    let mut modules = Vec::new();
    for (file, text) in files {
        match text {
            Ok(text) => modules.push((file, context::items(&text))),
            Err(e) => {
                emit(std::io::stderr(), &format!("lucid: cannot read `{file}`: {e}\n"));
                return ExitCode::FAILURE;
            }
        }
    }
    if modules.is_empty() {
        emit(std::io::stderr(), &format!("lucid: `{name}` has no `src` with `.lcd` files\n"));
        return ExitCode::FAILURE;
    }
    emit(std::io::stdout(), &context::render(&modules, json));
    ExitCode::SUCCESS
}

/// `lucid fmt`: formats a file, or every file of a package, with the Lucid
/// program `tools/fmt.lcd`. A file with a lexer error is reported and left
/// alone. The formatter runs in a child process, so that a trap in it still
/// leaves this process to check each changed file: if its tokens changed, that
/// is a formatter bug, and the old text goes back. See "`lucid fmt`" in
/// docs/compiler.md.
fn fmt(rest: &[&str]) -> ExitCode {
    if std::env::var_os(FMT_CHILD).is_some() {
        return run_formatter(rest);
    }
    let (check, path) = match rest {
        [] => (false, "."),
        ["--check"] => (true, "."),
        [path] if !path.starts_with('-') => (false, *path),
        ["--check", path] if !path.starts_with('-') => (true, *path),
        _ => return usage(),
    };
    let files = if std::path::Path::new(path).is_dir() {
        package_files(path)
    } else {
        vec![path.to_string()]
    };
    if files.is_empty() {
        emit(
            std::io::stderr(),
            &format!(
                "lucid: `{path}` has no `src` with `.lcd` files
"
            ),
        );
        return ExitCode::FAILURE;
    }
    let mut failed = false;
    let mut kept = Vec::new();
    for file in files {
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) => {
                emit(
                    std::io::stderr(),
                    &format!(
                        "lucid: cannot read `{file}`: {e}
"
                    ),
                );
                failed = true;
                continue;
            }
        };
        let (tokens, diags) = lexer::lex(&text);
        if diags.is_empty() {
            kept.push((file, text, tokens));
        } else {
            let mut source = Source::default();
            source.add(file.leak(), &text);
            report_errors(&source, &diags, false);
            failed = true;
        }
    }
    if kept.is_empty() {
        return ExitCode::FAILURE;
    }
    let child = std::env::current_exe().and_then(|lucid| {
        let mut command = std::process::Command::new(lucid);
        command.arg("fmt").args(check.then_some("--check")).env(FMT_CHILD, "1");
        command.args(kept.iter().map(|(file, ..)| file)).status()
    });
    let mut bug = false;
    for (file, old, tokens) in &kept {
        // Bytes that are not UTF-8 are a change too.
        let Ok(new) = std::fs::read(file) else { continue };
        if only_space_changed(old, tokens, &new) {
            continue;
        }
        bug = true;
        let restored = match std::fs::write(file, old) {
            Ok(()) => "its old text is back",
            Err(_) => "and its old text could not be written back",
        };
        let problem = format!(
            "lucid: bug in `lucid fmt`: it changed more than space in `{file}`, {restored}\n"
        );
        emit(std::io::stderr(), &problem);
    }
    if bug {
        return ExitCode::from(101);
    }
    match child {
        Ok(status) if status.success() && !failed => ExitCode::SUCCESS,
        Ok(status) if status.code() == Some(101) => ExitCode::from(101),
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            emit(
                std::io::stderr(),
                &format!(
                    "lucid: cannot start the formatter: {e}
"
                ),
            );
            ExitCode::FAILURE
        }
    }
}

/// True if two token lists differ only in their spans.
/// True if `new` is `old` with only space and line breaks changed, as the
/// formatter promises: the same tokens, and the same other characters, which
/// covers comments. A byte order mark may go. Bytes that are not UTF-8 are a
/// change.
fn only_space_changed(old: &str, tokens: &[lexer::Token], new: &[u8]) -> bool {
    let Ok(new) = std::str::from_utf8(new) else { return false };
    let solid = |text: &str| {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        text.chars().filter(|c| !c.is_whitespace()).collect::<String>()
    };
    let (after, diags) = lexer::lex(new);
    new == old || (diags.is_empty() && same_tokens(tokens, &after) && solid(new) == solid(old))
}

fn same_tokens(a: &[lexer::Token], b: &[lexer::Token]) -> bool {
    use lexer::{TextPart, TokenKind};
    let same_part = |pair: (&TextPart, &TextPart)| match pair {
        (TextPart::Expr(x), TextPart::Expr(y)) => same_tokens(x, y),
        (x, y) => x == y,
    };
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (&x.kind, &y.kind) {
            (TokenKind::Text(x), TokenKind::Text(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(same_part)
            }
            (x, y) => x == y,
        })
}

/// The child of `lucid fmt`: compiles `tools/fmt.lcd` and runs it on `args`.
fn run_formatter(args: &[&str]) -> ExitCode {
    lucid_rt::set_args(args.iter().map(|arg| (*arg).to_string()).collect());
    let (source, file, mut diags) = load(resolve::FMT_PATH, true);
    if diags.is_empty()
        && let Some(file) = &file
    {
        let (mut program, errors) = check::check(file, &source, false);
        diags = errors;
        if diags.is_empty() {
            diags = mono::run(&mut program);
        }
        if diags.is_empty() {
            ownership::run(&mut program, true);
            diags = native::run(&program, &source, STACK_BYTES).err().unwrap_or_default();
        }
    }
    report_errors(&source, &diags, false);
    if diags.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// The result of one test.
struct Outcome {
    failed: bool,
    /// What it printed. If it failed, also what stopped it.
    output: String,
}

/// The results of one file's tests.
struct FileTests {
    source: Source,
    tests: Vec<mir::TestCase>,
    outcomes: Vec<Outcome>,
}

/// `lucid test`, on a file or on every file of a package.
fn test(path: &str, json: bool) -> ExitCode {
    let package = std::path::Path::new(path).is_dir();
    let mut files = vec![path.to_string()];
    if package {
        let (source, _, diags) = load(path, false);
        if !diags.is_empty() {
            report_errors(&source, &diags, json);
            return ExitCode::FAILURE;
        }
        files = package_files(path);
    }
    // All files compile first, so an error in one wastes no test run.
    let mut results = Vec::new();
    for file in &files {
        match compile_tests(file, json) {
            Ok(result) => results.push(result),
            Err(code) => return code,
        }
    }
    for (file, result) in files.iter().zip(&mut results) {
        match run_tests(file, result.tests.len(), json) {
            Ok(outcomes) => result.outcomes = outcomes,
            Err(problem) => {
                if json {
                    emit(std::io::stdout(), &problem);
                } else {
                    emit(std::io::stderr(), &problem);
                }
                return ExitCode::FAILURE;
            }
        }
    }
    let outcomes = || results.iter().flat_map(|result| &result.outcomes);
    let report = if json {
        let items: Vec<String> = results.iter().flat_map(render_tests_json).collect();
        format!("[{}]\n", items.join(","))
    } else {
        let mut report = String::new();
        for (file, result) in files.iter().zip(&results) {
            if package && !result.tests.is_empty() {
                report += &format!("{file}\n");
            }
            report += &render_tests(result);
        }
        let failed = outcomes().filter(|outcome| outcome.failed).count();
        report + &format!("{} tests, {failed} failed\n", outcomes().count())
    };
    emit(std::io::stdout(), &report);
    if outcomes().any(|outcome| outcome.failed) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// Checks one file and lists its tests, for `run_tests`. A trap ends a process,
/// so tests run in a child, which goes on after a failure in a new child. That
/// compiles once more per failure. A child runs its tests here, and ends with
/// `Err`, which holds the exit code, as it does after a report of a compile error.
fn compile_tests(path: &str, json: bool) -> Result<FileTests, ExitCode> {
    let (source, file, mut diags) = load(path, false);
    let mut tests = Vec::new();
    if diags.is_empty()
        && let Some(file) = &file
    {
        let (mut program, errors) = check::check(file, &source, true);
        diags = errors;
        if diags.is_empty() {
            diags = mono::run(&mut program);
        }
        if diags.is_empty() {
            if let Ok(from) = std::env::var(TEST_FROM) {
                ownership::run(&mut program, std::env::var_os("LUCID_NAIVE").is_none());
                let from = from.parse().unwrap_or(usize::MAX);
                diags = native::run_tests(&program, &source, STACK_BYTES, from)
                    .err()
                    .unwrap_or_default();
                if !diags.is_empty() {
                    report_errors(&source, &diags, json);
                    return Err(ExitCode::FAILURE);
                }
                return Err(ExitCode::SUCCESS);
            }
            tests = std::mem::take(&mut program.tests);
        }
    }
    if !diags.is_empty() {
        report_errors(&source, &diags, json);
        return Err(ExitCode::FAILURE);
    }
    Ok(FileTests { source, tests, outcomes: Vec::new() })
}

/// Runs `count` tests in child processes. Each child gives a NUL before each
/// test it starts and after its last, and stops at the first that fails. So a
/// test that ends the process, even with exit code 0, shows as not finished.
/// An error means that a child did not start a test, and holds what it said.
fn run_tests(path: &str, count: usize, json: bool) -> Result<Vec<Outcome>, String> {
    let lucid = std::env::current_exe().map_err(|e| format!("lucid: {e}\n"))?;
    let mut outcomes = Vec::with_capacity(count);
    while outcomes.len() < count {
        let flags: &[&str] = if json { &["test", "--json"] } else { &["test"] };
        let child = std::process::Command::new(&lucid)
            .args(flags)
            .arg(path)
            .env(TEST_FROM, outcomes.len().to_string())
            .output()
            .map_err(|e| format!("lucid: cannot start a test process: {e}\n"))?;
        let stdout = String::from_utf8_lossy(&child.stdout);
        let stderr = String::from_utf8_lossy(&child.stderr);
        let mut parts: Vec<&str> = stdout.split('\0').skip(1).collect();
        if parts.is_empty() {
            return Err(format!("{stdout}{stderr}"));
        }
        let finished = child.status.success() && parts.len() == count - outcomes.len() + 1;
        if finished {
            parts.pop();
        }
        for (place, part) in parts.iter().enumerate() {
            let failed = !finished && place + 1 == parts.len();
            let printed = part.strip_prefix('\n').unwrap_or(part);
            let why = match failed {
                true if stderr.is_empty() => "the test ended the program\n",
                true => stderr.as_ref(),
                false => "",
            };
            outcomes.push(Outcome { failed, output: format!("{printed}{why}") });
        }
    }
    Ok(outcomes)
}

fn render_tests(result: &FileTests) -> String {
    let mut report = String::new();
    for (test, outcome) in result.tests.iter().zip(&result.outcomes) {
        report += &format!("{} {}\n", if outcome.failed { "FAIL" } else { "ok" }, test.name);
        if outcome.failed {
            report.extend(outcome.output.lines().map(|line| format!("    {line}\n")));
        }
    }
    report
}

fn render_tests_json(result: &FileTests) -> Vec<String> {
    let lines = diag::Lines::new(&result.source);
    let items = result.tests.iter().zip(&result.outcomes);
    items
        .map(|(test, outcome)| {
            let name = diag::json_string(&test.name);
            let (file, line, column) = lines.place(test.span.start);
            let file = diag::json_string(file);
            let (passed, output) = (!outcome.failed, diag::json_string(&outcome.output));
            format!(
                "{{\"name\":{name},\"passed\":{passed},\"file\":{file},\"line\":{line},\"column\":{column},\"output\":{output}}}"
            )
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_net_allows_only_space() {
        let old = "let a = [1, 2] # note\n";
        let kept = |new: &[u8]| only_space_changed(old, &lexer::lex(old).0, new);
        assert!(kept(b"let  a = [1,2]   # note\r\n"));
        let marked = format!("\u{feff}{old}");
        assert!(only_space_changed(&marked, &lexer::lex(&marked).0, old.as_bytes()));
        assert!(!kept(b"let a = [1, 2] # nte\n"), "a comment changed");
        assert!(!kept(b"let a = [12] # note\n"), "two tokens joined");
        assert!(!kept(b"let a = [1, 2] # not\xC3\n"), "bytes not UTF-8");
    }
}
