//! Reads `lucid.toml`: the part of TOML 1.0 that a manifest needs, by hand, since
//! the compiler takes no TOML dependency. See decision 121.
//!
//! Tables hold keys with bare names. A value is a string, `true`, `false`, an
//! array, or an inline table. Anything else is E0338, never a guess.

use crate::diag::{Diagnostic, Span};
use crate::lexer::KEYWORDS;

/// Deep enough for any manifest, and it keeps hostile input off the stack.
const MAX_DEPTH: usize = 8;

const PERMISSIONS: &[&str] = &["net", "files", "env", "process", "unsafe"];

pub struct Manifest {
    pub name: String,
    pub dependencies: Vec<Dependency>,
}

#[derive(Clone)]
pub struct Dependency {
    pub name: String,
    /// As written, relative to the manifest.
    pub path: String,
    pub span: Span,
}

enum Value {
    Text(String),
    Bool,
    List(Vec<(Value, Span)>),
    Table(Vec<Entry>),
}

struct Entry {
    key: String,
    span: Span,
    value: Value,
}

struct Table {
    name: String,
    span: Span,
    entries: Vec<Entry>,
}

/// Parses a manifest. `None` comes with at least one diagnostic.
pub fn parse(text: &str) -> (Option<Manifest>, Vec<Diagnostic>) {
    let mut reader = Reader { text, at: usize::from(text.starts_with('\u{feff}')) * 3 };
    match reader.tables() {
        Ok(tables) => read_tables(&tables),
        Err(error) => (None, vec![error]),
    }
}

