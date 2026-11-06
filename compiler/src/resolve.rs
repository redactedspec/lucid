//! Loads a program's modules and joins them into one tree. Each top-level name
//! of an imported module becomes `module.name`, a form that no written name can
//! take, so later stages see one file with no clashes. The entry file keeps its
//! names. See "Modules" in docs/compiler.md and decision 115.
//!
//! All files sit end to end in one `Source`, so a span needs no file id.

use crate::ast::{
    self, ArmBody, Expr, ExprKind, IsTarget, Name, Param, Pattern, PatternKind, Stmt, StmtKind,
    TaskDef, TaskSig, TextPart, Type, TypeBody, TypeKind, TypeParam,
};
use crate::check::{PRELUDE_TRAITS, is_built_in, is_literal};
use crate::diag::{self, Diagnostic, Source, Span};
use crate::lexer::{self, TextPart as RawPart, Token, TokenKind};
use crate::manifest::{self, Dependency};
use crate::parser;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What every file sees. The checker treats these like the file's own.
const PRELUDE: &str = "trait Error
    task error_message(self) -> text
";

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Task,
    /// A `type`, `choice`, or variant: a name that a pattern reads as a test.
    Data,
    Trait,
    Value,
}

#[derive(Clone)]
struct Entry {
    full: String,
    kind: Kind,
    public: bool,
}

type Names = HashMap<String, Entry>;

/// One `use`. `item` is the name it takes from the module, if not all of it.
struct Import {
    binding: String,
    span: Span,
    module: usize,
    item: Option<Name>,
}

struct Module {
    /// Empty for the entry file.
    key: Vec<String>,
    stmts: Vec<Stmt>,
    imports: Vec<Import>,
}

#[derive(Default)]
struct Loader {
    source: Source,
    diags: Vec<Diagnostic>,
    /// In dependency order, so the entry file comes last.
    modules: Vec<Module>,
    loaded: HashMap<String, usize>,
    /// The files whose imports are being read, for finding cycles.
    open: Vec<String>,
    /// By real directory, so two spellings of a path are one package. `None`
    /// if its manifest was bad, which is reported once.
    package_ids: HashMap<PathBuf, Option<usize>>,
    packages: Vec<Package>,
}

/// A directory with a `lucid.toml`.
struct Package {
    name: String,
    /// As shown in messages: empty, or ending in `/`.
    dir: String,
    /// With each path in the form of `dir`.
    dependencies: Vec<Dependency>,
    /// `src/main.lcd` or `src/lib.lcd`.
    entry: String,
    library: bool,
}

/// `path` with a final `/`, or empty for the current directory.
fn dir_prefix(path: &str) -> String {
    match path.trim_end_matches(['/', '\\']) {
        "." => String::new(),
        "" => path.to_string(),
        dir => format!("{dir}/"),
    }
}

/// The directory of the nearest `lucid.toml` at or above a file, reached with
/// `..` so that its path stays short in messages.
fn package_above(file: &str) -> Option<String> {
    let dir = &file[..file.rfind(['/', '\\']).map_or(0, |slash| slash + 1)];
    let real = std::fs::canonicalize(if dir.is_empty() { "." } else { dir }).ok()?;
    let mut up = dir.to_string();
    for at in real.ancestors() {
        if at.join("lucid.toml").is_file() {
            return Some(up);
        }
        up += "../";
    }
    None
}

/// The package that a directory on the command line means: its own, or the one
/// around it, so a command works from anywhere in a package.
fn package_root(dir: &str) -> String {
    let dir = dir_prefix(dir);
    package_above(&format!("{dir}x")).unwrap_or(dir)
}

/// Every `.lcd` file under a package's `src`, in a stable order, so that
/// `lucid test` also reaches modules that nothing imports. Symbolic links to
/// directories are skipped, since they can loop.
pub fn package_files(package: &str) -> Vec<String> {
    fn walk(dir: &str, files: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = format!("{dir}/{}", entry.file_name().display());
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                walk(&path, files);
            } else if path.ends_with(".lcd") {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&format!("{}src", package_root(package)), &mut files);
    files
}

