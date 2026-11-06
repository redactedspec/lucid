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

const USAGE: &str = "usage: lucid run FILE\n       lucid check [--json] [--costs] FILE";

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
        let (mut program, errors) = check::check(file);
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
    let (file, parse_diags) = parser::parse(tokens);
    diag::add_unless_same_line(&src, &mut diags, parse_diags);
    (src, Some(file), diags)
}