fn error(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::new("E0338", message, span)
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }

    fn span(&self, start: usize) -> Span {
        Span { start, end: self.at }
    }

    fn here(&self) -> Span {
        Span { start: self.at, end: self.at }
    }

    fn eat(&mut self, c: char) -> bool {
        let found = self.peek() == Some(c);
        if found {
            self.at += c.len_utf8();
        }
        found
    }

    /// Skips spaces and a comment on this line.
    fn blanks(&mut self) {
        while self.eat(' ') || self.eat('\t') {}
        if self.peek() == Some('#') {
            self.at = self.text[self.at..].find('\n').map_or(self.text.len(), |n| self.at + n);
        }
    }

    /// Skips blanks and line breaks.
    fn lines(&mut self) {
        loop {
            self.blanks();
            if !(self.eat('\n') || self.eat('\r')) {
                return;
            }
        }
    }

    fn line_end(&mut self) -> Result<(), Diagnostic> {
        self.blanks();
        self.eat('\r');
        if self.peek().is_none() || self.eat('\n') {
            return Ok(());
        }
        Err(error("expected the end of the line", self.here()))
    }

    fn key(&mut self) -> Result<(String, Span), Diagnostic> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            self.at += 1;
        }
        if self.at == start {
            return Err(error("expected a key", self.here()));
        }
        Ok((self.text[start..self.at].to_string(), self.span(start)))
    }

    fn tables(&mut self) -> Result<Vec<Table>, Diagnostic> {
        let mut tables: Vec<Table> = Vec::new();
        loop {
            self.lines();
            let start = self.at;
            if self.peek().is_none() {
                return Ok(tables);
            }
            if self.eat('[') {
                if self.peek() == Some('[') {
                    return Err(error("`lucid.toml` has no arrays of tables", self.span(start)));
                }
                let (name, span) = self.key()?;
                if !self.eat(']') {
                    let note = "write `name = { path = \"...\" }` under `[dependencies]`";
                    return Err(error("expected `]`", self.here()).note(note));
                }
                self.line_end()?;
                if tables.iter().any(|table| table.name == name) {
                    return Err(error(format!("table `{name}` appears twice"), span));
                }
                tables.push(Table { name, span, entries: Vec::new() });
                continue;
            }
            let entry = self.entry(0)?;
            self.line_end()?;
            let Some(table) = tables.last_mut() else {
                return Err(error("a key must follow a table header", entry.span));
            };
            add(&mut table.entries, entry)?;
        }
    }

    fn entry(&mut self, depth: usize) -> Result<Entry, Diagnostic> {
        let (key, span) = self.key()?;
        self.blanks();
        if !self.eat('=') {
            return Err(error("expected `=`", self.here()));
        }
        self.blanks();
        let (value, _) = self.value(depth)?;
        Ok(Entry { key, span, value })
    }

    fn value(&mut self, depth: usize) -> Result<(Value, Span), Diagnostic> {
        let start = self.at;
        if depth > MAX_DEPTH {
            return Err(error("values are nested too deep", self.here()));
        }
        let value = match self.peek() {
            Some('"') => self.basic_string()?,
            Some('\'') => self.literal_string()?,
            Some('[') => self.list(depth)?,
            Some('{') => self.inline_table(depth)?,
            _ if self.text[self.at..].starts_with("true") => {
                self.at += 4;
                Value::Bool
            }
            _ if self.text[self.at..].starts_with("false") => {
                self.at += 5;
                Value::Bool
            }
            _ => {
                let message = "expected a string, `true`, `false`, `[`, or `{`";
                return Err(error(message, self.here()));
            }
        };
        Ok((value, self.span(start)))
    }

    fn basic_string(&mut self) -> Result<Value, Diagnostic> {
        let start = self.at;
        self.at += 1;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.at += c.len_utf8();
            match c {
                '"' => return Ok(Value::Text(out)),
                '\\' => {
                    let escape = self.peek();
                    out.push(match escape {
                        Some('\\') => '\\',
                        Some('"') => '"',
                        Some('n') => '\n',
                        Some('r') => '\r',
                        Some('t') => '\t',
                        _ => return Err(error("this escape is not supported", self.here())),
                    });
                    self.at += 1;
                }
                '\n' | '\r' => break,
                c if c.is_control() && c != '\t' => {
                    return Err(error("a string cannot hold a control character", self.here()));
                }
                c => out.push(c),
            }
        }
        Err(error("this string has no end on its line", self.span(start)))
    }

    fn literal_string(&mut self) -> Result<Value, Diagnostic> {
        let start = self.at;
        self.at += 1;
        let line = self.text[self.at..].lines().next().unwrap_or("");
        let Some(length) = line.find('\'') else {
            return Err(error("this string has no end on its line", self.here()));
        };
        let out = line[..length].to_string();
        if out.chars().any(|c| c.is_control() && c != '\t') {
            return Err(error("a string cannot hold a control character", self.span(start)));
        }
        self.at += length + 1;
        Ok(Value::Text(out))
    }

    /// An array may span lines, and may end with a comma.
    fn list(&mut self, depth: usize) -> Result<Value, Diagnostic> {
        self.at += 1;
        let mut items = Vec::new();
        loop {
            self.lines();
            if self.eat(']') {
                return Ok(Value::List(items));
            }
            items.push(self.value(depth + 1)?);
            self.lines();
            if !self.eat(',') && self.peek() != Some(']') {
                return Err(error("expected `,` or `]`", self.here()));
            }
        }
    }

    /// An inline table sits on one line, and has no comma after its last pair.
    fn inline_table(&mut self, depth: usize) -> Result<Value, Diagnostic> {
        self.at += 1;
        let mut entries = Vec::new();
        self.blanks();
        if self.eat('}') {
            return Ok(Value::Table(entries));
        }
        loop {
            self.blanks();
            let entry = self.entry(depth + 1)?;
            add(&mut entries, entry)?;
            self.blanks();
            if self.eat('}') {
                return Ok(Value::Table(entries));
            }
            if !self.eat(',') {
                return Err(error("expected `,` or `}`", self.here()));
            }
        }
    }
}

fn add(entries: &mut Vec<Entry>, entry: Entry) -> Result<(), Diagnostic> {
    if entries.iter().any(|old| old.key == entry.key) {
        return Err(error(format!("key `{}` appears twice", entry.key), entry.span));
    }
    entries.push(entry);
    Ok(())
}

/// True if `use name` can spell this name.
fn valid_name(name: &str) -> bool {
    let mut letters = name.chars();
    letters.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && letters.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !KEYWORDS.contains(&name)
        && name != "std"
}

fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    let number = |part: &&str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    parts.len() == 3 && parts.iter().all(number)
}

fn check_name(name: &str, span: Span, diags: &mut Vec<Diagnostic>) {
    if !valid_name(name) {
        let message = format!("`{name}` cannot be a package name");
        let note = "use lowercase letters, digits, and `_`, and no keyword or `std`";
        diags.push(error(message, span).note(note));
    }
}