/// Reads and parses a program, from a file or a package directory. Returns the
/// text that spans refer to. With `run`, a library is an error.
pub fn load(path: &str, run: bool) -> (Source, Option<ast::File>, Vec<Diagnostic>) {
    let mut loader = Loader::default();
    let start = Span { start: 0, end: 0 };
    let mut entry = path.to_string();
    let mut library = false;
    if Path::new(path).is_dir() {
        let package = loader.package(&package_root(path), start);
        let Some(package) = package else { return (loader.source, None, loader.diags) };
        let package = &loader.packages[package];
        entry.clone_from(&package.entry);
        library = package.library;
        if run && library {
            let message = format!("`{}` is a library, so it cannot run", package.name);
            loader.diags.push(Diagnostic::new("E0339", message, start));
            return (loader.source, None, loader.diags);
        }
    }
    let found = loader.module(&entry, Vec::new(), start);
    let Loader { source, mut diags, mut modules, .. } = loader;
    if found.is_none() || !diags.is_empty() {
        return (source, None, diags);
    }
    // With imports, the entry file's names need a prefix too, or a module would
    // see them. Alone, it keeps its names, so that its messages stay plain.
    if modules.len() > 1
        && let Some(stem) = Path::new(&entry).file_stem()
        && let Some(last) = modules.last_mut()
    {
        last.key = vec![stem.display().to_string()];
    }
    let file = join(modules, library, &mut diags);
    (source, Some(file), diags)
}

fn shift(tokens: &mut [Token], base: usize) {
    for token in tokens {
        token.span.start += base;
        token.span.end += base;
        if let TokenKind::Text(parts) = &mut token.kind {
            for part in parts {
                if let RawPart::Expr(inner) = part {
                    shift(inner, base);
                }
            }
        }
    }
}

