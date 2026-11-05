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
}

impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        let (message, notes) = (message.into(), Vec::new());
        Self { code, message, span, notes }
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// Maps byte offsets to 1-based lines and columns. Columns count characters.
pub struct Lines<'a> {
    src: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(src: &'a str) -> Self {
        let newlines = src.match_indices('\n').map(|(i, _)| i + 1);
        Self { src, starts: std::iter::once(0).chain(newlines).collect() }
    }

    pub fn position(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|&start| start <= offset);
        let start = self.starts[line - 1];
        let prefix = &self.src[start..offset];
        let bom = usize::from(line == 1 && prefix.starts_with('\u{feff}'));
        (line, prefix.chars().count() + 1 - bom)
    }
}

/// Merges parser diagnostics into `diags`, except on lines that already have
/// one. The lexer's recovery often causes a second, misleading parse error.
pub fn add_unless_same_line(src: &str, diags: &mut Vec<Diagnostic>, more: Vec<Diagnostic>) {
    let lines = Lines::new(src);
    let line = |d: &Diagnostic| lines.position(d.span.start).0;
    let taken: std::collections::HashSet<usize> = diags.iter().map(line).collect();
    diags.extend(more.into_iter().filter(|d| !taken.contains(&line(d))));
}

/// Diagnostics must print in a stable order: by position, then as found.
fn sorted(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    let mut sorted: Vec<_> = diags.iter().collect();
    sorted.sort_by_key(|d| d.span.start);
    sorted
}

pub fn render_text(file: &str, src: &str, diags: &[Diagnostic]) -> String {
    let lines = Lines::new(src);
    let mut out = String::new();
    for d in sorted(diags) {
        let (line, column) = lines.position(d.span.start);
        out += &format!("{file}:{line}:{column}: error {}: {}\n", d.code, d.message);
        for note in &d.notes {
            out += &format!("  note: {note}\n");
        }
    }
    out
}

pub fn render_json(file: &str, src: &str, diags: &[Diagnostic]) -> String {
    let lines = Lines::new(src);
    let items: Vec<String> = sorted(diags)
        .iter()
        .map(|d| {
            let (line, column) = lines.position(d.span.start);
            let (end_line, end_column) = lines.position(d.span.end);
            let notes: Vec<String> = d.notes.iter().map(|n| json_string(n)).collect();
            format!(
                "{{\"code\":{},\"severity\":\"error\",\"message\":{},\"file\":{},\"line\":{line},\
                 \"column\":{column},\"end_line\":{end_line},\"end_column\":{end_column},\
                 \"notes\":[{}],\"fixes\":[]}}",
                json_string(d.code),
                json_string(&d.message),
                json_string(file),
                notes.join(","),
            )
        })
        .collect();
    format!("[{}]\n", items.join(","))
}

fn json_string(s: &str) -> String {
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
    fn positions_count_characters_and_lines() {
        let lines = Lines::new("ab\né x\n");
        assert_eq!(lines.position(0), (1, 1));
        assert_eq!(lines.position(3), (2, 1));
        assert_eq!(lines.position(6), (2, 3));
        assert_eq!(lines.position(8), (3, 1));
        assert_eq!(Lines::new("\u{feff}ab").position(4), (1, 2));
    }

    #[test]
    fn json_has_the_documented_shape_and_escapes_text() {
        let span = Span { start: 0, end: 2 };
        let diags = [Diagnostic::new("T0", "bad \"x\"", span).note("a\nb")];
        assert_eq!(
            render_json("a\\b.lcd", "ab", &diags),
            "[{\"code\":\"T0\",\"severity\":\"error\",\"message\":\"bad \\\"x\\\"\",\
             \"file\":\"a\\\\b.lcd\",\"line\":1,\"column\":1,\"end_line\":1,\"end_column\":3,\
             \"notes\":[\"a\\nb\"],\"fixes\":[]}]\n"
        );
    }

    #[test]
    fn output_is_sorted_by_position() {
        let at = |start| Span { start, end: start };
        let diags = [Diagnostic::new("E2", "b", at(1)), Diagnostic::new("E1", "a", at(0))];
        assert_eq!(render_text("f", "ab", &diags), "f:1:1: error E1: a\nf:1:2: error E2: b\n");
    }
}