fn text_list(entry: &Entry, diags: &mut Vec<Diagnostic>) {
    let text = |(item, _): &(Value, Span)| matches!(item, Value::Text(_));
    if !matches!(&entry.value, Value::List(items) if items.iter().all(text)) {
        diags.push(error(format!("`{}` must be a list of strings", entry.key), entry.span));
    }
}

fn read_package(table: &Table, diags: &mut Vec<Diagnostic>) -> Option<String> {
    let (mut name, mut version) = (None, None);
    for entry in &table.entries {
        match (entry.key.as_str(), &entry.value) {
            ("name", Value::Text(text)) => {
                check_name(text, entry.span, diags);
                name = Some(text.clone());
            }
            ("version", Value::Text(text)) => {
                if !valid_version(text) {
                    let message = format!("version `{text}` is not three numbers: `1.2.3`");
                    diags.push(error(message, entry.span));
                }
                version = Some(text);
            }
            ("name" | "version", _) => {
                diags.push(error(format!("`{}` must be a string", entry.key), entry.span));
            }
            (key, _) => {
                diags.push(error(format!("`[package]` has no key `{key}`"), entry.span));
            }
        }
    }
    if version.is_none() {
        diags.push(error("`[package]` needs `version`", table.span));
    }
    if name.is_none() {
        diags.push(error("`[package]` needs `name`", table.span));
    }
    name
}

fn read_permissions(table: &Table, diags: &mut Vec<Diagnostic>) {
    for entry in &table.entries {
        let key = entry.key.as_str();
        if !PERMISSIONS.contains(&key) {
            diags.push(error(format!("no permission `{key}`"), entry.span));
        } else if matches!(key, "files" | "env") {
            text_list(entry, diags);
        } else if !matches!(entry.value, Value::Bool) {
            diags.push(error(format!("`{key}` must be `true` or `false`"), entry.span));
        }
    }
}

fn read_dependency(entry: &Entry, diags: &mut Vec<Diagnostic>) -> Option<Dependency> {
    check_name(&entry.key, entry.span, diags);
    let mut path = None;
    if let Value::Table(fields) = &entry.value {
        for field in fields {
            match (field.key.as_str(), &field.value) {
                ("path", Value::Text(text)) => path = Some(text.clone()),
                ("version", Value::Text(_)) => {}
                ("allow", value) => {
                    text_list(field, diags);
                    let Value::List(items) = value else { continue };
                    for (item, span) in items {
                        if let Value::Text(text) = item
                            && !PERMISSIONS.contains(&text.as_str())
                        {
                            diags.push(error(format!("no permission `{text}`"), *span));
                        }
                    }
                }
                ("path" | "version", _) => {
                    diags.push(error(format!("`{}` must be a string", field.key), field.span));
                }
                (key, _) => {
                    diags.push(error(format!("a dependency has no key `{key}`"), field.span));
                }
            }
        }
    }
    if path.is_none() {
        let message = format!("dependency `{}` needs a `path`", entry.key);
        let note = "write `name = { path = \"../name\" }`. A registry is planned for M8";
        diags.push(error(message, entry.span).note(note));
    }
    Some(Dependency { name: entry.key.clone(), path: path?, span: entry.span })
}

