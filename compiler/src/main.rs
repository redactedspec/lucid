//! The `lucid` command. See docs/compiler.md.

#[allow(dead_code)] // Later stages read the tree that the parser builds now.
mod ast;
mod diag;
mod lexer;
mod native;
mod parser;

use diag::{Diagnostic, Span};
use std::process::ExitCode;

const USAGE: &str = "usage: lucid run FILE\n       lucid check [--json] FILE";

/// Debug builds use large stack frames, and `MAX_DEPTH` levels of nesting must fit.
const STACK_BYTES: usize = 256 * 1024 * 1024;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = std::thread::Builder::new().stack_size(STACK_BYTES).spawn(move || cli(&args));
    match cli.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        _ => ExitCode::from(101),
    }
}

fn cli(args: &[String]) -> ExitCode {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (run, json, path) = match args.as_slice() {
        ["run", path] => (true, false, *path),
        ["check", path] => (false, false, *path),
        ["check", "--json", path] => (false, true, *path),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let (src, file, mut diags) = load(path);
    if run
        && diags.is_empty()
        && let Some(Err(errors)) = file.as_ref().map(native::run)
    {
        diags = errors;
    }
    if json {
        print!("{}", diag::render_json(path, &src, &diags));
    } else {
        eprint!("{}", diag::render_text(path, &src, &diags));
    }
    if diags.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
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