impl Loader {
    /// Reads a file into `source`, and returns where it starts and its text.
    fn read(&mut self, path: &str, used_at: Span) -> Option<(usize, String)> {
        let shown: &'static str = path.to_string().leak();
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.source.add(shown, "");
                let message = format!("cannot read `{path}`: {e}");
                self.diags.push(Diagnostic::new("E0001", message, used_at));
                return None;
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(e) => {
                let valid = e.utf8_error().valid_up_to();
                let prefix = String::from_utf8_lossy(&e.as_bytes()[..valid]).into_owned();
                let base = self.source.add(shown, &prefix);
                let span = Span { start: base + valid, end: base + valid };
                self.diags.push(Diagnostic::new("E0002", "file is not valid UTF-8", span));
                return None;
            }
        };
        let base = self.source.add(shown, &text);
        Some((base, text))
    }

    /// Reads the package in `dir`, once. `None` means that it is reported.
    fn package(&mut self, dir: &str, used_at: Span) -> Option<usize> {
        let real = std::fs::canonicalize(if dir.is_empty() { "." } else { dir }).ok();
        if let Some(known) = real.as_ref().and_then(|real| self.package_ids.get(real)) {
            return *known;
        }
        let id = self.read_package(dir, used_at);
        if let Some(real) = real {
            self.package_ids.insert(real, id);
        }
        id
    }

    fn read_package(&mut self, dir: &str, used_at: Span) -> Option<usize> {
        let (base, text) = self.read(&format!("{dir}lucid.toml"), used_at)?;
        let (found, diags) = manifest::parse(&text);
        self.diags.extend(diags.into_iter().map(|mut d| {
            d.span = Span { start: d.span.start + base, end: d.span.end + base };
            d
        }));
        let found = found?;
        let at = Span { start: base, end: base };
        let src = format!("{dir}src/");
        let (main, lib) = (exact_file(&src, &["main"]), exact_file(&src, &["lib"]));
        if main == lib {
            let message = if main {
                "a package has `src/main.lcd` or `src/lib.lcd`, not both"
            } else {
                "a package needs `src/main.lcd` or `src/lib.lcd`"
            };
            self.diags.push(Diagnostic::new("E0339", message, at));
            return None;
        }
        if let Some(other) = self.packages.iter().find(|package| package.name == found.name) {
            let message = format!("two packages are named `{}`", found.name);
            let note = format!("the other one is in `{}`", other.dir);
            self.diags.push(Diagnostic::new("E0339", message, at).note(note));
            return None;
        }
        let dependencies = found.dependencies.into_iter().map(|mut dependency| {
            if !Path::new(&dependency.path).is_absolute() {
                dependency.path = format!("{dir}{}", dependency.path);
            }
            dependency.path = dir_prefix(&dependency.path);
            dependency
        });
        self.packages.push(Package {
            name: found.name,
            dir: dir.to_string(),
            dependencies: dependencies.collect(),
            entry: format!("{src}{}.lcd", if main { "main" } else { "lib" }),
            library: lib,
        });
        Some(self.packages.len() - 1)
    }

    /// Reports a fault in a dependency entry, once, however many `use`s reach it.
    fn fault(&mut self, dependency: &Dependency, message: String) {
        let span = dependency.span;
        if !self.diags.iter().any(|d| d.code == "E0339" && d.span == span) {
            self.diags.push(Diagnostic::new("E0339", message, span));
        }
    }

    /// The library that a manifest entry names.
    fn dependency(&mut self, dependency: &Dependency) -> Option<usize> {
        let name = &dependency.name;
        if !Path::new(&format!("{}lucid.toml", dependency.path)).is_file() {
            self.fault(dependency, format!("no `lucid.toml` in `{}`", dependency.path));
            return None;
        }
        let id = self.package(&dependency.path, dependency.span)?;
        let package = &self.packages[id];
        if package.name != *name {
            let message = format!("the package in `{}` is named `{}`", package.dir, package.name);
            self.fault(dependency, message);
            return None;
        }
        if !package.library {
            self.fault(dependency, format!("`{name}` has no `src/lib.lcd`, so nothing can use it"));
            return None;
        }
        Some(id)
    }

    /// Loads a file and all it imports. `None` if it cannot be read as text.
    fn module(&mut self, path: &str, key: Vec<String>, used_at: Span) -> Option<usize> {
        if let Some(&id) = self.loaded.get(path) {
            return Some(id);
        }
        let (base, text) = self.read(path, used_at)?;
        let (mut tokens, mut diags) = lexer::lex(&text);
        shift(&mut tokens, base);
        for d in &mut diags {
            d.span = Span { start: d.span.start + base, end: d.span.end + base };
        }
        let (file, parse_diags) = parser::parse(tokens);
        diag::add_unless_same_line(&self.source, &mut diags, parse_diags);
        self.diags.append(&mut diags);

        self.open.push(path.to_string());
        let mut imports = Vec::new();
        for stmt in &file.stmts {
            if let StmtKind::Use { relative, path: written, alias } = &stmt.kind {
                let import = if *relative {
                    self.import(path, &key, written, alias.as_ref(), stmt.span)
                } else {
                    self.import_package(path, written, alias.as_ref(), stmt.span)
                };
                imports.extend(import);
            }
        }
        self.open.pop();
        let id = self.modules.len();
        self.modules.push(Module { key, stmts: file.stmts, imports });
        self.loaded.insert(path.to_string(), id);
        Some(id)
    }

    /// Finds what `use ./a.b` means from the file at `from`. A file `a/b.lcd`
    /// comes first, and then the name `b` in `a.lcd`.
    fn import(
        &mut self,
        from: &str,
        key: &[String],
        written: &[Name],
        alias: Option<&Name>,
        span: Span,
    ) -> Option<Import> {
        let dir = &from[..from.rfind(['/', '\\']).map_or(0, |slash| slash + 1)];
        let parts: Vec<&str> = written.iter().map(|name| name.text.as_str()).collect();
        let last = written.last().expect("a path has a name");
        let Some((found, item)) = locate(dir, &parts, last) else {
            let message = format!("cannot find module `./{}`", parts.join("."));
            let note = format!("looked for `{dir}{}.lcd`", parts.join("/"));
            self.diags.push(Diagnostic::new("E0334", message, span).note(note));
            return None;
        };
        let parent = &key[..key.len().saturating_sub(1)];
        let module_parts = &parts[..parts.len() - usize::from(item.is_some())];
        let new_key = parent.iter().cloned().chain(module_parts.iter().map(|&s| s.into()));
        let binding = alias.map_or(&last.text, |alias| &alias.text).clone();
        self.bind(&found, new_key.collect(), binding, item, span)
    }

    /// Finds what `use name.a` means from the file at `from`. `name` must be a
    /// dependency of the package that holds the file. `use name` is its
    /// `src/lib.lcd`. A path follows the rule of `use ./`, from `src`, and a
    /// single name that is no file is a name in `src/lib.lcd`.
    fn import_package(
        &mut self,
        from: &str,
        written: &[Name],
        alias: Option<&Name>,
        span: Span,
    ) -> Option<Import> {
        let (first, rest) = written.split_first().expect("a path has a name");
        // `std` stays in the tree, and reports E0901 until it exists.
        if first.text == "std" {
            return None;
        }
        let Some(dir) = package_above(from) else {
            let message = format!("`{}` is not a dependency", first.text);
            let note = "no `lucid.toml` holds this file. A local module needs `./`";
            self.diags.push(Diagnostic::new("E0337", message, span).note(note));
            return None;
        };
        let holder = self.package(&dir, span)?;
        let package = &self.packages[holder];
        let found = package.dependencies.iter().find(|d| d.name == first.text);
        let Some(dependency) = found.cloned() else {
            let message = format!("`{}` is not a dependency", first.text);
            let note = format!(
                "add it to `[dependencies]` in `{}lucid.toml`. A local module needs `./`",
                package.dir
            );
            self.diags.push(Diagnostic::new("E0337", message, span).note(note));
            return None;
        };
        let target = self.dependency(&dependency)?;
        let package = &self.packages[target];
        let src = format!("{}src/", package.dir);
        let last = written.last().expect("a path has a name");
        let mut parts: Vec<&str> = rest.iter().map(|name| name.text.as_str()).collect();
        let mut located = if parts.is_empty() {
            parts.push("lib");
            Some((package.entry.clone(), None))
        } else {
            locate(&src, &parts, last)
        };
        if located.is_none() && parts.len() == 1 {
            let name = Name { text: last.text.clone(), span: last.span };
            located = Some((package.entry.clone(), Some(name)));
            parts = vec!["lib", parts[0]];
        }
        let Some((found, item)) = located else {
            let message = format!("cannot find module `{}.{}`", first.text, parts.join("."));
            let note = format!("looked for `{src}{}.lcd`", parts.join("/"));
            self.diags.push(Diagnostic::new("E0334", message, span).note(note));
            return None;
        };
        // No written name starts with `@`, so a package's modules cannot meet
        // a local module of the same name.
        let root = format!("@{}", package.name);
        let module_parts = &parts[..parts.len() - usize::from(item.is_some())];
        let key = std::iter::once(root).chain(module_parts.iter().map(|&s| s.into())).collect();
        let binding = alias.map_or(&last.text, |alias| &alias.text).clone();
        self.bind(&found, key, binding, item, span)
    }

    /// Loads an imported file, unless that would close a cycle.
    fn bind(
        &mut self,
        found: &str,
        key: Vec<String>,
        binding: String,
        item: Option<Name>,
        span: Span,
    ) -> Option<Import> {
        if self.open.iter().any(|open| open == found) {
            let message = format!("`{found}` imports itself, through this `use`");
            let note = "move what both files need into a third module";
            self.diags.push(Diagnostic::new("E0335", message, span).note(note));
            return None;
        }
        let module = self.module(found, key, span)?;
        Some(Import { binding, span, module, item })
    }
}