fn read_tables(tables: &[Table]) -> (Option<Manifest>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut name = None;
    let mut dependencies = Vec::new();
    for table in tables {
        match table.name.as_str() {
            "package" => name = read_package(table, &mut diags),
            "permissions" => read_permissions(table, &mut diags),
            "dependencies" => {
                let read = table.entries.iter();
                dependencies.extend(read.filter_map(|e| read_dependency(e, &mut diags)));
            }
            other => diags.push(error(format!("no table `[{other}]`"), table.span)),
        }
    }
    if !tables.iter().any(|table| table.name == "package") {
        diags.push(error("`lucid.toml` needs a `[package]` table", Span { start: 0, end: 0 }));
    }
    if !diags.is_empty() {
        return (None, diags);
    }
    (name.map(|name| Manifest { name, dependencies }), diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(text: &str) -> Vec<String> {
        parse(text).1.into_iter().map(|d| d.message).collect()
    }

    fn bad(text: &str, part: &str) {
        let found = errors(text);
        assert!(found.iter().any(|m| m.contains(part)), "{text:?} gave {found:?}");
    }

    const HEAD: &str = "[package]\nname = \"a\"\nversion = \"0.1.0\"\n";

    #[test]
    fn reads_a_full_manifest() {
        let text = "\u{feff}# top\r\n[package] # note\r\nname = \"blog\"\nversion = '0.1.0'\n\n\
            [permissions]\nnet = true\nunsafe = false\nfiles = [\n  \"./data\", # here\n  'x',\n]\n\
            env = []\n\n[dependencies]\nmd = { version = \"1.2\", path = \"..\\\\md\", allow = [] }\n\
            mail = {path='../mail',allow=[\"net\"]}\n";
        let (manifest, diags) = parse(text);
        assert!(diags.is_empty(), "{:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
        let manifest = manifest.expect("a manifest");
        assert_eq!(manifest.name, "blog");
        assert_eq!(manifest.dependencies[0].path, "..\\md");
        assert_eq!(manifest.dependencies[1].name, "mail");
    }

    #[test]
    fn rejects_what_it_does_not_read() {
        bad("", "needs a `[package]`");
        bad("[package]\n", "needs `name`");
        bad("[package]\n", "needs `version`");
        bad("[package]\nname = 1\n", "expected a string");
        bad("[package]\nname = \"task\"\nversion = \"1.0.0\"\n", "cannot be a package name");
        bad("[package]\nname = \"std\"\nversion = \"1.0.0\"\n", "cannot be a package name");
        bad("[package]\nname = \"A-b\"\nversion = \"1.0.0\"\n", "cannot be a package name");
        bad("[package]\nname = \"a\"\nversion = \"1.2\"\n", "three numbers");
        bad(&format!("{HEAD}name = \"b\"\n"), "appears twice");
        bad(&format!("{HEAD}[package]\n"), "appears twice");
        bad(&format!("{HEAD}[[x]]\n"), "arrays of tables");
        bad(&format!("{HEAD}[dependencies.x]\n"), "expected `]`");
        bad(&format!("{HEAD}[other]\n"), "no table");
        bad(&format!("{HEAD}extra = true\n"), "has no key");
        bad(&format!("{HEAD}[permissions]\nweb = true\n"), "no permission");
        bad(&format!("{HEAD}[permissions]\nnet = \"yes\"\n"), "must be `true`");
        bad(&format!("{HEAD}[permissions]\nfiles = true\n"), "list of strings");
        bad(&format!("{HEAD}[permissions]\nfiles = [true]\n"), "list of strings");
        bad(&format!("{HEAD}[dependencies]\nx = \"1.2\"\n"), "needs a `path`");
        bad(&format!("{HEAD}[dependencies]\nx = {{ path = \"a\", }}\n"), "expected a key");
        bad(&format!("{HEAD}[dependencies]\nx = {{ path = \"a\"\n"), "expected `,` or `}`");
        bad(&format!("{HEAD}[dependencies]\nx = {{ path = \"a\", z = true }}\n"), "no key `z`");
        bad(&format!("{HEAD}[dependencies]\nx = {{ path = \"a\", allow = [\"y\"] }}\n"), "no perm");
        bad(&format!("{HEAD}x = \"\\u0041\"\n"), "not supported");
        bad(&format!("{HEAD}x = \"open\n"), "no end");
        bad(&format!("{HEAD}x = 'open\n"), "no end");
        bad(&format!("{HEAD}x = true false\n"), "end of the line");
        bad("x = true\n", "must follow a table");
        bad(&format!("{HEAD}x = [{}\n", "[".repeat(100_000)), "too deep");
    }

    /// Only structure is checked, so no input may panic: cut a valid manifest
    /// at each byte, which also stops inside a multi-byte character.
    #[test]
    fn survives_any_prefix() {
        let text =
            "[package]\nname = \"é\"\nversion = '1.0.0'\n[dependencies]\nx = { path = 'é' }\n";
        for end in (0..=text.len()).filter(|&end| text.is_char_boundary(end)) {
            parse(&text[..end]);
        }
    }
}
