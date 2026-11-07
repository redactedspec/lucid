//! Mutation fuzzing of the front end: the parser, checker, monomorphizer, and
//! ownership pass must never panic, whatever text they get. See docs/compiler.md.
//!
//! `cargo test` runs a short, fixed round. `LUCID_FUZZ=<count>` runs more
//! inputs, and `LUCID_FUZZ_SEED=<number>` starts from another seed. An input
//! that panics is saved to `target/fuzz-crash-<seed>-<n>.lcd`.

use crate::{STACK_BYTES, check, mono, ownership, resolve};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

/// Characters that tend to reach unusual paths: brackets, operators, quotes,
/// interpolation, escapes, and text that is not ASCII.
const PIECES: &[&str] = &[
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    ",",
    ":",
    ".",
    "?",
    "=",
    "==",
    "<",
    ">",
    "+",
    "-",
    "*",
    "/",
    "%",
    "\"",
    "\\",
    "\\n",
    "\n",
    "\n    ",
    "    ",
    "\t",
    "\r\n",
    "#",
    "'",
    "@",
    "|",
    "&",
    "!",
    "..",
    "->",
    "=>",
    "0",
    "-1",
    "9223372036854775807",
    "9223372036854775808",
    "1.5e999",
    "0x",
    "_",
    "é",
    "\u{0}",
    "\u{feff}",
    "task",
    "type",
    "choice",
    "match",
    "if",
    "else",
    "while",
    "for",
    "in",
    "let",
    "var",
    "return",
    "try",
    "use",
    "public",
    "test",
    "expect",
    "unsafe",
    "extern",
    "change",
    "take",
    "any",
    "none",
    "self",
    "is",
    "trait",
    "list",
    "map",
    "set",
    "text",
    "int",
    "float",
    "bool",
    "empty",
];

/// xorshift64*, enough to pick mutations. Not for anything else.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn corpus(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            corpus(&path, out);
        } else if path.extension().is_some_and(|e| e == "lcd")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            out.push(text);
        }
    }
}

/// A byte position in `text` that is a character boundary.
fn boundary(rng: &mut Rng, text: &str) -> usize {
    let mut at = rng.below(text.len() + 1);
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A range in `text` of up to `max` bytes, on character boundaries.
fn range(rng: &mut Rng, text: &str, max: usize) -> (usize, usize) {
    let start = boundary(rng, text);
    let mut end = (start + rng.below(max + 1)).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (start, end)
}

fn mutate(rng: &mut Rng, files: &[String]) -> String {
    let mut text = files[rng.below(files.len())].clone();
    for _ in 0..=rng.below(4) {
        match rng.below(6) {
            0 => {
                let (a, b) = range(rng, &text, 40);
                text.replace_range(a..b, "");
            }
            1 => {
                let (a, b) = range(rng, &text, 40);
                let copy = text[a..b].to_string();
                let at = boundary(rng, &text);
                text.insert_str(at, &copy);
            }
            2 => {
                let other = &files[rng.below(files.len())];
                let (a, b) = range(rng, other, 200);
                let at = boundary(rng, &text);
                text.insert_str(at, &other[a..b]);
            }
            3 => {
                let (a, b) = range(rng, &text, 3);
                text.replace_range(a..b, PIECES[rng.below(PIECES.len())]);
            }
            4 => {
                let at = boundary(rng, &text);
                text.truncate(at);
            }
            _ => {
                let at = boundary(rng, &text);
                let piece = PIECES[rng.below(PIECES.len())].repeat(1 + rng.below(300));
                text.insert_str(at, &piece);
            }
        }
    }
    text
}

/// Runs a file through every stage before code generation, as `lucid check`
/// and the start of `lucid run` do.
fn front_end(path: &str) {
    let (source, file, diags) = resolve::load(path, false);
    let Some(file) = file else { return };
    if !diags.is_empty() {
        return;
    }
    let (mut program, errors) = check::check(&file, &source, true);
    if errors.is_empty() && mono::run(&mut program).is_empty() {
        ownership::run(&mut program, true);
    }
}

#[test]
fn front_end_never_panics() {
    let env = |name| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok());
    let count = env("LUCID_FUZZ").unwrap_or(300);
    let seed = env("LUCID_FUZZ_SEED").unwrap_or(1);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for dir in ["tests", "examples", "std", "tools"] {
        corpus(&root.join(dir), &mut files);
    }
    assert!(!files.is_empty(), "no corpus");
    // A fresh directory with no `lucid.toml` above it in the usual temp dir.
    let dir = std::env::temp_dir().join(format!("lucid-fuzz-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("fuzz.lcd");
    let crashes = root.join("target");
    let run = move || {
        let mut rng = Rng(seed.max(1));
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut failed = Vec::new();
        for n in 0..count {
            let text = mutate(&mut rng, &files);
            std::fs::write(&input, &text).unwrap();
            let path = input.display().to_string();
            if catch_unwind(AssertUnwindSafe(|| front_end(&path))).is_err() {
                let saved = crashes.join(format!("fuzz-crash-{seed}-{n}.lcd"));
                std::fs::write(&saved, &text).unwrap();
                failed.push(saved.display().to_string());
            }
        }
        std::panic::set_hook(hook);
        failed
    };
    let thread = std::thread::Builder::new().stack_size(STACK_BYTES).spawn(run).unwrap();
    let failed = thread.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failed.is_empty(),
        "the front end panicked on {} inputs:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
