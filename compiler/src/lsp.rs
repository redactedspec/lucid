//! `lucid lsp`: a language server on stdin and stdout, by decision 202. It
//! keeps the text of each open document, checks each one as `lucid check`
//! does, with the open texts in place of the files, and answers formatting, go
//! to definition, and hover. Positions count UTF-16 units. See "`lucid lsp`"
//! in docs/compiler.md.

use crate::diag::{Diagnostic, Source, Span, file_index, json_string, plain};
use crate::mir::Ref;
use crate::resolve::{OPEN, load};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

/// A JSON value, as the protocol sends it.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The value at a path of object keys.
    fn at(&self, keys: &[&str]) -> &Json {
        keys.iter().fold(self, |value, key| match value {
            Json::Object(fields) => {
                fields.iter().find(|(k, _)| k == key).map_or(&Json::Null, |f| &f.1)
            }
            _ => &Json::Null,
        })
    }

    fn text(&self) -> Option<&str> {
        if let Json::Text(text) = self { Some(text) } else { None }
    }

    fn number(&self) -> Option<f64> {
        if let Json::Number(n) = self { Some(*n) } else { None }
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => *out += "null",
            Json::Bool(b) => *out += if *b { "true" } else { "false" },
            Json::Number(n) => *out += &n.to_string(),
            Json::Text(text) => *out += &json_string(text),
            Json::List(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    *out += &json_string(key);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Reads one JSON value that fills `text`, by RFC 8259.
pub fn parse(text: &str) -> Option<Json> {
    let mut reader = Reader { bytes: text.as_bytes(), at: 0, depth: 0 };
    let value = reader.value()?;
    reader.space();
    (reader.at == reader.bytes.len()).then_some(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while self.bytes.get(self.at).is_some_and(|b| b" \t\r\n".contains(b)) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.space();
        let found = self.bytes.get(self.at) == Some(&byte);
        self.at += usize::from(found);
        found
    }

    fn word(&mut self, word: &str, value: Json) -> Option<Json> {
        let found = self.bytes[self.at..].starts_with(word.as_bytes());
        self.at += word.len();
        found.then_some(value)
    }

    fn value(&mut self) -> Option<Json> {
        self.space();
        // A deep nest would overflow the stack, and no message needs one.
        if self.depth > 64 {
            return None;
        }
        match *self.bytes.get(self.at)? {
            b'n' => self.word("null", Json::Null),
            b't' => self.word("true", Json::Bool(true)),
            b'f' => self.word("false", Json::Bool(false)),
            b'"' => self.string().map(Json::Text),
            b'[' => {
                self.at += 1;
                self.depth += 1;
                let mut items = Vec::new();
                if !self.eat(b']') {
                    loop {
                        items.push(self.value()?);
                        if self.eat(b']') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                self.depth -= 1;
                Some(Json::List(items))
            }
            b'{' => {
                self.at += 1;
                self.depth += 1;
                let mut fields = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.space();
                        let key = self.string()?;
                        if !self.eat(b':') {
                            return None;
                        }
                        fields.push((key, self.value()?));
                        if self.eat(b'}') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                self.depth -= 1;
                Some(Json::Object(fields))
            }
            _ => {
                let start = self.at;
                let number = |b: &u8| b.is_ascii_digit() || b"+-.eE".contains(b);
                while self.bytes.get(self.at).is_some_and(number) {
                    self.at += 1;
                }
                let text = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
                text.parse().ok().map(Json::Number)
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.bytes.get(self.at) != Some(&b'"') {
            return None;
        }
        self.at += 1;
        let mut units: Vec<u16> = Vec::new();
        let mut out = String::new();
        loop {
            let start = self.at;
            while !matches!(self.bytes.get(self.at), Some(b'"' | b'\\') | None) {
                self.at += 1;
            }
            if self.at > start {
                out += &String::from_utf16_lossy(&std::mem::take(&mut units));
                out += std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
            }
            match self.bytes.get(self.at)? {
                b'"' => {
                    self.at += 1;
                    out += &String::from_utf16_lossy(&units);
                    return Some(out);
                }
                _ => {
                    let escape = *self.bytes.get(self.at + 1)?;
                    self.at += 2;
                    let plain = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hex = self.bytes.get(self.at..self.at + 4)?;
                            self.at += 4;
                            let hex = std::str::from_utf8(hex).ok()?;
                            // Surrogate pairs come as two escapes, so units wait for their pair.
                            units.push(u16::from_str_radix(hex, 16).ok()?);
                            continue;
                        }
                        _ => return None,
                    };
                    out += &String::from_utf16_lossy(&std::mem::take(&mut units));
                    out.push(plain);
                }
            }
        }
    }
}

fn object(fields: &[(&str, Json)]) -> Json {
    Json::Object(fields.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect())
}

fn text(s: &str) -> Json {
    Json::Text(s.to_string())
}

/// Reads one message: headers, a blank line, then `Content-Length` bytes.
/// `None` is the end of input. `Some(None)` is a frame with no length or a
/// body that is not UTF-8, which the server answers and lives past.
fn read_message(input: &mut impl BufRead) -> Option<Option<String>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(length) = length else { return Some(None) };
    let mut body = vec![0; length];
    input.read_exact(&mut body).ok()?;
    Some(String::from_utf8(body).ok())
}

fn send(message: &Json) {
    let mut body = String::new();
    message.write(&mut body);
    let mut out = std::io::stdout().lock();
    // A closed stdout means the editor is gone, and the next read ends the loop.
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

fn respond(id: &Json, result: Json) {
    send(&object(&[("jsonrpc", text("2.0")), ("id", id.clone()), ("result", result)]));
}

fn respond_error(id: &Json, code: i32, message: &str) {
    let error = object(&[("code", Json::Number(code.into())), ("message", text(message))]);
    send(&object(&[("jsonrpc", text("2.0")), ("id", id.clone()), ("error", error)]));
}

fn notify(method: &str, params: Json) {
    send(&object(&[("jsonrpc", text("2.0")), ("method", text(method)), ("params", params)]));
}

/// The path of a `file:` URI, with its percent escapes read.
fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///C:/x` holds a Windows path after its third slash, and
    // `file://server/share/x` a network path, `//server/share/x`.
    let rest = match rest.as_bytes() {
        [b'/', _, b':', ..] | [b'/', _, b'%', b'3', b'A' | b'a', ..] if cfg!(windows) => &rest[1..],
        [b'/', ..] => rest,
        _ => &uri["file:".len()..],
    };
    let mut bytes = Vec::new();
    let mut parts = rest.as_bytes().iter();
    while let Some(&b) = parts.next() {
        if b == b'%' {
            let hex = [*parts.next()?, *parts.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).ok()
}

/// A path's canonical form, or the path itself for a file not on disk.
fn real(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// A `file:` URI for a path.
fn path_uri(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    let path = match path.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(&path).to_string(),
    };
    let path = path.replace('\\', "/");
    // A network path `//server/share` keeps the server as the URI's host.
    let (host, path) = match path.strip_prefix("//") {
        Some(share) => share.split_once('/').map_or((share, ""), |(host, rest)| (host, rest)),
        None => ("", path.as_str()),
    };
    let mut uri = format!("file://{host}");
    if !path.starts_with('/') {
        uri.push('/');
    }
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            uri.push(char::from(b));
        } else {
            uri += &format!("%{b:02X}");
        }
    }
    uri
}

/// One open document and what its last check found.
struct Document {
    path: String,
    real: PathBuf,
    source: Source,
    refs: Vec<Ref>,
    /// Where its text starts in `source`, if the last check read it.
    start: Option<usize>,
}

/// The zero-based line and UTF-16 column of an offset in `source`, in its file.
fn position(source: &Source, offset: usize) -> Json {
    let file = &source.files[file_index(&source.files, offset)];
    let offset = offset.min(source.text.len());
    let before = &source.text[file.start..offset];
    let line = before.matches('\n').count();
    let line_text = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
    // An editor does not show a byte order mark, so it takes no column.
    let line_text = if line == 0 { line_text.trim_start_matches('\u{feff}') } else { line_text };
    let column = line_text.encode_utf16().count();
    object(&[("line", Json::Number(line as f64)), ("character", Json::Number(column as f64))])
}

fn range(source: &Source, span: Span) -> Json {
    object(&[("start", position(source, span.start)), ("end", position(source, span.end))])
}

/// The offset in `text` of a zero-based line and UTF-16 column. A column past
/// the line's end is its end.
fn offset(text: &str, line: usize, column: usize) -> usize {
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    let mut units = 0;
    for (i, c) in text[start..].char_indices() {
        // A byte order mark takes no column, as in `position`.
        if start == 0 && i == 0 && c == '\u{feff}' {
            continue;
        }
        if units >= column || c == '\n' {
            return start + i;
        }
        units += c.len_utf16();
    }
    text.len()
}

/// The written name at `at`, as the checker found it: the narrowest span that
/// holds the offset, so a type argument wins over the type around it.
fn find_ref(refs: &[Ref], at: usize) -> Option<Ref> {
    let holds = refs.iter().filter(|r| r.at.start <= at && at <= r.at.end);
    holds.min_by_key(|r| r.at.end - r.at.start).copied()
}

/// Checks the document at `path` as `lucid check` does. A panic in the
/// compiler is a bug: the panic message goes to stderr, the editor's log, and
/// the server lives on with no results for this document.
// ponytail: each check leaks the compiler's small `'static` strings, such as
// paths and type names, so a long session grows slowly. Free them per check
// if an editor's memory shows it.
fn check(path: &str) -> (Source, Vec<Diagnostic>, Vec<Ref>) {
    let checked = std::panic::catch_unwind(|| {
        let (source, file, mut diags) = load(path, false);
        let mut refs = Vec::new();
        if diags.is_empty()
            && let Some(file) = &file
        {
            let (mut program, errors) = crate::check::check_with(file, &source, true, true);
            diags = errors;
            if diags.is_empty() {
                diags = crate::mono::run(&mut program, true);
            }
            diags.retain(|d| d.code != "E0901");
            refs = program.refs;
        }
        (source, diags, refs)
    });
    checked.unwrap_or_else(|_| (Source::default(), Vec::new(), Vec::new()))
}

fn diagnostic_json(source: &Source, d: &Diagnostic, span: Span, prefix: &str) -> Json {
    let mut message = format!("{prefix}{}", plain(&d.message));
    for note in &d.notes {
        message += &format!("\nnote: {}", plain(note));
    }
    object(&[
        ("range", range(source, span)),
        ("severity", Json::Number(1.0)),
        ("code", text(d.code)),
        ("source", text("lucid")),
        ("message", Json::Text(message)),
    ])
}

#[derive(Default)]
struct Server {
    /// Open documents, by the URI the editor gave.
    documents: HashMap<String, Document>,
    /// The URIs that the last round gave diagnostics.
    published: HashSet<String>,
}

impl Server {
    /// Checks every open document, since a change to one can change what
    /// another finds, and sends each file's diagnostics. A file that had some
    /// and now has none gets an empty list, which clears them.
    fn publish(&mut self) {
        let mut found: HashMap<String, Vec<String>> = HashMap::new();
        let mut items: HashMap<String, Vec<Json>> = HashMap::new();
        for uri in self.published.drain().chain(self.documents.keys().cloned()) {
            found.entry(uri).or_default();
        }
        let by_path: HashMap<PathBuf, String> =
            self.documents.iter().map(|(uri, doc)| (doc.real.clone(), uri.clone())).collect();
        for (uri, doc) in &mut self.documents {
            let (source, diags, refs) = check(&doc.path);
            let entry = source.files.iter().find(|file| real(file.path) == doc.real);
            doc.start = entry.map(|file| file.start);
            for d in &diags {
                let file = source.files[file_index(&source.files, d.span.start)].path;
                // A file of `std` has no URI, so its error shows at this
                // document's start, named.
                let (target, json) = if file.starts_with('@') {
                    let at = doc.start.unwrap_or(0);
                    let span = Span { start: at, end: at };
                    (uri.clone(), diagnostic_json(&source, d, span, &format!("{file}: ")))
                } else {
                    let path = real(file);
                    let target = by_path.get(&path).cloned().unwrap_or_else(|| path_uri(&path));
                    (target, diagnostic_json(&source, d, d.span, ""))
                };
                let mut key = String::new();
                json.write(&mut key);
                // Two documents that load one module find its errors twice.
                let seen = found.entry(target.clone()).or_default();
                if !seen.contains(&key) {
                    seen.push(key);
                    items.entry(target).or_default().push(json);
                }
            }
            (doc.source, doc.refs) = (source, refs);
        }
        for uri in found.into_keys() {
            let list = items.remove(&uri).unwrap_or_default();
            if !list.is_empty() {
                self.published.insert(uri.clone());
            }
            let params = object(&[("uri", Json::Text(uri)), ("diagnostics", Json::List(list))]);
            notify("textDocument/publishDiagnostics", params);
        }
    }

    fn open(&mut self, uri: &str, text: String) {
        let Some(path) = uri_path(uri) else { return };
        let real = real(&path);
        OPEN.with_borrow_mut(|open| open.insert(real.clone(), text));
        let (source, refs, start) = (Source::default(), Vec::new(), None);
        self.documents.insert(uri.to_string(), Document { path, real, source, refs, start });
    }

    /// The offset in the last check's source of a request's position.
    fn place(&self, params: &Json) -> Option<(&Document, usize)> {
        let doc = self.documents.get(params.at(&["textDocument", "uri"]).text()?)?;
        let text = OPEN.with_borrow(|open| open.get(&doc.real).cloned())?;
        let line = params.at(&["position", "line"]).number()? as usize;
        let column = params.at(&["position", "character"]).number()? as usize;
        Some((doc, doc.start? + offset(&text, line, column)))
    }

    fn definition(&self, params: &Json) -> Json {
        let Some((doc, at)) = self.place(params) else { return Json::Null };
        let Some(found) = find_ref(&doc.refs, at) else { return Json::Null };
        let file = doc.source.files[file_index(&doc.source.files, found.to.start)].path;
        if file.starts_with('@') {
            return Json::Null;
        }
        let path = real(file);
        let open = self.documents.iter().find(|(_, d)| d.real == path);
        let uri = open.map_or_else(|| path_uri(&path), |(uri, _)| uri.clone());
        object(&[("uri", Json::Text(uri)), ("range", range(&doc.source, found.to))])
    }

    fn hover(&self, params: &Json) -> Json {
        let Some((doc, at)) = self.place(params) else { return Json::Null };
        let Some(found) = find_ref(&doc.refs, at) else { return Json::Null };
        let all = &doc.source.text;
        let shown = match found.ty {
            Some(ty) => {
                format!("```lucid\n{}: {}\n```", &all[found.to.start..found.to.end], ty.name())
            }
            None => {
                // The declaration's line, with the `##` lines just above it.
                // The scan stops at the file's start, so no other file's lines join it.
                let file = doc.source.files[file_index(&doc.source.files, found.to.start)].start;
                let start = all[..found.to.start].rfind('\n').map_or(0, |i| i + 1).max(file);
                let end =
                    all[found.to.start..].find('\n').map_or(all.len(), |i| found.to.start + i);
                let mut docs = Vec::new();
                for line in all[file..start].lines().rev() {
                    let Some(doc_line) = line.trim_start().strip_prefix("##") else { break };
                    docs.push(doc_line.strip_prefix(' ').unwrap_or(doc_line));
                }
                docs.reverse();
                let mut shown = format!("```lucid\n{}\n```", plain(all[start..end].trim()));
                if !docs.is_empty() {
                    shown += "\n\n";
                    shown += &docs.join("\n");
                }
                shown
            }
        };
        let contents = object(&[("kind", text("markdown")), ("value", Json::Text(shown))]);
        object(&[("contents", contents), ("range", range(&doc.source, found.at))])
    }

    fn format(&self, params: &Json) -> Result<Json, String> {
        let uri = params.at(&["textDocument", "uri"]).text().unwrap_or("");
        let Some(doc) = self.documents.get(uri) else { return Ok(Json::Null) };
        let Some(old) = OPEN.with_borrow(|open| open.get(&doc.real).cloned()) else {
            return Ok(Json::Null);
        };
        let new = crate::format_text(&old)?;
        if new == old {
            return Ok(Json::List(Vec::new()));
        }
        // One edit replaces the whole text, to the end of its last line.
        let lines = old.matches('\n').count();
        let last = old[old.rfind('\n').map_or(0, |i| i + 1)..].encode_utf16().count();
        let end = object(&[
            ("line", Json::Number(lines as f64)),
            ("character", Json::Number(last as f64)),
        ]);
        let start = object(&[("line", Json::Number(0.0)), ("character", Json::Number(0.0))]);
        let range = object(&[("start", start), ("end", end)]);
        Ok(Json::List(vec![object(&[("range", range), ("newText", Json::Text(new))])]))
    }
}

/// Runs the server until `exit`, or until stdin ends.
pub fn serve() -> ExitCode {
    let mut input = std::io::stdin().lock();
    let mut server = Server::default();
    let mut shut_down = false;
    while let Some(body) = read_message(&mut input) {
        let Some(message) = body.as_deref().and_then(parse) else {
            respond_error(&Json::Null, -32700, "the message is not JSON");
            continue;
        };
        let id = message.at(&["id"]);
        let params = message.at(&["params"]);
        let method = message.at(&["method"]).text().unwrap_or("");
        let request = *id != Json::Null;
        if shut_down && method != "exit" {
            if request {
                respond_error(id, -32600, "the server is shut down");
            }
            continue;
        }
        match method {
            "initialize" => {
                let capabilities = object(&[
                    ("positionEncoding", text("utf-16")),
                    ("textDocumentSync", Json::Number(1.0)),
                    ("documentFormattingProvider", Json::Bool(true)),
                    ("definitionProvider", Json::Bool(true)),
                    ("hoverProvider", Json::Bool(true)),
                ]);
                let info = object(&[
                    ("name", text("lucid")),
                    ("version", text(env!("CARGO_PKG_VERSION"))),
                ]);
                respond(id, object(&[("capabilities", capabilities), ("serverInfo", info)]));
            }
            "shutdown" => {
                shut_down = true;
                respond(id, Json::Null);
            }
            "exit" => return if shut_down { ExitCode::SUCCESS } else { ExitCode::FAILURE },
            "textDocument/didOpen" => {
                let uri = params.at(&["textDocument", "uri"]).text().unwrap_or("");
                let text = params.at(&["textDocument", "text"]).text().unwrap_or("");
                server.open(uri, text.to_string());
                server.publish();
            }
            "textDocument/didChange" => {
                let uri = params.at(&["textDocument", "uri"]).text().unwrap_or("");
                // Full sync: the last change holds the whole text.
                let changes = match params.at(&["contentChanges"]) {
                    Json::List(changes) => changes.as_slice(),
                    _ => &[],
                };
                if let Some(text) = changes.last().and_then(|c| c.at(&["text"]).text())
                    && server.documents.contains_key(uri)
                {
                    server.open(uri, text.to_string());
                    server.publish();
                }
            }
            "textDocument/didClose" => {
                let uri = params.at(&["textDocument", "uri"]).text().unwrap_or("");
                if let Some(doc) = server.documents.remove(uri) {
                    OPEN.with_borrow_mut(|open| open.remove(&doc.real));
                    server.publish();
                }
            }
            "textDocument/definition" => respond(id, server.definition(params)),
            "textDocument/hover" => respond(id, server.hover(params)),
            "textDocument/formatting" => match server.format(params) {
                Ok(edits) => respond(id, edits),
                Err(problem) => respond_error(id, -32803, &problem),
            },
            _ if request => respond_error(id, -32601, &format!("`{method}` is not supported")),
            _ => {}
        }
    }
    if shut_down { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_reads_what_it_writes() {
        let text = r#"{"a":[1,-2.5e1,true,null],"b":"x\"\n\u00e9\ud83d\ude00"}"#;
        let value = parse(text).unwrap();
        assert_eq!(value.at(&["b"]).text(), Some("x\"\né😀"));
        let mut out = String::new();
        value.write(&mut out);
        assert_eq!(parse(&out), Some(value));
        assert_eq!(parse("[1,]"), None);
        assert_eq!(parse("{} x"), None);
        assert_eq!(parse(&"[".repeat(100_000)), None);
    }

    #[test]
    fn offsets_count_utf16_units() {
        let text = "a😀b\nxy";
        assert_eq!(offset(text, 0, 3), "a😀".len());
        assert_eq!(offset(text, 0, 99), "a😀b".len());
        assert_eq!(offset(text, 1, 1), "a😀b\nx".len());
        assert_eq!(offset("\u{feff}ab", 0, 1), "\u{feff}a".len());
        let mut source = Source::default();
        source.add("f", text);
        let at = |o| {
            (
                position(&source, o).at(&["line"]).number(),
                position(&source, o).at(&["character"]).number(),
            )
        };
        assert_eq!(at("a😀".len()), (Some(0.0), Some(3.0)));
        assert_eq!(at("a😀b\nx".len()), (Some(1.0), Some(1.0)));
    }

    #[test]
    fn uris_round_trip() {
        let path = if cfg!(windows) { "C:/a b/é.lcd" } else { "/a b/é.lcd" };
        let uri = path_uri(std::path::Path::new(path));
        assert!(uri.starts_with("file:///"), "{uri}");
        assert_eq!(uri_path(&uri).as_deref(), Some(path));
        if cfg!(windows) {
            assert_eq!(uri_path("file:///c%3A/x.lcd").as_deref(), Some("c:/x.lcd"));
            let share = path_uri(std::path::Path::new(r"\\?\UNC\server\share\x.lcd"));
            assert_eq!(share, "file://server/share/x.lcd");
            assert_eq!(uri_path(&share).as_deref(), Some("//server/share/x.lcd"));
        }
    }
}
