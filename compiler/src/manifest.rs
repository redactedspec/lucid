//! Reads `lucid.toml`: the part of TOML 1.0 that a manifest needs, by hand, since
//! the compiler takes no TOML dependency. See decision 121.
//!
//! Tables hold keys with bare names. A value is a string, `true`, `false`, an
//! array, or an inline table. Anything else is E0338, never a guess.

use crate::diag::{Diagnostic, Span};
pub use crate::diag::{Grant, NOTHING, PERMISSIONS};
use crate::lexer::KEYWORDS;
use std::path::{Component, Path, PathBuf};

/// Deep enough for any manifest, and it keeps hostile input off the stack.
const MAX_DEPTH: usize = 8;

pub struct Manifest {
    pub name: String,
    pub version: String,
    pub dependencies: Vec<Dependency>,
    /// `[registry] url`, as written, with its span.
    pub registry: Option<(String, Span)>,
    /// One for each of `PERMISSIONS`, in that order.
    pub permissions: Vec<Grant>,
}

/// True if this grant gives all that `other` gives. `files` compares paths
/// after `.` and `..` are removed, and a path covers all below it.
pub fn covers(own: &Grant, other: &Grant, permission: &str) -> bool {
    let (Grant::List(mine), Grant::List(theirs)) = (own, other) else {
        return *own == Grant::All;
    };
    // An empty path, as `.` becomes, is a prefix of every path, so it
    // covers only paths that stay inside it.
    let inside =
        |item: &Path| matches!(item.components().next(), None | Some(Component::Normal(_)));
    let covered = |item: &String| {
        mine.iter().any(|own| match permission {
            "files" => {
                let (item, own) = (normal(item), normal(own));
                item.starts_with(&own) && (!own.as_os_str().is_empty() || inside(&item))
            }
            _ => own == item,
        })
    };
    theirs.iter().all(covered)
}

