//! Diagnostics, in the shape defined in docs/agents.md.

/// Byte range in the source. `end` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub notes: Vec<String>,
    /// Changes that remove the error with near certainty. `lucid fix` applies them.
    pub fixes: Vec<Fix>,
}

/// Edits to make together: each replaces a span with text.
pub struct Fix {
    pub description: String,
    pub edits: Vec<(Span, String)>,
}

impl Span {
    /// The same span, in a file that starts at `base`.
    pub fn shifted(self, base: usize) -> Self {
        Self { start: self.start + base, end: self.end + base }
    }
}

impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        let (message, notes, fixes) = (message.into(), Vec::new(), Vec::new());
        Self { code, message, span, notes, fixes }
    }

    /// Adds a fix of one edit.
    pub fn fix(
        mut self,
        description: impl Into<String>,
        span: Span,
        text: impl Into<String>,
    ) -> Self {
        let description = description.into();
        self.fixes.push(Fix { description, edits: vec![(span, text.into())] });
        self
    }

    /// The same diagnostic, fixes too, in a file that starts at `base`.
    pub fn shifted(mut self, base: usize) -> Self {
        self.span = self.span.shifted(base);
        for edit in self.fixes.iter_mut().flat_map(|fix| &mut fix.edits) {
            edit.0 = edit.0.shifted(base);
        }
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// One file in a `Source`.
pub struct SourceFile {
    pub path: &'static str,
    pub start: usize,
}

/// Every file of a program, end to end, so that a span alone gives its file.
/// The entry file comes first, after the manifest of a package.
#[derive(Default)]
pub struct Source {
    pub text: String,
    pub files: Vec<SourceFile>,
    /// The start of each file whose package lacks the `unsafe` permission,
    /// with the path of the package's `lucid.toml`.
    pub no_unsafe: Vec<(usize, String)>,
    /// The permissions of the root package, one for each of
    /// `manifest::PERMISSIONS`. `None` for a program with no `lucid.toml`.
    pub grants: Option<Vec<crate::manifest::Grant>>,
}

impl Source {
    /// Adds a file, and returns where it starts. A line break follows it, so
    /// the next file starts on a line of its own, and the offset of this file's
    /// end still falls in this file.
    pub fn add(&mut self, path: &'static str, text: &str) -> usize {
        let start = self.text.len();
        self.files.push(SourceFile { path, start });
        self.text += text;
        self.text.push('\n');
        start
    }
}

/// The position in `files` of the file that holds a byte offset.
pub fn file_index(files: &[SourceFile], offset: usize) -> usize {
    files.partition_point(|file| file.start <= offset).saturating_sub(1)
}

/// Maps byte offsets to files and 1-based lines and columns. Columns count
/// characters.
pub struct Lines<'a> {
    source: &'a Source,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(source: &'a Source) -> Self {
        let newlines = source.text.match_indices('\n').map(|(i, _)| i + 1);
        Self { source, starts: std::iter::once(0).chain(newlines).collect() }
    }

    /// The file, line, and column of a byte offset.
    pub fn place(&self, offset: usize) -> (&'static str, usize, usize) {
        let SourceFile { path, start: file } =
            self.source.files[file_index(&self.source.files, offset)];
        let line = self.starts.partition_point(|&start| start <= offset);
        let start = self.starts[line - 1];
        let line = line - self.starts.partition_point(|&start| start < file);
        let prefix = &self.source.text[start..offset];
        let bom = usize::from(line == 1 && prefix.starts_with('\u{feff}'));
        (path, line, prefix.chars().count() + 1 - bom)
    }
}

/// Text with each module name of a package as its user writes it: `@a.lib.b` is
/// `a.b`, and `@a.c.b` is `a.c.b`. `resolve.rs` adds the `@`, and no written name
/// has one.
pub fn plain(text: &str) -> String {
    let mut parts = text.split('@');
    let mut out = parts.next().unwrap_or("").to_string();
    for part in parts {
        let length = part.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(part.len());
        let (package, rest) = part.split_at(length);
        if package.is_empty() || !rest.starts_with('.') {
            out.push('@');
            out += part;
            continue;
        }
        out += package;
        out += match rest.strip_prefix(".lib") {
            Some(tail) if tail.starts_with('.') => tail,
            _ => rest,
        };
    }
    out
}

/// Merges parser diagnostics into `diags`, except on lines that already have
/// one. The lexer's recovery often causes a second, misleading parse error.
pub fn add_unless_same_line(source: &Source, diags: &mut Vec<Diagnostic>, more: Vec<Diagnostic>) {
    let lines = Lines::new(source);
    let line = |d: &Diagnostic| lines.place(d.span.start).1;
    let taken: std::collections::HashSet<usize> = diags.iter().map(line).collect();
    diags.extend(more.into_iter().filter(|d| !taken.contains(&line(d))));
}

/// Diagnostics must print in a stable order: by position, then as found.
fn sorted(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    let mut sorted: Vec<_> = diags.iter().collect();
    sorted.sort_by_key(|d| d.span.start);
    sorted
}

