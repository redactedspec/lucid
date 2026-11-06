//! Turns source text into tokens. See "Layout" and "Tokens" in docs/grammar.md.

use crate::diag::{Diagnostic, Span};

/// Deepest allowed nesting of text interpolation, blocks, and expressions.
pub const MAX_DEPTH: usize = 128;

pub const KEYWORDS: &[&str] = &[
    "and", "any", "as", "break", "change", "choice", "continue", "else", "empty", "expect",
    "extern", "false", "for", "if", "in", "is", "let", "match", "none", "not", "or", "print",
    "public", "return", "self", "shared", "spawn", "take", "task", "test", "then", "trait", "true",
    "try", "type", "unsafe", "use", "var", "weak", "while",
];

/// Longest first, so that the first match is the longest match.
const SYMBOLS: &[&str] = &[
    "...", "..=", "..", "./", ".", "=>", "==", "=", "!=", "<=", "<", ">=", ">", "->", "-=", "-",
    "+=", "+", "*=", "*", "//", "/=", "/", "%", "(", ")", "[", "]", "{", "}", ",", ":", "?",
];

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Name(String),
    Int(String),
    Float(String),
    Text(Vec<TextPart>),
    /// A keyword or an operator.
    Sym(&'static str),
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextPart {
    Literal(String),
    /// The tokens of one `{expression}`, ended by a `}` token.
    Expr(Vec<Token>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl TokenKind {
    pub fn describe(&self) -> String {
        match self {
            Self::Name(name) => format!("name `{name}`"),
            Self::Int(_) | Self::Float(_) => "number".into(),
            Self::Text(_) => "text".into(),
            Self::Sym(sym) => format!("`{sym}`"),
            Self::Newline => "end of line".into(),
            Self::Indent => "indentation".into(),
            Self::Dedent => "end of block".into(),
            Self::Eof => "end of file".into(),
        }
    }
}

pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lexer = Lexer {
        src,
        pos: if src.starts_with('\u{feff}') { 3 } else { 0 },
        indents: vec![0],
        depth: 0,
        text_depth: 0,
        gave_up: false,
        line_start: true,
        line_has_tokens: false,
        diags: Vec::new(),
    };
    let mut out = Vec::new();
    lexer.scan(&mut out, false);
    lexer.finish(&mut out);
    (out, lexer.diags)
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    /// Widths of the open blocks. The bottom entry is always 0.
    indents: Vec<usize>,
    /// Open brackets. Layout is off while this is above 0.
    depth: usize,
    text_depth: usize,
    /// Set when nesting exceeds MAX_DEPTH and the rest of the file is dropped.
    gave_up: bool,
    line_start: bool,
    line_has_tokens: bool,
    diags: Vec<Diagnostic>,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.src[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) {
        while let Some(c) = self.peek().filter(|&c| keep(c)) {
            self.pos += c.len_utf8();
        }
    }

    fn span(&self, start: usize) -> Span {
        Span { start, end: self.pos }
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, start: usize) {
        self.diags.push(Diagnostic::new(code, message, self.span(start)));
    }

    fn push(&mut self, out: &mut Vec<Token>, kind: TokenKind, start: usize) {
        out.push(Token { kind, span: self.span(start) });
        self.line_has_tokens = true;
    }

    /// Scans tokens to end of file, or to the `}` that closes an interpolation.
    /// Returns true when it stopped at that `}`.
    fn scan(&mut self, out: &mut Vec<Token>, interpolation: bool) -> bool {
        let floor = self.depth;
        loop {
            if self.line_start {
                self.line_start = false;
                if self.depth == 0 {
                    self.layout(out);
                }
            }
            let Some(c) = self.peek() else { return false };
            let start = self.pos;
            match c {
                ' ' | '\t' => self.pos += 1,
                '\r' if self.rest().starts_with("\r\n") => self.pos += 1,
                '\n' => {
                    self.pos += 1;
                    if self.depth == 0 {
                        if self.line_has_tokens {
                            out.push(Token { kind: TokenKind::Newline, span: self.span(start) });
                            self.line_has_tokens = false;
                        }
                        self.line_start = true;
                    }
                }
                '#' => self.take_while(|c| c != '\n'),
                '"' => self.text(out),
                '0'..='9' => self.number(out),
                c if c == '_' || c.is_ascii_alphabetic() => self.name(out),
                _ => {
                    let rest = self.rest();
                    let Some(sym) = SYMBOLS.iter().copied().find(|s| rest.starts_with(s)) else {
                        self.pos += c.len_utf8();
                        self.error(
                            "E0103",
                            format!("unexpected character `{}`", c.escape_debug()),
                            start,
                        );
                        continue;
                    };
                    self.pos += sym.len();
                    self.push(out, TokenKind::Sym(sym), start);
                    match sym {
                        "(" | "[" | "{" => self.depth += 1,
                        "}" if interpolation && self.depth == floor => return true,
                        ")" | "]" | "}" if self.depth > floor => self.depth -= 1,
                        _ => {}
                    }
                }
            }
        }
    }

    /// Handles the start of a line, and leaves `pos` after the indentation.
    fn layout(&mut self, out: &mut Vec<Token>) {
        let start = self.pos;
        let (mut width, mut tab) = (0, false);
        loop {
            match self.peek() {
                Some(' ') => width += 1,
                Some('\t') => (width, tab) = (width + 4, true),
                _ => break,
            }
            self.pos += 1;
        }
        let rest = self.rest();
        if rest.is_empty() || rest.starts_with(['\n', '#']) || rest.starts_with("\r\n") {
            return;
        }
        if tab {
            self.error("E0101", "tab in indentation; indent with 4 spaces", start);
        }
        let top = self.indents.last().copied().unwrap_or(0);
        if width > top {
            self.indents.push(width);
            out.push(Token { kind: TokenKind::Indent, span: self.span(start) });
        }
        while self.indents.last().is_some_and(|&top| top > width) {
            self.indents.pop();
            out.push(Token { kind: TokenKind::Dedent, span: self.span(start) });
        }
        let matched = width == top + 4 || (width <= top && self.indents.last() == Some(&width));
        if !matched && !tab {
            let message = format!("indentation of {width} spaces; each block is 4 spaces deeper");
            self.error("E0102", message, start);
        }
    }

    fn finish(&mut self, out: &mut Vec<Token>) {
        let start = self.pos;
        if self.line_has_tokens {
            out.push(Token { kind: TokenKind::Newline, span: self.span(start) });
        }
        for _ in 1..self.indents.len() {
            out.push(Token { kind: TokenKind::Dedent, span: self.span(start) });
        }
        out.push(Token { kind: TokenKind::Eof, span: self.span(start) });
    }

    fn name(&mut self, out: &mut Vec<Token>) {
        let start = self.pos;
        self.take_while(|c| c == '_' || c.is_ascii_alphanumeric());
        let word = &self.src[start..self.pos];
        let kind = match KEYWORDS.iter().find(|k| **k == word) {
            Some(keyword) => TokenKind::Sym(keyword),
            None => TokenKind::Name(word.into()),
        };
        self.push(out, kind, start);
    }

    fn number(&mut self, out: &mut Vec<Token>) {
        let start = self.pos;
        let digits = |c: char| c == '_' || c.is_ascii_digit();
        let mut float = false;
        let mut valid = true;
        let radix = ["0x", "0b"].into_iter().find(|p| self.rest().starts_with(p));
        if let Some(prefix) = radix {
            let is_digit = move |c: char| {
                if prefix == "0x" { c.is_ascii_hexdigit() } else { matches!(c, '0' | '1') }
            };
            self.pos += 2;
            valid = self.peek().is_some_and(is_digit);
            self.take_while(|c| c == '_' || is_digit(c));
        } else {
            self.take_while(digits);
            let mut after = self.rest().chars().skip(1);
            if self.peek() == Some('.') && after.next().is_some_and(|c| c.is_ascii_digit()) {
                float = true;
                self.pos += 1;
                self.take_while(digits);
                self.exponent();
            }
        }
        if self.peek().is_some_and(|c| c == '_' || c.is_ascii_alphanumeric()) {
            valid = false;
            self.take_while(|c| c == '_' || c.is_ascii_alphanumeric());
        }
        let lexeme = self.src[start..self.pos].to_string();
        if !valid || lexeme.ends_with('_') {
            self.error("E0107", format!("malformed number `{lexeme}`"), start);
        }
        let kind = if float { TokenKind::Float(lexeme) } else { TokenKind::Int(lexeme) };
        self.push(out, kind, start);
    }

    /// Consumes an exponent. Leaves `pos` alone if there is no complete one.
    fn exponent(&mut self) {
        let Some(after_e) = self.rest().strip_prefix(['e', 'E']) else { return };
        let sign = usize::from(after_e.starts_with(['+', '-']));
        if after_e[sign..].starts_with(|c: char| c.is_ascii_digit()) {
            self.pos += 1 + sign;
            self.take_while(|c| c == '_' || c.is_ascii_digit());
        }
    }

    fn text(&mut self, out: &mut Vec<Token>) {
        let start = self.pos;
        let triple = self.rest().starts_with("\"\"\"");
        self.pos += if triple { 3 } else { 1 };
        if triple {
            let line = self.rest().split('\n').next().unwrap_or("");
            if line.trim().is_empty() {
                self.pos = (self.pos + line.len() + 1).min(self.src.len());
            } else {
                self.error("E0108", "`\"\"\"` must end its line", start);
            }
        }
        // Literal text stays raw until dedenting is done. NUL marks each interpolation.
        let mut raw = String::new();
        let mut exprs = Vec::new();
        loop {
            let rest = self.rest();
            let at = self.pos;
            let Some(c) = rest.chars().next() else {
                if !self.gave_up {
                    self.error("E0104", "text is not closed", start);
                }
                break;
            };
            match c {
                '"' if !triple => {
                    self.pos += 1;
                    break;
                }
                '"' if rest.starts_with("\"\"\"") => {
                    self.pos += 3;
                    break;
                }
                '\n' if !triple => {
                    self.error("E0104", "text is not closed before end of line", start);
                    break;
                }
                '\r' if rest.starts_with("\r\n") => self.pos += 1,
                '\\' => match rest[1..].chars().next() {
                    Some(e @ ('n' | 't' | 'r' | '\\' | '"')) => {
                        raw.push('\\');
                        raw.push(e);
                        self.pos += 2;
                    }
                    _ => {
                        self.pos += 1;
                        self.error("E0105", "unknown escape; use \\n \\t \\r \\\\ or \\\"", at);
                    }
                },
                '{' if rest.starts_with("{{") => {
                    raw += "{{";
                    self.pos += 2;
                }
                '}' if rest.starts_with("}}") => {
                    raw += "}}";
                    self.pos += 2;
                }
                '}' => {
                    self.pos += 1;
                    self.error("E0106", "`}` in text; write `}}` for a literal brace", at);
                }
                '{' => {
                    raw.push('\0');
                    exprs.push(self.interpolation());
                }
                '\0' => {
                    self.pos += 1;
                    self.error("E0103", "unexpected character NUL", at);
                }
                c => {
                    raw.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        let raw = if triple { dedent(&raw) } else { raw };
        let mut exprs = exprs.into_iter();
        let mut parts = Vec::new();
        for (i, literal) in raw.split('\0').enumerate() {
            if i > 0 {
                parts.extend(exprs.next().map(TextPart::Expr));
            }
            if !literal.is_empty() {
                parts.push(TextPart::Literal(decode(literal)));
            }
        }
        self.push(out, TokenKind::Text(parts), start);
    }

    fn interpolation(&mut self) -> Vec<Token> {
        let start = self.pos;
        self.pos += 1;
        if self.text_depth == MAX_DEPTH {
            self.error("E0205", "nesting is too deep", start);
            self.pos = self.src.len();
            self.gave_up = true;
        }
        let mut tokens = Vec::new();
        self.text_depth += 1;
        self.depth += 1;
        let closed = self.scan(&mut tokens, true);
        self.depth -= 1;
        self.text_depth -= 1;
        if !closed {
            tokens.push(Token { kind: TokenKind::Sym("}"), span: self.span(start) });
        }
        tokens
    }
}

/// Removes the common indentation of a `"""` text. The line of the closing
/// quotes counts toward it, and that line and its newline are dropped if blank.
fn dedent(raw: &str) -> String {
    let lines: Vec<&str> = raw.split('\n').collect();
    let last = lines.len() - 1;
    let closing_blank = lines[last].trim_matches(' ').is_empty();
    let common = lines
        .iter()
        .enumerate()
        .filter(|(i, line)| *i == last || !line.trim_matches(' ').is_empty())
        .map(|(_, line)| line.len() - line.trim_start_matches(' ').len())
        .min()
        .unwrap_or(0);
    let kept = if closing_blank { &lines[..last] } else { &lines[..] };
    let kept: Vec<&str> = kept.iter().map(|line| line.get(common..).unwrap_or("")).collect();
    kept.join("\n")
}

/// Replaces escapes and doubled braces. The scanner has already validated them.
fn decode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => {}
            },
            '{' | '}' => {
                chars.next();
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(tokens: &[Token]) -> String {
        let words: Vec<String> = tokens
            .iter()
            .map(|t| match &t.kind {
                TokenKind::Name(s) | TokenKind::Int(s) | TokenKind::Float(s) => s.clone(),
                TokenKind::Sym(s) => s.to_string(),
                TokenKind::Newline => "NL".into(),
                TokenKind::Indent => "IN".into(),
                TokenKind::Dedent => "DE".into(),
                TokenKind::Eof => "EOF".into(),
                TokenKind::Text(parts) => {
                    let parts: Vec<String> = parts
                        .iter()
                        .map(|p| match p {
                            TextPart::Literal(s) => format!("{s:?}"),
                            TextPart::Expr(tokens) => format!("<{}>", show(tokens)),
                        })
                        .collect();
                    format!("text[{}]", parts.join(" "))
                }
            })
            .collect();
        words.join(" ")
    }

    fn tokens(src: &str) -> String {
        let (tokens, diags) = lex(src);
        let codes: Vec<&str> = diags.iter().map(|d| d.code).collect();
        assert!(codes.is_empty(), "{src:?} gave {codes:?}");
        show(&tokens)
    }

    fn codes(src: &str) -> Vec<&'static str> {
        lex(src).1.iter().map(|d| d.code).collect()
    }

    #[test]
    fn layout_tokens_follow_the_grammar() {
        assert_eq!(tokens(""), "EOF");
        assert_eq!(tokens("a"), "a NL EOF");
        assert_eq!(tokens("a\n    b\n        c\nd\n"), "a NL IN b NL IN c NL DE DE d NL EOF");
        assert_eq!(tokens("a\n    b"), "a NL IN b NL DE EOF");
    }

    #[test]
    fn blank_and_comment_lines_produce_nothing() {
        assert_eq!(tokens("# x\n\na # y\n\n   \n  # z\n## doc\nb\n"), "a NL b NL EOF");
        assert_eq!(tokens("a\n    b\n# x\n  \n    c\n"), "a NL IN b NL c NL DE EOF");
    }

    #[test]
    fn brackets_turn_layout_off() {
        assert_eq!(tokens("a(\n  b,\n\n c\n)\nd"), "a ( b , c ) NL d NL EOF");
        assert_eq!(tokens("[\n{\n}\n]\n"), "[ { } ] NL EOF");
    }

    #[test]
    fn crlf_and_bom_are_accepted() {
        assert_eq!(tokens("\u{feff}a\r\n    b\r\n"), "a NL IN b NL DE EOF");
        assert_eq!(tokens("\"\"\"\r\n  a\r\n  b\r\n  \"\"\"\r\n"), "text[\"a\\nb\"] NL EOF");
    }

    #[test]
    fn numbers_and_ranges() {
        assert_eq!(tokens("0..10"), "0 .. 10 NL EOF");
        assert_eq!(tokens("0..=1_0"), "0 ..= 1_0 NL EOF");
        assert_eq!(
            tokens("1.5 1.5e3 1.5E-3 0xFF_ff 0b1_0"),
            "1.5 1.5e3 1.5E-3 0xFF_ff 0b1_0 NL EOF"
        );
        assert_eq!(tokens("1.foo"), "1 . foo NL EOF");
        assert_eq!(codes("0x"), ["E0107"]);
        assert_eq!(codes("0b2"), ["E0107"]);
        assert_eq!(codes("12ab"), ["E0107"]);
        assert_eq!(codes("1e5"), ["E0107"]);
        assert_eq!(codes("1.5e"), ["E0107"]);
    }

    #[test]
    fn symbols_use_the_longest_match() {
        assert_eq!(
            tokens("... ..= .. ./ . => == = -> -= //"),
            "... ..= .. ./ . => == = -> -= // NL EOF"
        );
    }

    #[test]
    fn keywords_are_not_names() {
        let (tokens, _) = lex("task tasks");
        assert_eq!(tokens[0].kind, TokenKind::Sym("task"));
        assert_eq!(tokens[1].kind, TokenKind::Name("tasks".into()));
    }

    #[test]
    fn text_literals() {
        assert_eq!(tokens(r#""""#), "text[] NL EOF");
        assert_eq!(tokens(r#""a{{b}}c""#), "text[\"a{b}c\"] NL EOF");
        assert_eq!(tokens(r#""\n\t\r\\\"""#), r#"text["\n\t\r\\\""] NL EOF"#);
        assert_eq!(tokens("\"é\""), "text[\"é\"] NL EOF");
    }

    #[test]
    fn interpolation_nests() {
        assert_eq!(tokens(r#""a{b + 1}c""#), "text[\"a\" <b + 1 }> \"c\"] NL EOF");
        assert_eq!(tokens(r#""{f("x{y}")}""#), "text[<f ( text[\"x\" <y }>] ) }>] NL EOF");
        assert_eq!(tokens(r#""{ {1: 2} }""#), "text[<{ 1 : 2 } }>] NL EOF");
        assert_eq!(tokens(r#""{{{x}}}""#), "text[\"{\" <x }> \"}\"] NL EOF");
    }

    #[test]
    fn multi_line_text_removes_common_indentation() {
        assert_eq!(
            tokens("\"\"\"\n    a\n      b\n\n    c\n    \"\"\""),
            "text[\"a\\n  b\\n\\nc\"] NL EOF"
        );
        assert_eq!(tokens("\"\"\"\n    a\n  \"\"\""), "text[\"  a\"] NL EOF");
        assert_eq!(tokens("\"\"\"\n  a\n  b\"\"\""), "text[\"a\\nb\"] NL EOF");
        assert_eq!(
            tokens("\"\"\"\n  a \"b\" {x}\n  \"\"\""),
            "text[\"a \\\"b\\\" \" <x }>] NL EOF"
        );
        assert_eq!(
            tokens("x\n    \"\"\"\n    a\n    \"\"\"\ny"),
            "x NL IN text[\"a\"] NL DE y NL EOF"
        );
    }

    #[test]
    fn errors_have_codes() {
        assert_eq!(codes("a\n\tb"), ["E0101"]);
        assert_eq!(codes("a\n  b"), ["E0102"]);
        assert_eq!(codes("a\n        b"), ["E0102"]);
        assert_eq!(codes("a\n    b\n  c"), ["E0102"]);
        assert_eq!(codes("!"), ["E0103"]);
        assert_eq!(codes("é"), ["E0103"]);
        assert_eq!(lex("\u{1b}").1[0].message, "unexpected character `\\u{1b}`");
        assert_eq!(codes("\"a\0\""), ["E0103"]);
        assert_eq!(codes("\"a"), ["E0104"]);
        assert_eq!(codes("\"a\nb"), ["E0104"]);
        assert_eq!(codes("\"\"\"\na"), ["E0104"]);
        assert_eq!(codes("\"{a"), ["E0104"]);
        assert_eq!(codes(r#""\q""#), ["E0105"]);
        assert_eq!(codes(r#""}""#), ["E0106"]);
        assert_eq!(codes("\"\"\"a\n\"\"\""), ["E0108"]);
        assert_eq!(codes(&"\"{".repeat(MAX_DEPTH + 1)), ["E0205"]);
    }

    #[test]
    fn bad_indentation_keeps_blocks_balanced() {
        let (tokens, _) = lex("a\n  b\n      c\n");
        let count = |kind: TokenKind| tokens.iter().filter(|t| t.kind == kind).count();
        assert_eq!(count(TokenKind::Indent), count(TokenKind::Dedent));
    }

    #[test]
    fn stray_closing_brackets_do_not_break_layout() {
        assert_eq!(tokens(")\na\n    b"), ") NL a NL IN b NL DE EOF");
    }
}