/// Finds the file of a path from `dir`: `a/b.lcd` first, and then the name `b`
/// in `a.lcd`.
fn locate(dir: &str, parts: &[&str], last: &Name) -> Option<(String, Option<Name>)> {
    let file = |parts: &[&str]| format!("{dir}{}.lcd", parts.join("/"));
    if exact_file(dir, parts) {
        return Some((file(parts), None));
    }
    let module = &parts[..parts.len() - 1];
    (!module.is_empty() && exact_file(dir, module))
        .then(|| (file(module), Some(Name { text: last.text.clone(), span: last.span })))
}

/// True if the file `dir` + `parts` + `.lcd` exists with exactly this spelling.
/// A file system that ignores case must not make one file two modules, or let
/// a program work only on Windows.
fn exact_file(dir: &str, parts: &[&str]) -> bool {
    let mut at = std::path::PathBuf::from(if dir.is_empty() { "." } else { dir });
    for (place, part) in parts.iter().enumerate() {
        let last = place + 1 == parts.len();
        let name = if last { format!("{part}.lcd") } else { (*part).to_string() };
        let Ok(entries) = std::fs::read_dir(&at) else { return false };
        if !entries.flatten().any(|entry| entry.file_name() == name.as_str()) {
            return false;
        }
        at.push(name);
    }
    at.is_file()
}

