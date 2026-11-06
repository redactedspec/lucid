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
use crate::parser;
use std::collections::{HashMap, HashSet};

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
}

/// Reads and parses a program. Returns the text that spans refer to.
pub fn load(path: &str) -> (Source, Option<ast::File>, Vec<Diagnostic>) {
    let mut loader = Loader::default();
    let start = Span { start: 0, end: 0 };
    let entry = loader.module(path, Vec::new(), start);
    let Loader { source, mut diags, mut modules, .. } = loader;
    if entry.is_none() || !diags.is_empty() {
        return (source, None, diags);
    }
    // With imports, the entry file's names need a prefix too, or a module would
    // see them. Alone, it keeps its names, so that its messages stay plain.
    if modules.len() > 1
        && let Some(stem) = std::path::Path::new(path).file_stem()
        && let Some(last) = modules.last_mut()
    {
        last.key = vec![stem.display().to_string()];
    }
    let file = join(modules, &mut diags);
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
    /// Loads a file and all it imports. `None` if it cannot be read as text.
    fn module(&mut self, path: &str, key: Vec<String>, used_at: Span) -> Option<usize> {
        if let Some(&id) = self.loaded.get(path) {
            return Some(id);
        }
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
            if let StmtKind::Use { relative: true, path: written, alias } = &stmt.kind {
                let import = self.import(path, &key, written, alias.as_ref(), stmt.span);
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
        let file = |parts: &[&str]| format!("{dir}{}.lcd", parts.join("/"));
        let (found, item) = if exact_file(dir, &parts) {
            (file(&parts), None)
        } else if parts.len() > 1 && exact_file(dir, &parts[..parts.len() - 1]) {
            (
                file(&parts[..parts.len() - 1]),
                Some(Name { text: last.text.clone(), span: last.span }),
            )
        } else {
            let message = format!("cannot find module `./{}`", parts.join("."));
            let note = format!("looked for `{}`", file(&parts));
            self.diags.push(Diagnostic::new("E0334", message, span).note(note));
            return None;
        };
        if self.open.contains(&found) {
            let message = format!("`{found}` imports itself, through this `use`");
            let note = "move what both files need into a third module";
            self.diags.push(Diagnostic::new("E0335", message, span).note(note));
            return None;
        }
        let parent = &key[..key.len().saturating_sub(1)];
        let module_parts = &parts[..parts.len() - usize::from(item.is_some())];
        let new_key = parent.iter().cloned().chain(module_parts.iter().map(|&s| s.into()));
        let module = self.module(&found, new_key.collect(), span)?;
        let binding = alias.map_or(&last.text, |alias| &alias.text).clone();
        Some(Import { binding, span, module, item })
    }
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

/// Renames every module, then puts their statements in one file.
fn join(modules: Vec<Module>, diags: &mut Vec<Diagnostic>) -> ast::File {
    let (prelude, _) = parser::parse(lexer::lex(PRELUDE).0);
    let mut stmts = prelude.stmts;
    let mut exports: Vec<Names> = Vec::with_capacity(modules.len());
    let count = modules.len();
    let keys: Vec<String> = modules.iter().map(|module| module.key.join(".")).collect();
    for (id, mut module) in modules.into_iter().enumerate() {
        let library = id + 1 < count;
        if library {
            check_library(&module.stmts, diags);
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
        // A `use` of a package stays, and reports E0901 until packages exist.
        module.stmts.retain(|stmt| !matches!(stmt.kind, StmtKind::Use { relative: true, .. }));
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
