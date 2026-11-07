//! `lucid context`: the public API of a module, as signatures and doc comments.
//! It reads tokens, not the tree, so a file with errors still shows what it can.

use crate::diag::json_string;
use crate::lexer::{self, Token, TokenKind};

/// One public declaration.
pub struct Item {
    pub line: usize,
    pub kind: &'static str,
    pub name: String,
    /// The header, with the fields, variants, or tasks of a type, choice, or
    /// trait, and no bodies. Without `public`, since every item has it.
    pub signature: String,
    /// The `##` lines just above it, without their `##`.
    pub doc: String,
}

/// Every public declaration of a file, in order.
pub fn items(text: &str) -> Vec<Item> {
    let (tokens, _) = lexer::lex(text);
    let lines = lines(&tokens);
    let mut items = Vec::new();
    let mut depth = 0;
    for (place, line) in lines.iter().enumerate() {
        depth += line.indent;
        let public = matches!(&line.tokens[0].kind, TokenKind::Sym("public"));
        if depth != 0 || !public {
            continue;
        }
        let kind = line.tokens.iter().find_map(|token| match &token.kind {
            TokenKind::Sym(kind @ ("task" | "type" | "choice" | "trait")) => Some(*kind),
            _ => None,
        });
        let Some(kind) = kind else { continue };
        let at = line.tokens.iter().position(|token| token.kind == TokenKind::Sym(kind));
        let name = match at.and_then(|at| line.tokens.get(at + 1)).map(|token| &token.kind) {
            Some(TokenKind::Name(name)) => name.clone(),
            _ => continue,
        };
        let mut signature = source(text, &line.tokens[1..]);
        // Members sit one level in: fields, variants, or the tasks of a trait.
        // Deeper lines are bodies.
        if kind != "task" {
            let mut inner = 0;
            for member in &lines[place + 1..] {
                inner += member.indent;
                if inner <= 0 {
                    break;
                }
                if inner == 1 {
                    signature += "\n    ";
                    signature += &source(text, member.tokens);
                }
            }
        }
        let start = line.tokens[0].span.start;
        items.push(Item {
            line: line_of(text, start),
            kind,
            name,
            signature,
            doc: doc(text, start),
        });
    }
    items
}

/// The tokens of one logical line, and the change of depth before it.
struct Line<'a> {
    indent: i32,
    tokens: &'a [Token],
}

fn lines(tokens: &[Token]) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let (mut indent, mut start) = (0, 0);
    for (at, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Indent => indent += 1,
            TokenKind::Dedent => indent -= 1,
            TokenKind::Newline | TokenKind::Eof => {
                // Layout tokens come before the first token of a line.
                let first = tokens[start..at].iter().position(|t| !is_layout(t)).map(|p| start + p);
                if let Some(first) = first {
                    lines.push(Line { indent, tokens: &tokens[first..at] });
                    indent = 0;
                }
                start = at + 1;
            }
            _ => {}
        }
    }
    lines
}

fn is_layout(token: &Token) -> bool {
    matches!(token.kind, TokenKind::Indent | TokenKind::Dedent | TokenKind::Newline)
}

/// The source of a run of tokens on one logical line. A line that brackets
/// spread over several becomes one.
fn source(text: &str, tokens: &[Token]) -> String {
    let (Some(first), Some(last)) = (tokens.first(), tokens.last()) else { return String::new() };
    let raw = &text[first.span.start..last.span.end];
    if !raw.contains('\n') {
        return raw.to_string();
    }
    let joined: Vec<&str> = raw.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    joined.join(" ").replace("( ", "(").replace(", )", ")").replace(" )", ")")
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
}

/// The `##` lines right above the line at `offset`.
fn doc(text: &str, offset: usize) -> String {
    let above = &text[..offset];
    let above = above.rsplit_once('\n').map_or("", |(above, _)| above);
    let mut doc: Vec<&str> = above
        .lines()
        .rev()
        .map(str::trim)
        .take_while(|line| line.starts_with("##"))
        .map(|line| line["##".len()..].strip_prefix(' ').unwrap_or(&line["##".len()..]))
        .collect();
    doc.reverse();
    doc.join("\n")
}

/// The report for modules named by `module`, each with its items.
pub fn render(modules: &[(String, Vec<Item>)], json: bool) -> String {
    if json {
        let items = modules.iter().flat_map(|(module, items)| {
            items.iter().map(move |item| {
                format!(
                    "{{\"module\":{},\"line\":{},\"kind\":\"{}\",\"name\":{},\"signature\":{},\"doc\":{}}}",
                    json_string(module),
                    item.line,
                    item.kind,
                    json_string(&item.name),
                    json_string(&item.signature),
                    json_string(&item.doc),
                )
            })
        });
        return format!("[{}]\n", items.collect::<Vec<_>>().join(","));
    }
    let mut out = String::new();
    for (module, items) in modules.iter().filter(|(_, items)| !items.is_empty()) {
        if !out.is_empty() {
            out.push('\n');
        }
        out += &format!("# {module}\n");
        // Items of one line sit together. Others get a line of space.
        let mut tall = true;
        for item in items {
            let this = !item.doc.is_empty() || item.signature.contains('\n');
            if tall || this {
                out.push('\n');
            }
            tall = this;
            for line in item.doc.lines() {
                out += &format!("## {line}\n");
            }
            out += &item.signature;
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_public_headers_members_and_docs() {
        let text = "## Adds.\n## Twice.\npublic task add(a: int,\n        b: int) -> int\n    return a + b\n\
                    task hidden()\n    return\n\npublic trait Named\n    task name(self) -> text\n    \
                    task greet(self) -> text\n        return \"hi\"\n\n# Not a doc.\npublic type Point\n    \
                    x: int\n    y: int = 0\nprint 1\n";
        let found = items(text);
        let shown: Vec<(&str, &str, &str, &str)> = found
            .iter()
            .map(|item| (item.kind, item.name.as_str(), item.signature.as_str(), item.doc.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("task", "add", "task add(a: int, b: int) -> int", "Adds.\nTwice."),
                (
                    "trait",
                    "Named",
                    "trait Named\n    task name(self) -> text\n    task greet(self) -> text",
                    ""
                ),
                ("type", "Point", "type Point\n    x: int\n    y: int = 0", ""),
            ]
        );
        assert_eq!(found[0].line, 3);
    }
}