/// Every top-level name of a module.
fn own_names(module: &Module) -> Names {
    let mut names = Names::new();
    let mut add = |name: &Name, kind, public| {
        let parts = module.key.iter().map(String::as_str).chain([name.text.as_str()]);
        let full = parts.collect::<Vec<_>>().join(".");
        names.entry(name.text.clone()).or_insert(Entry { full, kind, public });
    };
    for stmt in &module.stmts {
        match &stmt.kind {
            StmtKind::Task(def) if !def.sig.is_method() => {
                add(&def.sig.name, Kind::Task, def.public)
            }
            StmtKind::Type(def) => add(&def.name, Kind::Data, def.public),
            StmtKind::Trait(def) => add(&def.name, Kind::Trait, def.public),
            StmtKind::Choice(def) => {
                add(&def.name, Kind::Data, def.public);
                def.variants.iter().for_each(|variant| add(&variant.name, Kind::Data, def.public));
            }
            StmtKind::Let { name, .. } => add(name, Kind::Value, false),
            _ => {}
        }
    }
    names
}

/// Reports what a module that is not the entry file cannot hold.
fn check_library(stmts: &[Stmt], diags: &mut Vec<Diagnostic>) {
    for stmt in stmts {
        let allowed = match &stmt.kind {
            StmtKind::Use { .. }
            | StmtKind::Test { .. }
            | StmtKind::Task(_)
            | StmtKind::Type(_)
            | StmtKind::Choice(_)
            | StmtKind::Trait(_) => true,
            StmtKind::Let { mutable, value, .. } => !mutable && is_literal(value),
            _ => false,
        };
        if !allowed {
            let message = "an imported module cannot run code";
            let note = "it holds declarations and `let` with a literal value";
            diags.push(Diagnostic::new("E0336", message, stmt.span).note(note));
        }
    }
}

/// Renames every module, then puts their statements in one file. An entry file
/// that is a library follows the rules of an imported module, so `lucid check`
/// reports what would fail in a program that imports it.
fn join(modules: Vec<Module>, library_entry: bool, diags: &mut Vec<Diagnostic>) -> ast::File {
    let (prelude, _) = parser::parse(lexer::lex(PRELUDE).0);
    let mut stmts = prelude.stmts;
    let mut exports: Vec<Names> = Vec::with_capacity(modules.len());
    let count = modules.len();
    let keys: Vec<String> = modules.iter().map(|module| module.key.join(".")).collect();
    for (id, mut module) in modules.into_iter().enumerate() {
        let imported = id + 1 < count;
        if imported || library_entry {
            check_library(&module.stmts, diags);
        }
        if imported {
            // `lucid test` runs the entry file's tests.
            module.stmts.retain(|stmt| !matches!(stmt.kind, StmtKind::Test { .. }));
        }
        let own = own_names(&module);
        let mut names = own.clone();
        let mut aliases = HashMap::new();
        for import in &module.imports {
            let taken =
                names.contains_key(&import.binding) || aliases.contains_key(&import.binding);
            if taken {
                let message = format!("`{}` is already defined", import.binding);
                diags.push(Diagnostic::new("E0304", message, import.span));
                continue;
            }
            let built_in =
                is_built_in(&import.binding) || PRELUDE_TRAITS.contains(&import.binding.as_str());
            if import.item.is_some() && built_in {
                let message = format!("`{}` is already defined", import.binding);
                diags.push(Diagnostic::new("E0304", message, import.span));
                continue;
            }
            match &import.item {
                None => {
                    aliases.insert(import.binding.clone(), import.module);
                }
                Some(item) => match exports[import.module].get(&item.text) {
                    Some(entry) => {
                        check_public(entry, &item.text, item.span, diags);
                        names.insert(import.binding.clone(), entry.clone());
                    }
                    None => {
                        let message = format!("the module has no name `{}`", item.text);
                        diags.push(Diagnostic::new("E0301", message, item.span));
                    }
                },
            }
        }
        let mut renamer =
            Renamer { names, aliases, exports: &exports, keys: &keys, scopes: Vec::new(), diags };
        renamer.stmts(&mut module.stmts);
        // A `use` of `std` stays, and reports E0901 until `std` exists.
        module.stmts.retain(|stmt| match &stmt.kind {
            StmtKind::Use { relative, path, .. } => !relative && path[0].text == "std",
            _ => true,
        });
        stmts.append(&mut module.stmts);
        exports.push(own);
    }
    ast::File { stmts }
}