pub fn render_text(source: &Source, diags: &[Diagnostic]) -> String {
    let lines = Lines::new(source);
    let mut out = String::new();
    for d in sorted(diags) {
        let (file, line, column) = lines.place(d.span.start);
        out += &format!("{file}:{line}:{column}: error {}: {}\n", d.code, plain(&d.message));
        for note in &d.notes {
            out += &format!("  note: {}\n", plain(note));
        }
        for fix in &d.fixes {
            out += &format!("  fix: {}\n", plain(&fix.description));
        }
    }
    out
}

pub fn render_json(source: &Source, diags: &[Diagnostic]) -> String {
    let lines = Lines::new(source);
    let items: Vec<String> = sorted(diags)
        .iter()
        .map(|d| {
            let (file, line, column) = lines.place(d.span.start);
            let (_, end_line, end_column) = lines.place(d.span.end);
            let notes: Vec<String> = d.notes.iter().map(|n| json_string(&plain(n))).collect();
            let fixes: Vec<String> = d.fixes.iter().map(|fix| fix_json(&lines, fix)).collect();
            format!(
                "{{\"code\":{},\"severity\":\"error\",\"message\":{},\"file\":{},\"line\":{line},\
                 \"column\":{column},\"end_line\":{end_line},\"end_column\":{end_column},\
                 \"notes\":[{}],\"fixes\":[{}]}}",
                json_string(d.code),
                json_string(&plain(&d.message)),
                json_string(file),
                notes.join(","),
                fixes.join(","),
            )
        })
        .collect();
    format!("[{}]\n", items.join(","))
}

fn fix_json(lines: &Lines, fix: &Fix) -> String {
    let edits: Vec<String> = fix
        .edits
        .iter()
        .map(|(span, text)| {
            let (_, line, column) = lines.place(span.start);
            let (_, end_line, end_column) = lines.place(span.end);
            format!(
                "{{\"line\":{line},\"column\":{column},\"end_line\":{end_line},\
                 \"end_column\":{end_column},\"text\":{}}}",
                json_string(text)
            )
        })
        .collect();
    let description = json_string(&plain(&fix.description));
    format!("{{\"description\":{description},\"edits\":[{}]}}", edits.join(","))
}

/// How many single-character changes turn `a` into `b`: insertions,
/// deletions, replacements, and swaps of neighbors.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    // Rows for the last two prefixes of `a`, and this one.
    let mut before: Vec<usize> = Vec::new();
    let mut last: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let replace = last[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            row[j] = replace.min(last[j] + 1).min(row[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(before[j - 2] + 1);
            }
        }
        before = std::mem::replace(&mut last, row);
    }
    last[b.len()]
}

pub fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if c < ' ' => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_read_as_written() {
        assert_eq!(plain("found `@q.lib.User`, not `@q.a.b.T`"), "found `q.User`, not `q.a.b.T`");
        assert_eq!(plain("mail a@ or @ or @x, @.y"), "mail a@ or @ or @x, @.y");
    }

    fn one(text: &str) -> Source {
        let mut source = Source::default();
        source.add("f", text);
        source
    }

    #[test]
    fn positions_count_characters_and_lines() {
        let source = one("ab\né x\n");
        let lines = Lines::new(&source);
        assert_eq!(lines.place(0), ("f", 1, 1));
        assert_eq!(lines.place(3), ("f", 2, 1));
        assert_eq!(lines.place(6), ("f", 2, 3));
        assert_eq!(lines.place(8), ("f", 3, 1));
        assert_eq!(Lines::new(&one("\u{feff}ab")).place(4), ("f", 1, 2));
    }

    #[test]
    fn json_has_the_documented_shape_and_escapes_text() {
        let span = Span { start: 0, end: 2 };
        let diags = [Diagnostic::new("T0", "bad \"x\"", span).note("a\nb")];
        let mut source = Source::default();
        source.add("a\\b.lcd", "ab");
        assert_eq!(
            render_json(&source, &diags),
            "[{\"code\":\"T0\",\"severity\":\"error\",\"message\":\"bad \\\"x\\\"\",\
             \"file\":\"a\\\\b.lcd\",\"line\":1,\"column\":1,\"end_line\":1,\"end_column\":3,\
             \"notes\":[\"a\\nb\"],\"fixes\":[]}]\n"
        );
    }

    #[test]
    fn edit_distance_counts_each_change() {
        assert_eq!(edit_distance("totl", "total"), 1);
        assert_eq!(edit_distance("ttoal", "total"), 1);
        assert_eq!(edit_distance("cat", "dog"), 3);
        assert_eq!(edit_distance("", "ab"), 2);
    }

    #[test]
    fn output_is_sorted_by_position() {
        let at = |start| Span { start, end: start };
        let diags = [Diagnostic::new("E2", "b", at(1)), Diagnostic::new("E1", "a", at(0))];
        assert_eq!(render_text(&one("ab"), &diags), "f:1:1: error E1: a\nf:1:2: error E2: b\n");
    }
}