/// A path with `.` removed, and each `..` taken against the part before it.
/// A `..` with no part before it stays.
pub fn normal(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[derive(Clone)]
pub struct Dependency {
    pub name: String,
    /// As written, relative to the manifest. Empty for a registry dependency
    /// that `lucid.lock` and the cache do not cover yet.
    pub path: String,
    /// The lowest version wanted from the registry, if there is no `path`.
    pub version: Option<String>,
    pub span: Span,
    /// The permissions that its `allow` entry names, each with its span.
    pub allow: Vec<(String, Span)>,
}

enum Value {
    Text(String),
    Bool(bool),
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
                Value::Bool(true)
            }
            _ if self.text[self.at..].starts_with("false") => {
                self.at += 5;
                Value::Bool(false)
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
pub fn valid_name(name: &str) -> bool {
    let mut letters = name.chars();
    letters.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && letters.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !KEYWORDS.contains(&name)
        && name != "std"
}

pub fn valid_version(version: &str) -> bool {
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

fn read_package(table: &Table, diags: &mut Vec<Diagnostic>) -> Option<(String, String)> {
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
    Some((name?, version?.clone()))
}

/// `[registry]`: its `url`, an `https://` address or a local directory.
fn read_registry(table: &Table, diags: &mut Vec<Diagnostic>) -> Option<(String, Span)> {
    let mut url = None;
    for entry in &table.entries {
        match (entry.key.as_str(), &entry.value) {
            ("url", Value::Text(text)) => {
                // Any other scheme, or `HTTPS://`, would read as a directory.
                if text.is_empty() || (text.contains("://") && !text.starts_with("https://")) {
                    let message = "`url` must be an `https://` address or a directory";
                    diags.push(error(message, entry.span));
                }
                url = Some((text.clone(), entry.span));
            }
            ("url", _) => diags.push(error("`url` must be a string", entry.span)),
            (key, _) => {
                diags.push(error(format!("`[registry]` has no key `{key}`"), entry.span));
            }
        }
    }
    if url.is_none() {
        diags.push(error("`[registry]` needs `url`", table.span));
    }
    url
}

fn read_permissions(table: &Table, diags: &mut Vec<Diagnostic>) -> Vec<Grant> {
    let mut grants = vec![NOTHING; PERMISSIONS.len()];
    for entry in &table.entries {
        let key = entry.key.as_str();
        let Some(at) = PERMISSIONS.iter().position(|&p| p == key) else {
            diags.push(error(format!("no permission `{key}`"), entry.span));
            continue;
        };
        let listed = matches!(key, "files" | "env");
        grants[at] = match &entry.value {
            Value::Bool(true) => Grant::All,
            Value::Bool(false) => NOTHING,
            Value::List(items) if listed => {
                text_list(entry, diags);
                let text = |(item, _): &(Value, Span)| match item {
                    Value::Text(text) => Some(text.clone()),
                    _ => None,
                };
                let items: Vec<String> = items.iter().filter_map(text).collect();
                if items.iter().any(String::is_empty) {
                    diags.push(error(format!("`{key}` cannot list an empty string"), entry.span));
                }
                // The runtime reads grants one per line, so a line break in
                // an item would split it into other grants. See `runtime_grants`.
                if items.iter().any(|item| item.chars().any(char::is_control)) {
                    let message = format!(
                        "`{key}` cannot list a control character. In a \"...\" string, write `\\` as `\\\\`"
                    );
                    diags.push(error(message, entry.span));
                }
                Grant::List(items)
            }
            _ if listed => {
                let message = format!("`{key}` must be `true`, `false`, or a list of strings");
                diags.push(error(message, entry.span));
                NOTHING
            }
            _ => {
                diags.push(error(format!("`{key}` must be `true` or `false`"), entry.span));
                NOTHING
            }
        };
    }
    grants
}

fn read_dependency(entry: &Entry, diags: &mut Vec<Diagnostic>) -> Option<Dependency> {
    check_name(&entry.key, entry.span, diags);
    let mut path = None;
    let mut version = None;
    let mut allow = Vec::new();
    if let Value::Table(fields) = &entry.value {
        for field in fields {
            match (field.key.as_str(), &field.value) {
                ("path", Value::Text(text)) => path = Some(text.clone()),
                ("version", Value::Text(text)) => {
                    if !valid_version(text) {
                        let message = format!("version `{text}` is not three numbers: `1.2.3`");
                        diags.push(error(message, field.span));
                    }
                    version = Some(text.clone());
                }
                ("allow", value) => {
                    text_list(field, diags);
                    let Value::List(items) = value else { continue };
                    for (item, span) in items {
                        let Value::Text(text) = item else { continue };
                        if PERMISSIONS.contains(&text.as_str()) {
                            allow.push((text.clone(), *span));
                        } else {
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
    if path.is_none() && version.is_none() {
        let message = format!("dependency `{}` needs a `path` or a `version`", entry.key);
        let note = "write `name = { path = \"../name\" }`, or `name = { version = \"1.0.0\" }`";
        diags.push(error(message, entry.span).note(note));
    }
    // A `path` wins, and its `version` is only a note to the reader.
    let version = if path.is_some() { None } else { version };
    let path = path.unwrap_or_default();
    Some(Dependency { name: entry.key.clone(), path, version, span: entry.span, allow })
}

fn read_tables(tables: &[Table]) -> (Option<Manifest>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut name = None;
    let mut registry = None;
    let mut dependencies = Vec::new();
    let mut permissions = vec![NOTHING; PERMISSIONS.len()];
    for table in tables {
        match table.name.as_str() {
            "package" => name = read_package(table, &mut diags),
            "permissions" => permissions = read_permissions(table, &mut diags),
            "registry" => registry = read_registry(table, &mut diags),
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
    let manifest =
        name.map(|(name, version)| Manifest { name, version, dependencies, registry, permissions });
    (manifest, diags)
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
            env = []\n\n[dependencies]\nmd = { version = \"1.2.0\", path = \"..\\\\md\", allow = [] }\n\
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
        bad(&format!("{HEAD}[permissions]\nfiles = [true]\n"), "list of strings");
        bad(&format!("{HEAD}[permissions]\nenv = [\"\"]\n"), "empty string");
        bad(&format!("{HEAD}[permissions]\nfiles = [\"\\new\"]\n"), "control character");
        bad(&format!("{HEAD}[permissions]\nenv = [\"A\\tB\"]\n"), "control character");
        bad(&format!("{HEAD}[dependencies]\nx = \"1.2\"\n"), "a `path` or a `version`");
        bad(&format!("{HEAD}[dependencies]\nx = {{ version = \"1.2\" }}\n"), "three numbers");
        bad(&format!("{HEAD}[registry]\n"), "needs `url`");
        bad(&format!("{HEAD}[registry]\nurl = \"http://x\"\n"), "`https://`");
        bad(&format!("{HEAD}[registry]\nurl = \"HTTPS://x\"\n"), "`https://`");
        bad(&format!("{HEAD}[registry]\nurl = \"file:///x\"\n"), "`https://`");
        bad(&format!("{HEAD}[registry]\nurl = true\n"), "must be a string");
        bad(&format!("{HEAD}[registry]\nurl = \"x\"\nkey = \"y\"\n"), "no key `key`");
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

    #[test]
    fn grants_cover_lists() {
        let list = |items: &[&str]| Grant::List(items.iter().map(|s| s.to_string()).collect());
        assert!(covers(&list(&["./data"]), &list(&["data/a", "data/b/../c", "./data"]), "files"));
        assert!(!covers(&list(&["./data"]), &list(&["data/../x"]), "files"));
        assert!(!covers(&list(&["./data"]), &list(&["database"]), "files"));
        assert!(!covers(&list(&["data/x"]), &list(&["data"]), "files"));
        assert!(covers(&Grant::All, &list(&["../anything"]), "files"));
        for own in [".", "./", "a/.."] {
            assert!(covers(&list(&[own]), &list(&["data", "./x/y", "."]), "files"));
            for outside in ["/etc", "../shared", "x/../.."] {
                assert!(!covers(&list(&[own]), &list(&[outside]), "files"), "{own} {outside}");
            }
        }
        assert!(!covers(&list(&["/"]), &Grant::All, "files"));
        assert!(!covers(&list(&["PORT"]), &list(&["PORT", "HOME"]), "env"));
        assert!(!NOTHING.held() && list(&["x"]).held());
        let (manifest, _) = parse(&format!(
            "{HEAD}[permissions]
files = true
net = false
"
        ));
        let permissions = manifest.expect("a manifest").permissions;
        assert_eq!(permissions[..2], [NOTHING, Grant::All]);
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