fn check_public(entry: &Entry, name: &str, span: Span, diags: &mut Vec<Diagnostic>) {
    if !entry.public {
        let message = format!("`{name}` is private to its module");
        let note = format!("mark it `public` in `{}` to use it here", entry.full);
        diags.push(Diagnostic::new("E0333", message, span).note(note));
    }
}

/// Rewrites one module's names. A local keeps its name, and hides a top-level
/// one by the same rules as in the checker.
struct Renamer<'a> {
    names: Names,
    /// Each module that `use` names, by the name that it has here.
    aliases: HashMap<String, usize>,
    exports: &'a [Names],
    keys: &'a [String],
    scopes: Vec<HashSet<String>>,
    diags: &'a mut Vec<Diagnostic>,
}

impl Renamer<'_> {
    fn local(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.contains(name))
    }

    fn bind(&mut self, name: &str) {
        self.scopes.last_mut().expect("a scope is open").insert(name.into());
    }

    fn scoped(&mut self, inside: impl FnOnce(&mut Self)) {
        self.scopes.push(HashSet::new());
        inside(self);
        self.scopes.pop();
    }

    fn block(&mut self, stmts: &mut [Stmt]) {
        self.scoped(|this| this.stmts(stmts));
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        stmts.iter_mut().for_each(|stmt| self.stmt(stmt));
    }

    /// Renames a top-level name that this module declares or imports.
    fn rename(&self, name: &mut Name) {
        if let Some(entry) = self.names.get(&name.text) {
            name.text.clone_from(&entry.full);
        }
    }

    /// `module.name`. A name that it lacks stays in this form, which no
    /// declaration has, so the checker reports it as it would any other.
    fn member(&mut self, module: usize, name: &str, span: Span) -> String {
        match self.exports[module].get(name) {
            Some(entry) => {
                check_public(entry, name, span, self.diags);
                entry.full.clone()
            }
            None => format!("{}.{name}", self.keys[module]),
        }
    }

    fn stmt(&mut self, stmt: &mut Stmt) {
        match &mut stmt.kind {
            StmtKind::Use { .. } | StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder => {
            }
            StmtKind::Test { body, .. } | StmtKind::Unsafe(body) => self.block(body),
            StmtKind::Task(def) => self.task(def),
            StmtKind::Type(def) => {
                self.rename(&mut def.name);
                self.type_params(&mut def.type_params);
                match &mut def.body {
                    TypeBody::Alias(target) => self.ty(target),
                    TypeBody::Fields(fields) => {
                        for field in fields {
                            self.ty(&mut field.ty);
                            field.default.iter_mut().for_each(|value| self.expr(value));
                        }
                    }
                }
            }
            StmtKind::Choice(def) => {
                self.rename(&mut def.name);
                self.type_params(&mut def.type_params);
                for variant in &mut def.variants {
                    self.rename(&mut variant.name);
                    self.params(&mut variant.params);
                }
            }
            StmtKind::Trait(def) => {
                self.rename(&mut def.name);
                self.type_params(&mut def.type_params);
                def.tasks.iter_mut().for_each(|sig| self.signature(sig));
            }
            StmtKind::Let { name, ty, value, .. } => {
                ty.iter_mut().for_each(|ty| self.ty(ty));
                self.expr(value);
                if self.scopes.is_empty() {
                    self.rename(name);
                } else {
                    self.bind(&name.text);
                }
            }
            StmtKind::If { branches, otherwise } => {
                for (condition, body) in branches {
                    self.expr(condition);
                    self.block(body);
                }
                otherwise.iter_mut().for_each(|body| self.block(body));
            }
            StmtKind::While { condition, body } => {
                self.expr(condition);
                self.block(body);
            }
            StmtKind::For { name, iterable, body, .. } => {
                self.expr(iterable);
                self.scoped(|this| {
                    this.bind(&name.text);
                    this.stmts(body);
                });
            }
            StmtKind::Return(value) => value.iter_mut().for_each(|value| self.expr(value)),
            StmtKind::Print(value) | StmtKind::Expect(value) | StmtKind::Expr(value) => {
                self.expr(value);
            }
            StmtKind::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
        }
    }

    fn task(&mut self, def: &mut TaskDef) {
        if !self.scopes.is_empty() {
            self.bind(&def.sig.name.text);
        } else if !def.sig.is_method() {
            self.rename(&mut def.sig.name);
        }
        self.signature(&mut def.sig);
        if let Some(body) = &mut def.body {
            let params = &def.sig.params;
            self.scoped(|this| {
                params.iter().for_each(|param| this.bind(&param.name.text));
                this.stmts(body);
            });
        }
    }

    fn signature(&mut self, sig: &mut TaskSig) {
        self.type_params(&mut sig.type_params);
        self.params(&mut sig.params);
        sig.returns.iter_mut().for_each(|ty| self.ty(ty));
    }

    /// A parameter that reuses a type's name is an error in the checker, which
    /// sees it only under the full name.
    fn type_params(&mut self, params: &mut [TypeParam]) {
        for param in params {
            let named = self.names.get(&param.name.text);
            if named.is_some_and(|entry| matches!(entry.kind, Kind::Data | Kind::Trait)) {
                self.rename(&mut param.name);
            }
            param.bounds.iter_mut().for_each(|bound| self.ty(bound));
        }
    }

    fn params(&mut self, params: &mut [Param]) {
        for param in params {
            param.ty.iter_mut().for_each(|ty| self.ty(ty));
            param.default.iter_mut().for_each(|value| self.expr(value));
        }
    }

    fn ty(&mut self, ty: &mut Type) {
        match &mut ty.kind {
            TypeKind::Named { path, args } => {
                args.iter_mut().for_each(|arg| self.ty(arg));
                match path.as_mut_slice() {
                    [name] => {
                        let entry = self.names.get(&name.text);
                        if entry.is_some_and(|e| matches!(e.kind, Kind::Data | Kind::Trait)) {
                            self.rename(name);
                        }
                    }
                    [module, name] => {
                        if let Some(&id) = self.aliases.get(&module.text) {
                            let span = ty.span;
                            let text = self.member(id, &name.text, span);
                            *path = vec![Name { text, span }];
                        }
                    }
                    _ => {}
                }
            }
            TypeKind::None => {}
            TypeKind::Task { params, returns } => {
                params.iter_mut().for_each(|param| self.ty(param));
                returns.iter_mut().for_each(|returns| self.ty(returns));
            }
            TypeKind::Union(terms) => terms.iter_mut().for_each(|term| self.ty(term)),
            TypeKind::Optional(inner)
            | TypeKind::Shared(inner)
            | TypeKind::Weak(inner)
            | TypeKind::Any(inner) => self.ty(inner),
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        if let ExprKind::Field { base, name } = &expr.kind
            && let ExprKind::Name(module) = &base.kind
            && !self.local(module)
            && let Some(&id) = self.aliases.get(module)
        {
            expr.kind = ExprKind::Name(self.member(id, &name.text, expr.span));
            return;
        }
        match &mut expr.kind {
            ExprKind::Name(name) => {
                if !self.local(name)
                    && let Some(entry) = self.names.get(name.as_str())
                {
                    name.clone_from(&entry.full);
                }
            }
            ExprKind::SelfValue
            | ExprKind::None
            | ExprKind::Bool(_)
            | ExprKind::Int(_)
            | ExprKind::Float(_) => {}
            ExprKind::Text(parts) => {
                for part in parts {
                    if let TextPart::Expr(value) = part {
                        self.expr(value);
                    }
                }
            }
            ExprKind::List(items) => items.iter_mut().for_each(|item| self.expr(item)),
            ExprKind::Map(entries) => {
                for (key, value) in entries {
                    self.expr(key);
                    self.expr(value);
                }
            }
            ExprKind::Unary(_, value) | ExprKind::Optional(value) => self.expr(value),
            ExprKind::Field { base, .. } => self.expr(base),
            ExprKind::Binary(_, left, right) => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Is { value, target, .. } => {
                self.expr(value);
                if let IsTarget::Type(ty) = target {
                    self.ty(ty);
                }
            }
            ExprKind::If { condition, then, otherwise } => {
                self.expr(condition);
                self.expr(then);
                self.expr(otherwise);
            }
            ExprKind::Lambda { params, body } => self.scoped(|this| {
                params.iter().for_each(|param| this.bind(&param.text));
                this.expr(body);
            }),
            ExprKind::Call { callee, args } => {
                self.expr(callee);
                args.iter_mut().for_each(|arg| self.expr(&mut arg.value));
            }
            ExprKind::Index { base, indices } => {
                self.expr(base);
                indices.iter_mut().for_each(|index| self.expr(index));
            }
            ExprKind::Type(ty) => self.ty(ty),
            ExprKind::Match { subject, arms } => {
                self.expr(subject);
                for arm in arms {
                    self.scoped(|this| {
                        this.pattern(&mut arm.pattern);
                        match &mut arm.body {
                            ArmBody::Expr(value) => this.expr(value),
                            ArmBody::Stmt(stmt) => this.stmt(stmt),
                            ArmBody::Block(body) => this.stmts(body),
                        }
                    });
                }
            }
        }
    }

    /// The name that `name` has where it is declared, if it is a type or
    /// variant. An import may have renamed it.
    fn data_name(&self, name: &str) -> Option<String> {
        let entry = self.names.get(name).filter(|entry| entry.kind == Kind::Data)?;
        Some(ast::short_name(&entry.full).to_string())
    }

    /// A variant needs no prefix: the type of the subject finds it, so a
    /// pattern needs no import. The checker rejects a binding that reads like
    /// a type or variant of any module, so only other names bind here.
    fn pattern(&mut self, pattern: &mut Pattern) {
        match &mut pattern.kind {
            PatternKind::Wildcard | PatternKind::None => {}
            PatternKind::Literal(value) => self.expr(value),
            PatternKind::Name(name) => match self.data_name(&name.text) {
                Some(written) => name.text = written,
                None => self.bind(&name.text),
            },
            PatternKind::Variant { name, bindings } => {
                if let Some(written) = self.data_name(&name.text) {
                    name.text = written;
                }
                bindings.iter().for_each(|name| self.bind(&name.text));
            }
            PatternKind::Typed { name, ty } => {
                self.ty(ty);
                self.bind(&name.text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Lines;

    #[test]
    fn shift_reaches_inside_text() {
        let (mut tokens, _) = lexer::lex("\"{a}\"\n");
        shift(&mut tokens, 100);
        let TokenKind::Text(parts) = &tokens[0].kind else { panic!() };
        let RawPart::Expr(inner) = &parts[0] else { panic!() };
        assert_eq!(inner[0].span.start, 102);
    }

    #[test]
    fn source_keeps_lines_per_file() {
        let mut source = Source::default();
        source.add("a.lcd", "x\ny");
        let base = source.add("b.lcd", "z\n");
        assert_eq!(Lines::new(&source).place(base + 1).0, "b.lcd");
        assert_eq!(Lines::new(&source).place(base).1, 1);
    }
}
