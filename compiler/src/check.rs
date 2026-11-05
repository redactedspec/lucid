//! Binds names, checks types, and lowers the syntax tree to MIR in one pass.
//! There are no modules or generics yet, so nothing needs separate stages.
//! List, map, and set tasks are built in until generics can express them.

use crate::ast::{
    self, BinaryOp, ExprKind as Ast, StmtKind, TextPart, TypeBody, TypeKind, UnaryOp,
};
use crate::diag::{Diagnostic, Span};
use crate::mir::{Expr, ExprKind, Local, Program, Stmt, Task, Ty, Update, inner};
use std::collections::{HashMap, HashSet};

/// Names of the spec's built-in types that this milestone cannot represent.
const LATER_TYPES: [&str; 10] =
    ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "pointer"];

/// Type names that a program cannot define for itself.
const BUILT_IN_TYPES: [&str; 8] = ["int", "float", "bool", "byte", "text", "list", "map", "set"];

/// The same limit as nesting in the parser, for a chain of aliases.
const MAX_ALIASES: usize = 128;

/// Dot calls that change their receiver, so they only work as statements.
const CHANGING_CALLS: [&str; 4] = ["push", "swap", "add", "remove"];

const JOIN_TEXT: &str = "join text with interpolation: \"{a}{b}\"";

/// Checks a file. A program using an unbuilt feature gets only E0901, because
/// names and types that feature would supply make any other error unreliable.
pub fn check(file: &ast::File) -> (Program, Vec<Diagnostic>) {
    let mut checker = Checker::default();
    checker.declare_types(file);
    checker.declare(file);
    let tasks = file
        .stmts
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Task(def) => def.body.as_ref().map(|body| (def, body, stmt.span)),
            _ => None,
        })
        .enumerate()
        .map(|(id, (def, body, span))| checker.task(id, def, body, span))
        .collect();
    let main = checker.main(file);

    let mut diags = checker.diags;
    if diags.iter().any(|d| d.code == "E0901") {
        diags.retain(|d| d.code == "E0901");
    }
    (Program { tasks, main }, diags)
}

struct Signature<'a> {
    params: Vec<Param<'a>>,
    returns: Ty,
}

struct Param<'a> {
    name: &'a str,
    ty: Ty,
    default: Option<Expr>,
}

/// A written type name.
#[derive(Clone, Copy)]
enum Named<'a> {
    Known(Ty),
    /// Replaced by `Known` once resolved.
    Alias(&'a ast::Type),
}

/// A `type`, which has one variant with its own name, or a `choice`.
struct Data<'a> {
    name: &'a ast::Name,
    ty: Ty,
    choice: bool,
    variants: Vec<Variant<'a>>,
}

struct Variant<'a> {
    name: &'a str,
    /// In written order, which is what a pattern binds.
    fields: Vec<Field<'a>>,
    /// The number of counted fields, which take the first slots.
    counted: usize,
}

struct Field<'a> {
    name: &'a str,
    ty: Ty,
    slot: usize,
    default: Option<Expr>,
}

/// A field as written: its name, type, default, and whether it has the plain
/// form. A variant's fields use the task parameter grammar, which allows more.
type WrittenField<'a> = (&'a ast::Name, Option<&'a ast::Type>, Option<&'a ast::Expr>, bool);

/// What the arms of a `match` have covered so far.
#[derive(Default)]
struct Covered {
    /// An arm matches anything.
    all: bool,
    variants: HashSet<usize>,
    literals: HashSet<String>,
}

/// What a task can see of a top-level `let` or `var`.
enum Top<'a> {
    Var,
    /// A `let` with a literal value, which is known before any statement runs.
    Literal(&'a ast::Expr),
    Let,
}

#[derive(Default)]
struct Checker<'a> {
    diags: Vec<Diagnostic>,
    task_ids: HashMap<&'a str, usize>,
    signatures: Vec<Signature<'a>>,
    types: HashMap<&'a str, Named<'a>>,
    datas: Vec<Data<'a>>,
    /// A type or variant name, with its data and variant index.
    constructors: HashMap<&'a str, (usize, usize)>,
    /// The aliases being resolved, innermost last.
    resolving: Vec<&'a str>,
    top: HashMap<&'a str, Top<'a>>,
    scopes: Vec<HashMap<&'a str, Local>>,
    locals: Vec<Ty>,
    mutable: Vec<bool>,
    /// The return type, or `None` at top level.
    returns: Option<Ty>,
    loops: usize,
}

fn poisoned(span: Span) -> Expr {
    Expr { kind: ExprKind::None, ty: Ty::Error, span }
}

fn is_literal(expr: &ast::Expr) -> bool {
    match &expr.kind {
        Ast::Int(_) | Ast::Float(_) | Ast::Bool(_) => true,
        Ast::Text(parts) => parts.iter().all(|part| matches!(part, TextPart::Literal(_))),
        _ => false,
    }
}

/// A place to write through, such as `x[i]` in `x[i][j] = 1`. `local` holds the
/// value, and `setup` fills it. `take` empties its slot in the container, so
/// that the local holds the only count and the write needs no copy. `finish`
/// stores it back. Nothing may read the container between `take` and `finish`.
struct Place {
    local: Local,
    setup: Vec<Stmt>,
    take: Vec<Stmt>,
    finish: Vec<Stmt>,
}

impl Place {
    /// The whole write. `write` holds the value that the write changes.
    fn stmts(mut self, write: Stmt) -> Stmt {
        self.setup.append(&mut self.take);
        self.setup.push(write);
        self.setup.append(&mut self.finish);
        Stmt::Block(self.setup)
    }
}

/// True if control never reaches the end of `stmts`.
fn ends(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|stmt| match stmt {
        Stmt::Return(_) | Stmt::Placeholder(_) => true,
        Stmt::Block(stmts) => ends(stmts),
        Stmt::If { branches, otherwise } => {
            branches.iter().all(|(_, then)| ends(then)) && ends(otherwise)
        }
        _ => false,
    })
}

fn want_name(want: Option<Ty>) -> String {
    want.map_or(String::new(), Ty::name)
}

fn compound(op: ast::AssignOp) -> Option<BinaryOp> {
    match op {
        ast::AssignOp::Set => None,
        ast::AssignOp::Add => Some(BinaryOp::Add),
        ast::AssignOp::Sub => Some(BinaryOp::Sub),
        ast::AssignOp::Mul => Some(BinaryOp::Mul),
        ast::AssignOp::Div => Some(BinaryOp::Div),
    }
}

fn symbol(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::FloorDiv => "//",
        BinaryOp::Rem => "%",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::And => "and",
        BinaryOp::Or => "or",
        _ => unreachable!("reported as unsupported first"),
    }
}

impl<'a> Checker<'a> {
    fn report(&mut self, diag: Diagnostic) -> Expr {
        let span = diag.span;
        self.diags.push(diag);
        poisoned(span)
    }

    fn fail(&mut self, code: &'static str, message: String, span: Span) -> Expr {
        self.report(Diagnostic::new(code, message, span))
    }

    fn unsupported(&mut self, what: &str, span: Span) -> Expr {
        let message = format!("{what} parses, but cannot run yet");
        let note = "see docs/roadmap.md for the milestone that adds it";
        self.report(Diagnostic::new("E0901", message, span).note(note))
    }

    fn later(&mut self, what: &str, span: Span) -> Option<Stmt> {
        self.unsupported(what, span);
        None
    }

    /// Reports E0303 unless `value` has type `want`.
    fn expect(&mut self, value: &Expr, want: Ty, what: &str) {
        if value.ty == want || value.ty == Ty::Error || want == Ty::Error {
            return;
        }
        let message = format!("{what} needs `{}`, found `{}`", want.name(), value.ty.name());
        let diag = Diagnostic::new("E0303", message, value.span);
        self.report(match (want, value.ty) {
            (Ty::Float, Ty::Int) => diag.note("conversions are explicit: use `.to_float()`"),
            (Ty::Int, Ty::Float) => diag.note("conversions are explicit: use `.to_int()`"),
            _ => diag,
        });
    }

    fn ty(&mut self, ty: &'a ast::Type) -> Ty {
        let name = match &ty.kind {
            TypeKind::None => return Ty::None,
            TypeKind::Named { path, args } => match (path.as_slice(), args.as_slice()) {
                ([name], args) if matches!(name.text.as_str(), "list" | "map" | "set") => {
                    return self.collection_type(&name.text, args, ty.span);
                }
                ([name], []) => name.text.as_str(),
                _ => "",
            },
            _ => "",
        };
        match name {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "bool" => Ty::Bool,
            "byte" => Ty::Byte,
            "text" => Ty::Text,
            "" => self.unsupported("this type", ty.span).ty,
            name if LATER_TYPES.contains(&name) => {
                self.unsupported(&format!("type `{name}`"), ty.span).ty
            }
            name => match self.types.get_key_value(name) {
                Some((_, &Named::Known(known))) => known,
                Some((&name, &Named::Alias(target))) => self.alias(name, target, ty.span),
                None => self.fail("E0302", format!("type `{name}` is not defined"), ty.span).ty,
            },
        }
    }

    fn alias(&mut self, name: &'a str, target: &'a ast::Type, span: Span) -> Ty {
        if self.resolving.contains(&name) {
            let message = format!("alias `{name}` is defined by itself");
            return self.fail("E0315", message, span).ty;
        }
        if self.resolving.len() == MAX_ALIASES {
            return self.fail("E0205", "aliases are nested too deep".into(), span).ty;
        }
        self.resolving.push(name);
        let ty = self.ty(target);
        self.resolving.pop();
        self.types.insert(name, Named::Known(ty));
        ty
    }

    fn collection_type(&mut self, name: &str, args: &'a [ast::Type], span: Span) -> Ty {
        let args: Vec<Ty> = args.iter().map(|arg| self.ty(arg)).collect();
        match (name, args.as_slice()) {
            (_, args) if args.contains(&Ty::Error) => Ty::Error,
            ("list", &[item]) => Ty::List(inner(item)),
            ("set", &[item]) => {
                self.check_key(item, span);
                Ty::Set(inner(item))
            }
            ("map", &[key, value]) => {
                self.check_key(key, span);
                Ty::Map(inner(key), inner(value))
            }
            _ => {
                let needs = if name == "map" { "two types" } else { "one type" };
                self.fail("E0303", format!("`{name}` needs {needs}"), span).ty
            }
        }
    }

    /// A collection needs the derived `Hash` trait, which is not built yet. A
    /// `float` never gets it, since NaN is not equal to itself.
    fn check_key(&mut self, ty: Ty, span: Span) {
        match ty {
            Ty::Error => {}
            _ if ty.is_compound() => {
                self.unsupported(&format!("`{}` as a key", ty.name()), span);
            }
            _ if ty.is_key() => {}
            _ => {
                let message = format!("`{}` cannot be a key", ty.name());
                self.fail("E0303", message, span);
            }
        }
    }

    /// Reports E0304 and returns false if a type or task has this name. A
    /// `type`, `choice`, variant, and task share one namespace, so that a
    /// variant reads the same in a pattern and in a call.
    fn define(&mut self, name: &'a ast::Name) -> bool {
        let text = name.text.as_str();
        let taken = self.types.contains_key(text)
            || self.constructors.contains_key(text)
            || self.task_ids.contains_key(text);
        if taken {
            self.fail("E0304", format!("`{text}` is already defined"), name.span);
        }
        !taken
    }

    /// Like `define`, for a name that a type or `set(...)` already means.
    fn define_data(&mut self, name: &'a ast::Name) -> bool {
        let text = name.text.as_str();
        if BUILT_IN_TYPES.contains(&text) || LATER_TYPES.contains(&text) {
            self.fail("E0304", format!("`{text}` is already defined"), name.span);
            return false;
        }
        self.define(name)
    }

    fn data_header(&mut self, public: bool, params: &[ast::TypeParam], span: Span) {
        if public {
            self.unsupported("`public`", span);
        }
        if let Some(first) = params.first() {
            self.unsupported("a generic type", first.name.span);
        }
    }

    fn new_data(&mut self, name: &'a ast::Name, choice: bool) -> usize {
        let id = self.datas.len();
        let ty = Ty::Data(name.text.clone().leak(), id);
        self.datas.push(Data { name, ty, choice, variants: Vec::new() });
        self.types.insert(&name.text, Named::Known(ty));
        id
    }

    /// Runs before anything that uses a type: fields may name any type, so
    /// all names go in first, then aliases resolve, then fields.
    fn declare_types(&mut self, file: &'a ast::File) {
        let mut aliases = Vec::new();
        let mut written: Vec<(usize, usize, Vec<WrittenField<'a>>)> = Vec::new();
        for stmt in &file.stmts {
            match &stmt.kind {
                StmtKind::Type(def) => {
                    self.data_header(def.public, &def.type_params, stmt.span);
                    if !self.define_data(&def.name) {
                        continue;
                    }
                    match &def.body {
                        TypeBody::Alias(target) => {
                            self.types.insert(&def.name.text, Named::Alias(target));
                            aliases.push((def.name.text.as_str(), target, def.name.span));
                        }
                        TypeBody::Fields(fields) => {
                            let id = self.new_data(&def.name, false);
                            let name = def.name.text.as_str();
                            self.datas[id].variants.push(Variant {
                                name,
                                fields: Vec::new(),
                                counted: 0,
                            });
                            self.constructors.insert(name, (id, 0));
                            let fields = fields
                                .iter()
                                .map(|f| (&f.name, Some(&f.ty), f.default.as_ref(), true))
                                .collect();
                            written.push((id, 0, fields));
                        }
                    }
                }
                StmtKind::Choice(def) => {
                    self.data_header(def.public, &def.type_params, stmt.span);
                    if !self.define_data(&def.name) {
                        continue;
                    }
                    let id = self.new_data(&def.name, true);
                    for variant in &def.variants {
                        let name = variant.name.text.as_str();
                        let index = self.datas[id].variants.len();
                        if self.define_data(&variant.name) {
                            self.constructors.insert(name, (id, index));
                        }
                        self.datas[id].variants.push(Variant {
                            name,
                            fields: Vec::new(),
                            counted: 0,
                        });
                        let fields = variant
                            .params
                            .iter()
                            .map(|p| {
                                let plain = p.mode == ast::ParamMode::Read && p.name.text != "self";
                                (&p.name, p.ty.as_ref(), p.default.as_ref(), plain)
                            })
                            .collect();
                        written.push((id, index, fields));
                    }
                }
                _ => {}
            }
        }
        for (name, target, span) in aliases {
            if let Some(Named::Alias(_)) = self.types.get(name) {
                self.alias(name, target, span);
            }
        }
        for (id, variant, fields) in written {
            let (fields, counted) = self.fields(fields);
            let variant = &mut self.datas[id].variants[variant];
            variant.fields = fields;
            variant.counted = counted;
        }
        self.check_finite();
    }

    /// Counted fields take the first slots, so that a record needs only their
    /// number to know what it holds.
    fn fields(&mut self, written: Vec<WrittenField<'a>>) -> (Vec<Field<'a>>, usize) {
        let mut fields: Vec<Field<'a>> = Vec::with_capacity(written.len());
        for (name, ty, default, plain) in written {
            if !plain {
                let message = "a field cannot be `self`, `change`, or `take`";
                self.fail("E0307", message.into(), name.span);
            }
            let ty = ty.map_or(Ty::Error, |ty| self.ty(ty));
            if fields.iter().any(|field| field.name == name.text) {
                self.fail("E0304", format!("`{}` is already defined", name.text), name.span);
            }
            let default = default.map(|value| self.default_value(value, ty));
            fields.push(Field { name: &name.text, ty, slot: 0, default });
        }
        let counted = fields.iter().filter(|field| field.ty.is_counted()).count();
        let (mut held, mut plain) = (0, counted);
        for field in &mut fields {
            let next = if field.ty.is_counted() { &mut held } else { &mut plain };
            field.slot = *next;
            *next += 1;
        }
        (fields, counted)
    }

    /// Reports a type with no finite value. Recursion must pass through a
    /// variant that can avoid it, or a collection that can be empty.
    fn check_finite(&mut self) {
        let mut finite = vec![false; self.datas.len()];
        let holds = |ty: Ty, finite: &[bool]| !matches!(ty, Ty::Data(_, id) if !finite[id]);
        let mut changed = true;
        while changed {
            changed = false;
            for (id, data) in self.datas.iter().enumerate() {
                let reachable = data
                    .variants
                    .iter()
                    .any(|variant| variant.fields.iter().all(|f| holds(f.ty, &finite)));
                if reachable && !finite[id] {
                    finite[id] = true;
                    changed = true;
                }
            }
        }
        for id in (0..finite.len()).filter(|&id| !finite[id]) {
            let name = self.datas[id].name;
            let message = format!("`{}` has no finite value", name.text);
            let note = "building it needs a type that holds itself: add a variant with no such \
                        field, or hold the value in a list";
            self.report(Diagnostic::new("E0315", message, name.span).note(note));
        }
    }

    /// First pass: tasks and top-level values may be used before their line.
    fn declare(&mut self, file: &'a ast::File) {
        for stmt in &file.stmts {
            let StmtKind::Task(def) = &stmt.kind else { continue };
            if def.body.is_none() {
                self.unsupported("`unsafe extern task`", stmt.span);
                continue;
            }
            if def.public {
                self.unsupported("`public`", stmt.span);
            }
            let name = def.sig.name.text.as_str();
            if self.define(&def.sig.name) {
                self.task_ids.insert(name, self.signatures.len());
            }
            let signature = self.signature(&def.sig);
            self.signatures.push(signature);
        }
        for stmt in &file.stmts {
            if let StmtKind::Let { mutable, name, value, .. } = &stmt.kind {
                let top = match (*mutable, is_literal(value)) {
                    (true, _) => Top::Var,
                    (false, true) => Top::Literal(value),
                    (false, false) => Top::Let,
                };
                self.top.insert(&name.text, top);
            }
        }
    }

    fn signature(&mut self, sig: &'a ast::TaskSig) -> Signature<'a> {
        if let Some(first) = sig.type_params.first() {
            self.unsupported("a generic task", first.name.span);
        }
        let mut params: Vec<Param<'a>> = Vec::new();
        for param in &sig.params {
            let span = param.name.span;
            if param.mode != ast::ParamMode::Read {
                self.unsupported("a `change` or `take` parameter", span);
            }
            let ty = param.ty.as_ref().map_or(Ty::Error, |ty| self.ty(ty));
            if param.name.text == "self" {
                self.unsupported("a `self` parameter", span);
            } else if params.iter().any(|p| p.name == param.name.text) {
                self.fail("E0304", format!("`{}` is already defined", param.name.text), span);
            }
            let default = param.default.as_ref().map(|value| self.default_value(value, ty));
            if default.is_none() && params.last().is_some_and(|p| p.default.is_some()) {
                let message =
                    format!("`{}` needs a default, like the one before it", param.name.text);
                self.fail("E0307", message, span);
            }
            params.push(Param { name: &param.name.text, ty, default });
        }
        let returns = sig.returns.as_ref().map_or(Ty::None, |ty| self.ty(ty));
        Signature { params, returns }
    }

    fn default_value(&mut self, value: &'a ast::Expr, ty: Ty) -> Expr {
        if !is_literal(value) {
            return self.fail("E0313", "a default must be a literal".into(), value.span);
        }
        let value = self.expr(value);
        self.expect(&value, ty, "this default");
        value
    }

    fn task(
        &mut self,
        id: usize,
        def: &'a ast::TaskDef,
        body: &'a [ast::Stmt],
        span: Span,
    ) -> Task {
        let returns = self.signatures[id].returns;
        self.returns = Some(returns);
        let params: Vec<(&'a str, Ty)> =
            self.signatures[id].params.iter().map(|p| (p.name, p.ty)).collect();
        let mut scope = HashMap::new();
        for &(name, ty) in &params {
            scope.insert(name, self.new_local(ty, false));
        }
        self.scopes.push(scope);
        let body = self.stmts(body);
        self.scopes.pop();
        if returns != Ty::None && returns != Ty::Error && !ends(&body) {
            let name = &def.sig.name;
            let message = format!("`{}` must return `{}` on every path", name.text, returns.name());
            self.fail("E0309", message, name.span);
        }
        self.mutable.clear();
        Task { params: params.len(), returns, locals: std::mem::take(&mut self.locals), body, span }
    }

    fn main(&mut self, file: &'a ast::File) -> Task {
        self.returns = None;
        self.scopes.push(HashMap::new());
        let body = self.stmts(&file.stmts);
        let span = Span { start: 0, end: 0 };
        Task { params: 0, returns: Ty::None, locals: std::mem::take(&mut self.locals), body, span }
    }

    fn new_local(&mut self, ty: Ty, mutable: bool) -> Local {
        self.locals.push(ty);
        self.mutable.push(mutable);
        self.locals.len() - 1
    }

    /// Adds a name to the innermost scope. Shadowing there is an error.
    fn bind(&mut self, name: &'a ast::Name, ty: Ty, mutable: bool) -> Local {
        let text = name.text.as_str();
        let top_level = self.returns.is_none() && self.scopes.len() == 1;
        let scope = self.scopes.last().expect("a scope is open");
        let global = self.task_ids.contains_key(text) || self.constructors.contains_key(text);
        if scope.contains_key(text) || (top_level && global) {
            self.fail("E0304", format!("`{text}` is already defined"), name.span);
        }
        let local = self.new_local(ty, mutable);
        self.scopes.last_mut().expect("a scope is open").insert(text, local);
        local
    }

    fn lookup(&self, name: &str) -> Option<Local> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name).copied())
    }

    fn block(&mut self, stmts: &'a [ast::Stmt]) -> Vec<Stmt> {
        self.scopes.push(HashMap::new());
        let stmts = self.stmts(stmts);
        self.scopes.pop();
        stmts
    }

    fn stmts(&mut self, stmts: &'a [ast::Stmt]) -> Vec<Stmt> {
        stmts.iter().filter_map(|stmt| self.stmt(stmt)).collect()
    }

    fn condition(&mut self, value: &'a ast::Expr, what: &str) -> Expr {
        let value = self.expr(value);
        self.expect(&value, Ty::Bool, what);
        value
    }

    fn stmt(&mut self, stmt: &'a ast::Stmt) -> Option<Stmt> {
        let span = stmt.span;
        Some(match &stmt.kind {
            StmtKind::Task(_) => {
                if self.returns.is_some() || self.scopes.len() > 1 {
                    self.unsupported("a nested task", span);
                }
                return None;
            }
            StmtKind::Let { mutable, name, ty, value } => {
                let want = ty.as_ref().map(|ty| self.ty(ty));
                if let Ast::Match { subject, arms } = &value.kind {
                    let (matched, result) = self.matching(subject, arms, Some(want));
                    let ty = self.locals[result];
                    let read = self.read(result, value.span);
                    let mut stmts = vec![matched, Stmt::Set(self.bind(name, ty, *mutable), read)];
                    if ty.is_counted() {
                        stmts.push(Stmt::Clear(result));
                    }
                    return Some(Stmt::Block(stmts));
                }
                let value = self.expected(value, want);
                let ty = want.unwrap_or(value.ty);
                self.expect(&value, ty, "this annotation");
                Stmt::Set(self.bind(name, ty, *mutable), value)
            }
            StmtKind::Assign { target, op, value } => return self.assign(target, *op, value, span),
            StmtKind::If { branches, otherwise } => {
                let branches = branches
                    .iter()
                    .map(|(condition, then)| (self.condition(condition, "`if`"), self.block(then)))
                    .collect();
                let otherwise = otherwise.as_ref().map_or(Vec::new(), |body| self.block(body));
                Stmt::If { branches, otherwise }
            }
            StmtKind::While { condition, body } => {
                let condition = self.condition(condition, "`while`");
                self.loops += 1;
                let body = self.block(body);
                self.loops -= 1;
                Stmt::While { condition, body }
            }
            StmtKind::For { change, name, iterable, body } => {
                self.for_loop(*change, name, iterable, body)
            }
            StmtKind::Return(value) => {
                let none = Expr { kind: ExprKind::None, ty: Ty::None, span };
                let mut matched = None;
                let value = match value {
                    Some(ast::Expr { kind: Ast::Match { subject, arms }, span }) => {
                        let (stmt, result) = self.matching(subject, arms, Some(self.returns));
                        matched = Some(stmt);
                        self.read(result, *span)
                    }
                    Some(value) => self.expected(value, self.returns),
                    None => none,
                };
                match self.returns {
                    Some(returns) => self.expect(&value, returns, "this `return`"),
                    None => {
                        self.fail("E0310", "`return` is only allowed in a task".into(), span);
                    }
                }
                match matched {
                    Some(matched) => Stmt::Block(vec![matched, Stmt::Return(value)]),
                    None => Stmt::Return(value),
                }
            }
            StmtKind::Print(value) => match &value.kind {
                Ast::Text(parts) => match parts.as_slice() {
                    [] => Stmt::PrintText(String::new()),
                    [TextPart::Literal(text)] => Stmt::PrintText(text.clone()),
                    _ => Stmt::Print(self.expr(value)),
                },
                _ => {
                    let value = self.expr(value);
                    Stmt::Print(self.showable(value))
                }
            },
            StmtKind::Break | StmtKind::Continue if self.loops == 0 => {
                self.fail("E0311", "`break` and `continue` need a loop".into(), span);
                return None;
            }
            StmtKind::Break => Stmt::Break,
            StmtKind::Continue => Stmt::Continue,
            StmtKind::Placeholder => Stmt::Placeholder(span),
            StmtKind::Expr(value) => return self.expr_stmt(value, span),
            StmtKind::Use { .. } => return self.later("`use`", span),
            StmtKind::Test { .. } => return self.later("`test`", span),
            StmtKind::Type(_) | StmtKind::Choice(_) => return None,
            StmtKind::Trait(_) => return self.later("`trait`", span),
            StmtKind::Expect(_) => return self.later("`expect`", span),
            StmtKind::Unsafe(_) => return self.later("`unsafe`", span),
        })
    }

    fn expr_stmt(&mut self, value: &'a ast::Expr, span: Span) -> Option<Stmt> {
        Some(match &value.kind {
            Ast::Match { subject, arms } => self.matching(subject, arms, None).0,
            Ast::Call { callee, args } => match &callee.kind {
                Ast::Field { base, name }
                    if CHANGING_CALLS.contains(&name.text.as_str()) && self.is_collection(base) =>
                {
                    return self.changing_call(base, name, args, span);
                }
                _ => Stmt::Expr(self.expr(value)),
            },
            _ => Stmt::Expr(self.expr(value)),
        })
    }

    /// Lowers a `match` to an `if` chain over a hidden local, which keeps the
    /// value fixed if an arm assigns the variable it came from. The checks
    /// below prove that the last arm is all that can remain, so it needs no
    /// test, which is also how `ends` sees that every path is covered.
    ///
    /// ponytail: tests run one by one, and a text pattern allocates. A jump
    /// table and static text can follow if M4 benchmarks ask for them.
    ///
    /// `value` is `None` for a statement, or else holds the type that the
    /// context needs, if known. Returns the local that receives the value.
    fn matching(
        &mut self,
        subject: &'a ast::Expr,
        arms: &'a [ast::Arm],
        value: Option<Option<Ty>>,
    ) -> (Stmt, Local) {
        let subject = self.expr(subject);
        let ty = subject.ty;
        let holder = self.new_local(ty, false);
        let result = self.new_local(Ty::None, false);
        let mut want = value.flatten();
        let mut covered = Covered::default();
        let mut lowered = Vec::with_capacity(arms.len());
        for arm in arms {
            if self.missing(ty, &covered).is_empty() {
                let message = "this arm can never run: earlier arms cover everything";
                self.fail("E0317", message.into(), arm.pattern.span);
            }
            self.scopes.push(HashMap::new());
            let (test, mut body) = self.pattern(&arm.pattern, holder, ty, &mut covered);
            match &arm.body {
                ast::ArmBody::Expr(arm_value) if value.is_some() => {
                    let lowered = self.expected(arm_value, want);
                    let wanted = *want.get_or_insert(lowered.ty);
                    self.expect(&lowered, wanted, "this arm");
                    body.push(Stmt::Set(result, lowered));
                }
                ast::ArmBody::Expr(arm_value) => {
                    body.extend(self.expr_stmt(arm_value, arm_value.span));
                }
                ast::ArmBody::Stmt(stmt) => {
                    if value.is_some() && matches!(stmt.kind, StmtKind::Print(_)) {
                        let message = "this `match` needs a value, and `print` gives none";
                        self.fail("E0303", message.into(), stmt.span);
                    }
                    body.extend(self.stmt(stmt));
                }
                ast::ArmBody::Block(stmts) => body.extend(self.stmts(stmts)),
            }
            self.scopes.pop();
            lowered.push((test, body));
        }
        let missing = self.missing(ty, &covered);
        if !missing.is_empty() && ty != Ty::Error {
            let (message, note) = (
                format!("this `match` does not cover every `{}`", ty.name()),
                format!("add an arm for {}", missing.join(", ")),
            );
            let span = arms.last().map_or(subject.span, |arm| arm.pattern.span);
            self.report(Diagnostic::new("E0316", message, span).note(note));
        }
        self.locals[result] = want.unwrap_or(Ty::None);

        let otherwise = lowered.pop().map_or(Vec::new(), |(_, body)| body);
        let span = subject.span;
        let always = || Expr { kind: ExprKind::Bool(true), ty: Ty::Bool, span };
        let branches =
            lowered.into_iter().map(|(test, body)| (test.unwrap_or_else(always), body)).collect();
        let mut stmts = vec![Stmt::Set(holder, subject), Stmt::If { branches, otherwise }];
        if ty.is_counted() {
            stmts.push(Stmt::Clear(holder));
        }
        (Stmt::Block(stmts), result)
    }

    /// What no arm has covered, in the words of a note. Empty if nothing is left.
    fn missing(&self, ty: Ty, covered: &Covered) -> Vec<String> {
        if covered.all {
            return Vec::new();
        }
        match self.choice_of(ty) {
            Some(id) => (self.datas[id].variants.iter().enumerate())
                .filter(|(index, _)| !covered.variants.contains(index))
                .map(|(_, variant)| format!("`{}`", variant.name))
                .collect(),
            None if ty == Ty::Bool => ["true", "false"]
                .into_iter()
                .filter(|literal| !covered.literals.contains(*literal))
                .map(|literal| format!("`{literal}`"))
                .collect(),
            None => vec!["`_`".into()],
        }
    }

    /// The `choice` that a value of this type belongs to, if any.
    fn choice_of(&self, ty: Ty) -> Option<usize> {
        match ty {
            Ty::Data(_, id) if self.datas[id].choice => Some(id),
            _ => None,
        }
    }

    /// Finds a variant of a choice, or reports that the name is wrong. A
    /// misspelled variant must not become a name that catches everything.
    fn variant_index(&mut self, id: usize, name: &ast::Name) -> Option<usize> {
        let data = &self.datas[id];
        let found = data.variants.iter().position(|variant| variant.name == name.text);
        if found.is_none() {
            let message = format!("`{}` has no variant `{}`", data.name.text, name.text);
            let names: Vec<&str> = data.variants.iter().map(|variant| variant.name).collect();
            let note = format!("its variants are {}, and `_` matches the rest", names.join(", "));
            self.report(Diagnostic::new("E0318", message, name.span).note(note));
        }
        found
    }

    /// Lowers a pattern. Returns its test, or `None` if anything matches, and
    /// the statements that bind its names.
    fn pattern(
        &mut self,
        pattern: &'a ast::Pattern,
        holder: Local,
        ty: Ty,
        covered: &mut Covered,
    ) -> (Option<Expr>, Vec<Stmt>) {
        let span = pattern.span;
        match &pattern.kind {
            ast::PatternKind::Wildcard => {
                covered.all = true;
                (None, Vec::new())
            }
            ast::PatternKind::None => {
                self.unsupported("a `none` pattern", span);
                (None, Vec::new())
            }
            ast::PatternKind::Typed { .. } => {
                self.unsupported("a typed pattern", span);
                (None, Vec::new())
            }
            ast::PatternKind::Literal(value) => (self.literal(value, holder, ty, covered), vec![]),
            ast::PatternKind::Name(name) if self.choice_of(ty).is_some() => {
                self.variant_pattern(name, None, holder, ty, covered)
            }
            ast::PatternKind::Name(name) => {
                covered.all = true;
                self.check_binding(name);
                let local = self.bind(name, ty, false);
                (None, vec![Stmt::Set(local, self.read(holder, span))])
            }
            ast::PatternKind::Variant { name, bindings } => {
                self.variant_pattern(name, Some(bindings), holder, ty, covered)
            }
        }
    }

    /// A variant or type name in a binding would read as a test, and catch
    /// everything. See decision 52.
    fn check_binding(&mut self, name: &ast::Name) {
        if self.constructors.contains_key(name.text.as_str()) {
            let message = format!("`{}` cannot be bound here", name.text);
            let note = "it names a type or variant, and patterns do not nest, so use another name";
            self.report(Diagnostic::new("E0318", message, name.span).note(note));
        }
    }

    fn literal(
        &mut self,
        value: &'a ast::Expr,
        holder: Local,
        ty: Ty,
        covered: &mut Covered,
    ) -> Option<Expr> {
        let span = value.span;
        if !is_literal(value) {
            self.fail("E0318", "a text pattern cannot hold `{}`".into(), span);
            return None;
        }
        let value = self.expr(value);
        self.expect(&value, ty, "this pattern");
        if value.ty != ty {
            return None;
        }
        let key = match &value.kind {
            ExprKind::Int(number) => number.to_string(),
            ExprKind::Float(number) => number.to_bits().to_string(),
            ExprKind::Bool(flag) => flag.to_string(),
            ExprKind::Text(text) => text.clone(),
            _ => unreachable!("`is_literal` allows only these"),
        };
        if !covered.literals.insert(key) && !self.missing(ty, covered).is_empty() {
            self.fail("E0317", "this arm can never run: an earlier arm has it".into(), span);
        }
        Some(self.binary_values(BinaryOp::Eq, self.read(holder, span), value, span))
    }

    /// `bindings` is `None` if the name stands alone. Bindings take the fields
    /// in written order, and `_` skips one.
    fn variant_pattern(
        &mut self,
        name: &'a ast::Name,
        bindings: Option<&'a Vec<ast::Name>>,
        holder: Local,
        ty: Ty,
        covered: &mut Covered,
    ) -> (Option<Expr>, Vec<Stmt>) {
        let span = name.span;
        let mut bound = Vec::new();
        let parentheses = bindings.is_some();
        let bindings = bindings.map_or(&[][..], Vec::as_slice);
        let variant = match self.choice_of(ty) {
            Some(id) => self.variant_index(id, name).map(|variant| (id, variant)),
            None => {
                if ty != Ty::Error {
                    let message = format!("`{}` is not a `choice`", ty.name());
                    self.fail("E0318", message, span);
                }
                None
            }
        };
        let Some((id, variant)) = variant else {
            for binding in bindings.iter().filter(|binding| binding.text != "_") {
                self.bind(binding, Ty::Error, false);
            }
            return (None, bound);
        };
        if !covered.variants.insert(variant) && !self.missing(ty, covered).is_empty() {
            let message = format!("this arm can never run: an earlier arm has `{}`", name.text);
            self.fail("E0317", message, span);
        }
        let fields = self.datas[id].variants[variant].fields.len();
        if fields == 0 && parentheses {
            let message = format!("`{}` has no fields", name.text);
            let note = format!("write `{}`, with no `()`", name.text);
            self.report(Diagnostic::new("E0318", message, span).note(note));
        } else if bindings.len() != fields {
            let plural = if fields == 1 { "" } else { "s" };
            let message = format!("`{}` has {fields} field{plural}", name.text);
            let note = format!("bind all {fields} in order, and use `_` to skip one");
            self.report(Diagnostic::new("E0318", message, span).note(note));
        }
        for (index, binding) in bindings.iter().enumerate() {
            let field = self.datas[id].variants[variant].fields.get(index);
            let (field_ty, slot) = field.map_or((Ty::Error, 0), |field| (field.ty, field.slot));
            if binding.text == "_" {
                continue;
            }
            self.check_binding(binding);
            let local = self.bind(binding, field_ty, false);
            let from = self.read(holder, span).into();
            let kind = ExprKind::Field(from, slot);
            bound.push(Stmt::Set(local, Expr { kind, ty: field_ty, span }));
        }
        let int = |kind| Box::new(Expr { kind, ty: Ty::Int, span });
        let tag = int(ExprKind::Tag(self.read(holder, span).into()));
        let kind = ExprKind::Binary(BinaryOp::Eq, tag, int(ExprKind::Int(variant as i64)));
        (Some(Expr { kind, ty: Ty::Bool, span }), bound)
    }

    fn assign(
        &mut self,
        target: &'a ast::Expr,
        op: ast::AssignOp,
        value: &'a ast::Expr,
        span: Span,
    ) -> Option<Stmt> {
        let name = match &target.kind {
            Ast::Name(name) => name,
            Ast::Index { base, indices } => {
                return self.assign_index(base, indices, op, value, span);
            }
            Ast::Field { base, name } => return self.assign_field(base, name, op, value, span),
            _ => unreachable!("the parser allows only a name, field, or index"),
        };
        let Some(local) = self.variable(name, target.span) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let ty = self.locals[local];
        let value = match compound(op) {
            Some(op) => self.binary(op, target, value, span),
            None => self.expected(value, Some(ty)),
        };
        self.expect(&value, ty, "this assignment");
        Some(Stmt::Set(local, value))
    }

    /// The local of a name that can change. Reports E0305 if it cannot, and
    /// returns `None` unless the name is a local at all.
    fn variable(&mut self, name: &str, span: Span) -> Option<Local> {
        if self.lookup(name).is_none() && self.task_ids.contains_key(name) {
            self.fail("E0305", format!("`{name}` cannot change"), span);
            return None;
        }
        let place = self.name(name, span);
        if place.ty == Ty::Error {
            return None;
        }
        let ExprKind::Local(local) = place.kind else {
            self.fail("E0305", format!("`{name}` cannot change"), span);
            return None;
        };
        if !self.mutable[local] {
            let diag = Diagnostic::new("E0305", format!("`{name}` cannot change"), span);
            self.report(diag.note("only a name declared with `var` can change"));
        }
        Some(local)
    }

    fn read(&self, local: Local, span: Span) -> Expr {
        Expr { kind: ExprKind::Local(local), ty: self.locals[local], span }
    }

    /// Finds what a write changes. Each index runs once, before the value
    /// that is written, so that the order is the order on the page.
    fn place(&mut self, target: &'a ast::Expr) -> Option<Place> {
        match &target.kind {
            Ast::Name(name) => {
                let local = self.variable(name, target.span)?;
                Some(Place { local, setup: Vec::new(), take: Vec::new(), finish: Vec::new() })
            }
            Ast::Index { base, indices } => {
                let mut place = self.place(base)?;
                let (key, item) = self.key(self.locals[place.local], indices, target.span)?;
                let key = self.hoist(key, &mut place);
                let read = self.element(place.local, key, item, target.span);
                let key = self.read(key, target.span);
                Some(self.step(place, key, read))
            }
            Ast::Field { base, name } => {
                let mut place = self.place(base)?;
                let (slot, ty) = self.field(self.locals[place.local], name)?;
                let span = target.span;
                let read = self.slot_of(place.local, slot, ty, span);
                let key = Expr { kind: ExprKind::Int(slot as i64), ty: Ty::Int, span };
                place = self.step(place, key, read);
                Some(place)
            }
            _ => {
                let diag = Diagnostic::new("E0305", "this value cannot change", target.span);
                self.report(diag.note("only a name declared with `var` can change"));
                None
            }
        }
    }

    /// Goes one level deeper: `read` is the element at `key` of `place`.
    fn step(&mut self, mut place: Place, key: Expr, read: Expr) -> Place {
        let (span, item) = (read.span, read.ty);
        let value = self.new_local(item, true);
        place.setup.push(Stmt::Set(value, read));
        let empty = Expr { kind: ExprKind::None, ty: Ty::None, span };
        place.take.push(self.write(place.local, key.clone(), empty, span));
        let stored = self.read(value, span);
        let write = self.write(place.local, key, stored, span);
        place.finish.insert(0, write);
        place.finish.push(Stmt::Clear(value));
        place.local = value;
        place
    }

    /// Moves a value to a hidden local, so that it can be read twice or read
    /// before `take`. The local lets go of it when the write ends.
    fn hoist(&mut self, value: Expr, place: &mut Place) -> Local {
        let local = self.new_local(value.ty, false);
        if value.ty.is_counted() {
            place.finish.push(Stmt::Clear(local));
        }
        place.setup.push(Stmt::Set(local, value));
        local
    }

    /// Reads an element from the container in `local`.
    fn element(&self, local: Local, key: Local, ty: Ty, span: Span) -> Expr {
        let (container, key) = (self.read(local, span).into(), self.read(key, span).into());
        let kind = match self.locals[local] {
            Ty::List(_) => ExprKind::ListGet(container, key),
            _ => ExprKind::MapGet(container, key),
        };
        Expr { kind, ty, span }
    }

    fn write(&self, local: Local, key: Expr, value: Expr, span: Span) -> Stmt {
        let op = match self.locals[local] {
            Ty::List(_) => Update::ListSet,
            Ty::Data(..) => Update::FieldSet,
            _ => Update::MapSet,
        };
        Stmt::Update { local, op, args: vec![key, value], span }
    }

    fn assign_index(
        &mut self,
        base: &'a ast::Expr,
        indices: &'a [ast::Expr],
        op: ast::AssignOp,
        value: &'a ast::Expr,
        span: Span,
    ) -> Option<Stmt> {
        let Some(mut place) = self.place(base) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let local = place.local;
        let Some((key, item)) = self.key(self.locals[local], indices, span) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let key = self.hoist(key, &mut place);
        let value = match compound(op) {
            Some(op) => {
                let old = self.element(local, key, item, span);
                let value = self.expr(value);
                self.binary_values(op, old, value, span)
            }
            None => self.expected(value, Some(item)),
        };
        self.expect(&value, item, "this assignment");
        let value = self.ready(value, &mut place);
        let write = self.write(local, self.read(key, span), value, span);
        Some(place.stmts(write))
    }

    fn assign_field(
        &mut self,
        base: &'a ast::Expr,
        name: &'a ast::Name,
        op: ast::AssignOp,
        value: &'a ast::Expr,
        span: Span,
    ) -> Option<Stmt> {
        let Some(mut place) = self.place(base) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let local = place.local;
        let Some((slot, ty)) = self.field(self.locals[local], name) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let value = match compound(op) {
            Some(op) => {
                let old = self.slot_of(local, slot, ty, span);
                let value = self.expr(value);
                self.binary_values(op, old, value, span)
            }
            None => self.expected(value, Some(ty)),
        };
        self.expect(&value, ty, "this assignment");
        let value = self.ready(value, &mut place);
        let key = Expr { kind: ExprKind::Int(slot as i64), ty: Ty::Int, span };
        let write = self.write(local, key, value, span);
        Some(place.stmts(write))
    }

    fn slot_of(&self, local: Local, slot: usize, ty: Ty, span: Span) -> Expr {
        Expr { kind: ExprKind::Field(self.read(local, span).into(), slot), ty, span }
    }

    /// Finds a field of a `type`. Returns its slot and type.
    fn field(&mut self, ty: Ty, name: &ast::Name) -> Option<(usize, Ty)> {
        let id = match ty {
            Ty::Error => return None,
            Ty::Data(_, id) if !self.datas[id].choice => id,
            Ty::Data(..) => {
                let message = format!("`{}` is a `choice`, so it has no fields", ty.name());
                let diag = Diagnostic::new("E0303", message, name.span);
                self.report(diag.note("use `match` to reach the fields of a variant"));
                return None;
            }
            _ => usize::MAX,
        };
        let fields = self.datas.get(id).map_or(&[][..], |data| &data.variants[0].fields);
        let found = fields.iter().find(|f| f.name == name.text).map(|f| (f.slot, f.ty));
        if found.is_none() {
            let message = format!("`{}` has no field `{}`", ty.name(), name.text);
            self.fail("E0303", message, name.span);
        }
        found
    }

    /// Builds a `type` or a variant. Arguments are always named, and run in
    /// the order written, so any order keeps the order on the page.
    fn construct(
        &mut self,
        (id, variant): (usize, usize),
        args: &'a [ast::Arg],
        span: Span,
    ) -> Expr {
        let name = self.datas[id].variants[variant].name;
        let count = self.datas[id].variants[variant].fields.len();
        let mut given = vec![false; count];
        let mut fields = Vec::with_capacity(count);
        for arg in args {
            let info = &self.datas[id].variants[variant];
            let found = arg
                .name
                .as_ref()
                .and_then(|label| info.fields.iter().position(|field| field.name == label.text));
            let want = found.map_or(Ty::Error, |index| info.fields[index].ty);
            let value = self.expected(&arg.value, Some(want));
            let span = arg.value.span;
            let Some(label) = &arg.name else {
                let diag =
                    Diagnostic::new("E0307", format!("`{name}` needs named arguments"), span);
                self.report(diag.note("write `field: value`"));
                continue;
            };
            let Some(index) = found else {
                self.fail("E0307", format!("`{name}` has no field `{}`", label.text), label.span);
                continue;
            };
            if given[index] {
                let message = format!("`{}` is given twice", label.text);
                self.fail("E0307", message, label.span);
                continue;
            }
            given[index] = true;
            self.expect(&value, want, "this field");
            fields.push((self.datas[id].variants[variant].fields[index].slot, value));
        }
        for index in (0..count).filter(|&index| !given[index]) {
            let field = &self.datas[id].variants[variant].fields[index];
            match field.default.clone() {
                Some(value) => fields.push((field.slot, value)),
                None => {
                    let message = format!("`{name}` is missing field `{}`", field.name);
                    self.fail("E0307", message, span);
                }
            }
        }
        let ty = self.datas[id].ty;
        let counted = self.datas[id].variants[variant].counted;
        Expr { kind: ExprKind::New { tag: variant, counted, fields }, ty, span }
    }

    /// Reads a value now if the write takes a slot that it may read.
    fn ready(&mut self, value: Expr, place: &mut Place) -> Expr {
        if place.take.is_empty() {
            return value;
        }
        let span = value.span;
        let value = self.hoist(value, place);
        self.read(value, span)
    }

    /// True if `value` has a list, map, or set type. Errors stay for the real
    /// lowering to report.
    fn is_collection(&mut self, value: &'a ast::Expr) -> bool {
        let reported = self.diags.len();
        let collection = self.expr(value).ty.is_collection();
        self.diags.truncate(reported);
        collection
    }

    fn changing_call(
        &mut self,
        base: &'a ast::Expr,
        name: &'a ast::Name,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Option<Stmt> {
        let mut place = self.place(base)?;
        let local = place.local;
        let ty = self.locals[local];
        let (op, wants): (Update, &[Ty]) = match (ty, name.text.as_str()) {
            (Ty::List(item), "push") => (Update::ListPush, std::slice::from_ref(item)),
            (Ty::List(_), "swap") => (Update::ListSwap, &[Ty::Int, Ty::Int]),
            (Ty::Set(item), "add") => (Update::MapSet, std::slice::from_ref(item)),
            (Ty::Set(key) | Ty::Map(key, _), "remove") => {
                (Update::MapRemove, std::slice::from_ref(key))
            }
            (ty, _) => {
                let message = format!("`{}` does not work on `{}`", name.text, ty.name());
                self.fail("E0303", message, span);
                return None;
            }
        };
        if args.len() != wants.len() || args.iter().any(|arg| arg.change || arg.name.is_some()) {
            let (count, plural) = (wants.len(), if wants.len() == 1 { "" } else { "s" });
            let message = format!("`{}` takes {count} positional argument{plural}", name.text);
            self.fail("E0307", message, span);
            return None;
        }
        let mut values = Vec::with_capacity(2);
        for (arg, &want) in args.iter().zip(wants) {
            let value = self.expected(&arg.value, Some(want));
            self.expect(&value, want, "this argument");
            values.push(self.ready(value, &mut place));
        }
        if name.text == "add" {
            values.push(Expr { kind: ExprKind::None, ty: Ty::None, span });
        }
        Some(place.stmts(Stmt::Update { local, op, args: values, span }))
    }

    fn range(&mut self, start: &'a ast::Expr, end: &'a ast::Expr) -> (Expr, Expr) {
        let start = self.expr(start);
        let end = self.expr(end);
        self.expect(&start, Ty::Int, "a range");
        self.expect(&end, Ty::Int, "a range");
        (start, end)
    }

    /// Checks a loop body, in a new scope that holds the loop name.
    fn loop_body(
        &mut self,
        name: &'a ast::Name,
        ty: Ty,
        body: &'a [ast::Stmt],
    ) -> (Local, Vec<Stmt>) {
        self.scopes.push(HashMap::new());
        let local = self.bind(name, ty, false);
        self.loops += 1;
        let body = self.stmts(body);
        self.loops -= 1;
        self.scopes.pop();
        (local, body)
    }

    fn for_loop(
        &mut self,
        change: bool,
        name: &'a ast::Name,
        iterable: &'a ast::Expr,
        body: &'a [ast::Stmt],
    ) -> Stmt {
        let Ast::Binary(op @ (BinaryOp::Range | BinaryOp::RangeInclusive), start, end) =
            &iterable.kind
        else {
            let value = self.expr(iterable);
            let span = value.span;
            let value = match (change, value.ty) {
                (_, Ty::Error) => value,
                (true, Ty::Text) => self.fail("E0303", "text cannot change".into(), span),
                (true, Ty::List(_)) => self.unsupported("`for change` on a list", span),
                (true, Ty::Map(..) | Ty::Set(_)) => {
                    self.fail("E0303", "a key cannot change".into(), span)
                }
                (false, Ty::Text | Ty::List(_) | Ty::Map(..) | Ty::Set(_)) => value,
                (_, ty) => {
                    self.fail("E0303", format!("`for` cannot loop over `{}`", ty.name()), span)
                }
            };
            return self.for_each(value, name, body);
        };
        let (start, end) = self.range(start, end);
        if change {
            self.fail("E0303", "a range has nothing to change".into(), iterable.span);
        }
        let (local, body) = self.loop_body(name, Ty::Int, body);
        Stmt::For { local, start, end, inclusive: *op == BinaryOp::RangeInclusive, body }
    }

    /// Lowers to a `while` over a hidden position. A hidden local keeps the
    /// value alive and fixed, since the body may assign the variable it came
    /// from. The position moves before the body, so `continue` needs no
    /// separate step. A text position is a byte offset, a map's is an entry.
    fn for_each(&mut self, value: Expr, name: &'a ast::Name, body: &'a [ast::Stmt]) -> Stmt {
        let (span, ty) = (value.span, value.ty);
        let item = match ty {
            Ty::List(item) | Ty::Set(item) | Ty::Map(item, _) => *item,
            _ => ty,
        };
        let owner = self.new_local(ty, false);
        let position = self.new_local(Ty::Int, true);
        let (current, body) = self.loop_body(name, item, body);

        let expr = |kind, ty| Expr { kind, ty, span };
        let local = |id, ty| Box::new(expr(ExprKind::Local(id), ty));
        let (held, at) = (local(owner, ty), local(position, Ty::Int));
        let zero = || Box::new(expr(ExprKind::Int(0), Ty::Int));
        let one = Box::new(expr(ExprKind::Int(1), Ty::Int));
        let next = |at| ExprKind::MapNext(held.clone(), at);
        let after = ExprKind::Binary(BinaryOp::Add, at.clone(), one);
        let (first, inside, read, advance) = match ty {
            Ty::Text => {
                let byte_len = |id| Box::new(expr(ExprKind::TextByteLen(local(id, ty)), Ty::Int));
                (
                    ExprKind::Int(0),
                    ExprKind::Binary(BinaryOp::Lt, at.clone(), byte_len(owner)),
                    ExprKind::TextCharAt(held.clone(), at.clone()),
                    ExprKind::Binary(BinaryOp::Add, at, byte_len(current)),
                )
            }
            Ty::List(_) => {
                let len = expr(ExprKind::Len(held.clone()), Ty::Int);
                (
                    ExprKind::Int(0),
                    ExprKind::Binary(BinaryOp::Lt, at.clone(), len.into()),
                    ExprKind::ListGet(held.clone(), at),
                    after,
                )
            }
            _ => (
                next(zero()),
                ExprKind::Binary(BinaryOp::Ge, at.clone(), zero()),
                ExprKind::MapKey(held.clone(), at),
                next(expr(after, Ty::Int).into()),
            ),
        };
        let mut inner =
            vec![Stmt::Set(current, expr(read, item)), Stmt::Set(position, expr(advance, Ty::Int))];
        inner.extend(body);
        Stmt::Block(vec![
            Stmt::Set(owner, value),
            Stmt::Set(position, expr(first, Ty::Int)),
            Stmt::While { condition: expr(inside, Ty::Bool), body: inner },
            Stmt::Clear(owner),
        ])
    }

    fn name(&mut self, name: &str, span: Span) -> Expr {
        if let Some(local) = self.lookup(name) {
            return Expr { kind: ExprKind::Local(local), ty: self.locals[local], span };
        }
        if self.returns.is_some() {
            match self.top.get(name) {
                Some(&Top::Literal(value)) => {
                    // Its own statement reports any error.
                    let reported = self.diags.len();
                    let value = self.expr(value);
                    self.diags.truncate(reported);
                    return Expr { span, ..value };
                }
                Some(Top::Var) => {
                    let message = format!("a task cannot use top-level `var` `{name}`");
                    let diag = Diagnostic::new("E0306", message, span);
                    return self.report(diag.note("pass it as a parameter"));
                }
                Some(Top::Let) => {
                    return self.unsupported("a task reading a non-literal top-level `let`", span);
                }
                None => {}
            }
        }
        if self.task_ids.contains_key(name) {
            return self.unsupported("a task used as a value", span);
        }
        if let Some(&(id, variant)) = self.constructors.get(name) {
            if self.datas[id].variants[variant].fields.is_empty() {
                return self.construct((id, variant), &[], span);
            }
            let diag = Diagnostic::new("E0307", format!("`{name}` needs its fields"), span);
            return self.report(diag.note(format!("write `{name}(field: value)`")));
        }
        self.fail("E0301", format!("name `{name}` is not defined"), span)
    }

    fn int(&mut self, lexeme: &str, span: Span) -> Expr {
        let negative = lexeme.starts_with('-');
        let digits = lexeme.trim_start_matches('-').replace('_', "");
        let parsed = match digits.get(..2) {
            Some("0x") => u64::from_str_radix(&digits[2..], 16),
            Some("0b") => u64::from_str_radix(&digits[2..], 2),
            _ => digits.parse(),
        };
        let limit = i64::MAX as u64 + u64::from(negative);
        match parsed {
            Ok(value) if value <= limit => {
                let value = if negative { value.wrapping_neg() } else { value } as i64;
                Expr { kind: ExprKind::Int(value), ty: Ty::Int, span }
            }
            _ => self.fail("E0312", format!("`{lexeme}` does not fit in `int`"), span),
        }
    }

    fn float(&mut self, lexeme: &str, span: Span) -> Expr {
        let digits = lexeme.replace('_', "");
        let mantissa = digits.split(['e', 'E']).next().unwrap_or_default();
        let nonzero = mantissa.contains(|c: char| matches!(c, '1'..='9'));
        match digits.parse::<f64>() {
            Ok(value) if value.is_finite() && (value != 0.0 || !nonzero) => {
                Expr { kind: ExprKind::Float(value), ty: Ty::Float, span }
            }
            _ => self.fail("E0312", format!("`{lexeme}` does not fit in `float`"), span),
        }
    }

    /// Lowers `expr`. `want` is the type that the context needs, if known,
    /// which is how an empty list or map gets its type.
    fn expected(&mut self, expr: &'a ast::Expr, want: Option<Ty>) -> Expr {
        let span = expr.span;
        match &expr.kind {
            Ast::List(items) => self.list(items, want, span),
            Ast::Map(entries) => self.map(entries, want, span),
            Ast::If { condition, then, otherwise } => {
                let condition = self.condition(condition, "`if`");
                let then = self.expected(then, want);
                let otherwise = self.expected(otherwise, want.or(Some(then.ty)));
                self.expect(&otherwise, then.ty, "`else`, to match `then`,");
                let ty = then.ty;
                let kind = ExprKind::If(condition.into(), then.into(), otherwise.into());
                Expr { kind, ty, span }
            }
            Ast::Call { callee, args } => match &callee.kind {
                Ast::Name(name) if self.is_set_call(name) => self.set_call(args, want, span),
                _ => self.expr(expr),
            },
            _ => self.expr(expr),
        }
    }

    /// True if `set` here means the built-in, and no name of the program hides it.
    fn is_set_call(&self, name: &str) -> bool {
        name == "set"
            && self.lookup(name).is_none()
            && !self.top.contains_key(name)
            && !self.task_ids.contains_key(name)
    }

    fn set_call(&mut self, args: &'a [ast::Arg], want: Option<Ty>, span: Span) -> Expr {
        let [ast::Arg { name: None, change: false, value }] = args else {
            return self.fail("E0307", "`set` takes one list".into(), span);
        };
        let item = match want {
            Some(Ty::Set(item)) => Some(item),
            _ => None,
        };
        let inner_want = if want == Some(Ty::Error) { want } else { item.map(Ty::List) };
        let list = self.expected(value, inner_want);
        match list.ty {
            Ty::Error => poisoned(span),
            Ty::List(listed) => {
                // The written type already reported a bad key.
                if item.is_none() {
                    self.check_key(*listed, list.span);
                }
                Expr { kind: ExprKind::SetOf(list.into()), ty: Ty::Set(listed), span }
            }
            ty => {
                let message = format!("`set` needs a list, found `{}`", ty.name());
                self.fail("E0303", message, list.span)
            }
        }
    }

    fn untyped_empty(&mut self, what: &str, want: Option<Ty>, example: &str, span: Span) -> Expr {
        match want {
            Some(Ty::Error) => return poisoned(span),
            Some(Ty::Set(_)) => {
                let message = format!("an empty {what} does not fit `{}`", want_name(want));
                let diag = Diagnostic::new("E0303", message, span);
                return self.report(diag.note("an empty set is `set([])`"));
            }
            Some(_) => {
                let message = format!("an empty {what} does not fit `{}`", want_name(want));
                return self.fail("E0303", message, span);
            }
            None => {}
        }
        let message = format!("an empty {what} needs a type");
        let diag = Diagnostic::new("E0314", message, span);
        self.report(diag.note(format!("write its type, as in `{example}`")))
    }

    fn list(&mut self, items: &'a [ast::Expr], want: Option<Ty>, span: Span) -> Expr {
        let mut item = match want {
            Some(Ty::List(item)) => Some(*item),
            _ => None,
        };
        let mut values = Vec::with_capacity(items.len());
        for value in items {
            let value = self.expected(value, item);
            let ty = *item.get_or_insert(value.ty);
            self.expect(&value, ty, "this list");
            values.push(value);
        }
        match item {
            Some(Ty::Error) => poisoned(span),
            Some(item) => Expr { kind: ExprKind::ListOf(values), ty: Ty::List(inner(item)), span },
            None => self.untyped_empty("list", want, "let x: list[int] = []", span),
        }
    }

    fn map(&mut self, entries: &'a [(ast::Expr, ast::Expr)], want: Option<Ty>, span: Span) -> Expr {
        let (mut key_ty, mut value_ty) = match want {
            Some(Ty::Map(key, value)) => (Some(*key), Some(*value)),
            _ => (None, None),
        };
        let inferred = key_ty.is_none();
        let mut lowered = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let key = self.expected(key, key_ty);
            let value = self.expected(value, value_ty);
            let want_key = *key_ty.get_or_insert(key.ty);
            let want_value = *value_ty.get_or_insert(value.ty);
            self.expect(&key, want_key, "this map");
            self.expect(&value, want_value, "this map");
            lowered.push((key, value));
        }
        match (key_ty, value_ty) {
            (Some(Ty::Error), _) | (_, Some(Ty::Error)) => poisoned(span),
            (Some(key), Some(value)) => {
                if inferred {
                    self.check_key(key, span);
                }
                let ty = Ty::Map(inner(key), inner(value));
                Expr { kind: ExprKind::MapOf(lowered), ty, span }
            }
            _ => self.untyped_empty("map", want, "let x: map[text, int] = {}", span),
        }
    }

    fn expr(&mut self, expr: &'a ast::Expr) -> Expr {
        let span = expr.span;
        let (kind, ty) = match &expr.kind {
            Ast::Int(lexeme) => return self.int(lexeme, span),
            Ast::Float(lexeme) => return self.float(lexeme, span),
            Ast::Bool(value) => (ExprKind::Bool(*value), Ty::Bool),
            Ast::None => (ExprKind::None, Ty::None),
            Ast::Name(name) => return self.name(name, span),
            Ast::Unary(op, value) => return self.unary(*op, value, span),
            Ast::Binary(op, left, right) => return self.binary(*op, left, right, span),
            Ast::If { .. } | Ast::List(_) | Ast::Map(_) => return self.expected(expr, None),
            Ast::Call { callee, args } => return self.call(callee, args, span),
            Ast::Text(parts) => return self.text(parts, span),
            Ast::Is { .. } => return self.unsupported("`is`", span),
            Ast::Lambda { .. } => return self.unsupported("a lambda", span),
            Ast::Index { base, indices } => return self.index(base, indices, span),
            Ast::Field { base, name } => {
                let value = self.expr(base);
                let Some((slot, ty)) = self.field(value.ty, name) else {
                    return poisoned(span);
                };
                (ExprKind::Field(value.into(), slot), ty)
            }
            Ast::Match { .. } => unreachable!("only `let`, `return`, and statements hold one"),
            Ast::SelfValue => return self.unsupported("`self`", span),
        };
        Expr { kind, ty, span }
    }

    fn index(&mut self, base: &'a ast::Expr, indices: &'a [ast::Expr], span: Span) -> Expr {
        let value = self.expr(base);
        if value.ty != Ty::Text {
            let Some((key, ty)) = self.key(value.ty, indices, span) else {
                return poisoned(span);
            };
            let kind = match value.ty {
                Ty::List(_) => ExprKind::ListGet(value.into(), key.into()),
                _ => ExprKind::MapGet(value.into(), key.into()),
            };
            return Expr { kind, ty, span };
        }
        let (op, start, end) = match indices {
            [
                ast::Expr {
                    kind: Ast::Binary(op @ (BinaryOp::Range | BinaryOp::RangeInclusive), start, end),
                    ..
                },
            ] => (*op, start, end),
            _ => {
                let diag = Diagnostic::new("E0303", "text needs a range index", span);
                return self.report(diag.note("for one character, use `x[i..i + 1]`"));
            }
        };
        let (start, mut end) = self.range(start, end);
        if op == BinaryOp::RangeInclusive {
            let span = end.span;
            let one = Expr { kind: ExprKind::Int(1), ty: Ty::Int, span };
            let kind = ExprKind::Binary(BinaryOp::Add, end.into(), one.into());
            end = Expr { kind, ty: Ty::Int, span };
        }
        let kind = ExprKind::TextSlice(value.into(), start.into(), end.into());
        Expr { kind, ty: Ty::Text, span }
    }

    /// Checks an index of a list or map of type `ty`, or of a value that can
    /// have no index. Returns the index and the element type.
    fn key(&mut self, ty: Ty, indices: &'a [ast::Expr], span: Span) -> Option<(Expr, Ty)> {
        match (ty, indices) {
            (Ty::Error, _) => None,
            (
                Ty::List(_),
                [
                    ast::Expr {
                        kind: Ast::Binary(BinaryOp::Range | BinaryOp::RangeInclusive, ..),
                        ..
                    },
                ],
            ) => {
                self.unsupported("a list slice", span);
                None
            }
            (Ty::List(item), [index]) => {
                let index = self.expected(index, Some(Ty::Int));
                self.expect(&index, Ty::Int, "a list index");
                Some((index, *item))
            }
            (Ty::Map(key, value), [index]) => {
                let index = self.expected(index, Some(*key));
                self.expect(&index, *key, "a map key");
                Some((index, *value))
            }
            (Ty::List(_) | Ty::Map(..), _) => {
                self.fail("E0303", "this needs one index".into(), span);
                None
            }
            (Ty::Set(_), _) => {
                let diag = Diagnostic::new("E0303", "a set has no index", span);
                self.report(diag.note("to test for a value, use `x in items`"));
                None
            }
            (Ty::Text, _) => {
                self.fail("E0303", "text cannot change".into(), span);
                None
            }
            _ => {
                self.unsupported("indexing", span);
                None
            }
        }
    }

    /// Reports a value that `print` and interpolation cannot show yet.
    fn showable(&mut self, value: Expr) -> Expr {
        if value.ty.is_compound() {
            return self.unsupported(&format!("showing `{}`", value.ty.name()), value.span);
        }
        value
    }

    fn text(&mut self, parts: &'a [TextPart], span: Span) -> Expr {
        let mut values: Vec<Expr> = parts
            .iter()
            .map(|part| match part {
                TextPart::Literal(text) => {
                    Expr { kind: ExprKind::Text(text.clone()), ty: Ty::Text, span }
                }
                TextPart::Expr(value) => {
                    let value = self.expr(value);
                    let value = self.showable(value);
                    match value.ty {
                        Ty::Text | Ty::Error => value,
                        _ => {
                            let span = value.span;
                            Expr { kind: ExprKind::ToText(value.into()), ty: Ty::Text, span }
                        }
                    }
                }
            })
            .collect();
        match values.len() {
            0 => Expr { kind: ExprKind::Text(String::new()), ty: Ty::Text, span },
            1 => values.remove(0),
            _ => Expr { kind: ExprKind::Join(values), ty: Ty::Text, span },
        }
    }

    fn unary(&mut self, op: UnaryOp, value: &'a ast::Expr, span: Span) -> Expr {
        match op {
            UnaryOp::Neg => {
                let value = self.expr(value);
                match value.ty {
                    Ty::Error => poisoned(span),
                    Ty::Int => {
                        let zero = Expr { kind: ExprKind::Int(0), ty: Ty::Int, span };
                        let kind = ExprKind::Binary(BinaryOp::Sub, zero.into(), value.into());
                        Expr { kind, ty: Ty::Int, span }
                    }
                    Ty::Float => Expr { kind: ExprKind::Neg(value.into()), ty: Ty::Float, span },
                    Ty::Byte => self.unsupported("arithmetic on `byte`", span),
                    ty => {
                        let message = format!("`-` needs a number, found `{}`", ty.name());
                        self.fail("E0303", message, value.span)
                    }
                }
            }
            UnaryOp::Not => {
                let value = self.condition(value, "`not`");
                Expr { kind: ExprKind::Not(value.into()), ty: Ty::Bool, span }
            }
            UnaryOp::Try => self.unsupported("`try`", span),
            UnaryOp::Spawn => self.unsupported("`spawn`", span),
            UnaryOp::Shared | UnaryOp::Weak => self.unsupported("`shared` and `weak`", span),
        }
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        left: &'a ast::Expr,
        right: &'a ast::Expr,
        span: Span,
    ) -> Expr {
        match op {
            BinaryOp::Fallback => return self.unsupported("`else` on a value", span),
            BinaryOp::In | BinaryOp::NotIn => return self.contains(op, left, right, span),
            BinaryOp::Range | BinaryOp::RangeInclusive => {
                return self.unsupported("a range outside `for`", span);
            }
            _ => {}
        }
        let left = self.expr(left);
        let right = self.expr(right);
        self.binary_values(op, left, right, span)
    }

    fn contains(
        &mut self,
        op: BinaryOp,
        left: &'a ast::Expr,
        right: &'a ast::Expr,
        span: Span,
    ) -> Expr {
        let left = self.expr(left);
        let right = self.expr(right);
        let (item, list) = match right.ty {
            Ty::Error => return poisoned(span),
            Ty::List(item) => (*item, true),
            Ty::Set(item) | Ty::Map(item, _) => (*item, false),
            ty => {
                let message = format!("`in` needs a list, map, or set, found `{}`", ty.name());
                return self.fail("E0303", message, right.span);
            }
        };
        if list && item.is_compound() {
            let what = format!("`in` on a list of `{}`", item.name());
            return self.unsupported(&what, right.span);
        }
        if list && !item.is_key() && item != Ty::Error {
            let message = format!("`in` on a list cannot compare `{}`", item.name());
            return self.fail("E0303", message, right.span);
        }
        self.expect(&left, item, "`in`");
        let kind = if list {
            ExprKind::ListHas(left.into(), right.into())
        } else {
            ExprKind::MapHas(left.into(), right.into())
        };
        let found = Expr { kind, ty: Ty::Bool, span };
        if op == BinaryOp::In {
            return found;
        }
        Expr { kind: ExprKind::Not(found.into()), ty: Ty::Bool, span }
    }

    fn binary_values(&mut self, op: BinaryOp, left: Expr, right: Expr, span: Span) -> Expr {
        if left.ty == Ty::Error || right.ty == Ty::Error {
            return poisoned(span);
        }
        let name = symbol(op);
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            self.expect(&left, Ty::Bool, &format!("`{name}`"));
            self.expect(&right, Ty::Bool, &format!("`{name}`"));
            let (then, otherwise) = if op == BinaryOp::And {
                (right, Expr { kind: ExprKind::Bool(false), ty: Ty::Bool, span })
            } else {
                (Expr { kind: ExprKind::Bool(true), ty: Ty::Bool, span }, right)
            };
            let kind = ExprKind::If(left.into(), then.into(), otherwise.into());
            return Expr { kind, ty: Ty::Bool, span };
        }
        if left.ty != right.ty {
            let (a, b) = (left.ty.name(), right.ty.name());
            let message = format!("`{name}` needs one type on both sides, found `{a}` and `{b}`");
            let note = if left.ty == Ty::Text || right.ty == Ty::Text {
                JOIN_TEXT
            } else {
                "conversions are explicit: use `.to_float()` or `.to_int()`"
            };
            return self.report(Diagnostic::new("E0303", message, span).note(note));
        }
        let ty = left.ty;
        let equality = matches!(op, BinaryOp::Eq | BinaryOp::Ne);
        let order = matches!(op, BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge);
        let int_only = matches!(op, BinaryOp::FloorDiv | BinaryOp::Rem);
        let valid = match ty {
            Ty::Int => true,
            Ty::Float => !int_only,
            Ty::Bool => equality,
            Ty::Text => equality || order,
            Ty::Byte if equality || order => true,
            Ty::Byte => return self.unsupported("arithmetic on `byte`", span),
            ty if ty.is_compound() && equality => {
                return self.unsupported(&format!("`{name}` on `{}`", ty.name()), span);
            }
            _ => false,
        };
        if !valid {
            let needs = if int_only { "`int`" } else { "a number" };
            let message = format!("`{name}` needs {needs}, found `{}`", ty.name());
            let diag = Diagnostic::new("E0303", message, left.span);
            return self.report(if ty == Ty::Text && op == BinaryOp::Add {
                diag.note(JOIN_TEXT)
            } else {
                diag
            });
        }
        let result = match op {
            _ if equality || order => Ty::Bool,
            BinaryOp::Div => Ty::Float,
            _ => ty,
        };
        let float = |value: Expr| {
            let span = value.span;
            Expr { kind: ExprKind::ToFloat(value.into()), ty: Ty::Float, span }
        };
        let (left, right) = if op == BinaryOp::Div && ty == Ty::Int {
            (float(left), float(right))
        } else {
            (left, right)
        };
        Expr { kind: ExprKind::Binary(op, left.into(), right.into()), ty: result, span }
    }

    fn call(&mut self, callee: &'a ast::Expr, args: &'a [ast::Arg], span: Span) -> Expr {
        match &callee.kind {
            Ast::Field { base, name } if CHANGING_CALLS.contains(&name.text.as_str()) => {
                let value = self.expr(base);
                match value.ty {
                    Ty::Error => poisoned(span),
                    ty if ty.is_collection() => {
                        let message =
                            format!("`{}` gives no value, so it must be a statement", name.text);
                        self.fail("E0303", message, span)
                    }
                    _ => self.unsupported("this dot call", span),
                }
            }
            Ast::Field { base, name } if args.is_empty() => {
                let value = self.expr(base);
                match (value.ty, name.text.as_str()) {
                    (Ty::Error, _) => poisoned(span),
                    (Ty::Int, "to_float") => {
                        Expr { kind: ExprKind::ToFloat(value.into()), ty: Ty::Float, span }
                    }
                    (Ty::Float, "to_int") => {
                        Expr { kind: ExprKind::ToInt(value.into()), ty: Ty::Int, span }
                    }
                    (Ty::Text, "len") => {
                        Expr { kind: ExprKind::TextLen(value.into()), ty: Ty::Int, span }
                    }
                    (Ty::Text, "is_empty") => {
                        Expr { kind: ExprKind::TextIsEmpty(value.into()), ty: Ty::Bool, span }
                    }
                    (Ty::Text, "bytes") => {
                        let ty = Ty::List(&Ty::Byte);
                        Expr { kind: ExprKind::TextBytes(value.into()), ty, span }
                    }
                    (Ty::Byte, "to_int") => Expr { ty: Ty::Int, ..value },
                    (ty, "len") if ty.is_collection() => {
                        Expr { kind: ExprKind::Len(value.into()), ty: Ty::Int, span }
                    }
                    (ty, "is_empty") if ty.is_collection() => {
                        let int = |kind| Box::new(Expr { kind, ty: Ty::Int, span });
                        let len = int(ExprKind::Len(value.into()));
                        let kind = ExprKind::Binary(BinaryOp::Eq, len, int(ExprKind::Int(0)));
                        Expr { kind, ty: Ty::Bool, span }
                    }
                    (ty, "to_float" | "to_int" | "len" | "is_empty" | "bytes") => {
                        let message = format!("`{}` does not work on `{}`", name.text, ty.name());
                        self.fail("E0303", message, span)
                    }
                    _ => self.unsupported("this dot call", span),
                }
            }
            Ast::Name(name) if self.is_set_call(name) => self.set_call(args, None, span),
            Ast::Name(name) => {
                if let Some(&target) = self.constructors.get(name.as_str())
                    && self.lookup(name).is_none()
                    && !self.top.contains_key(name.as_str())
                {
                    let (id, variant) = target;
                    if self.datas[id].variants[variant].fields.is_empty() {
                        let diag =
                            Diagnostic::new("E0307", format!("`{name}` has no fields"), span);
                        return self.report(diag.note(format!("write `{name}`, with no `()`")));
                    }
                    return self.construct(target, args, span);
                }
                if let Some(&id) = self.task_ids.get(name.as_str())
                    && self.lookup(name).is_none()
                    && !self.top.contains_key(name.as_str())
                {
                    return self.task_call(id, name, args, span);
                }
                for arg in args {
                    self.expected(&arg.value, Some(Ty::Error));
                }
                if self.lookup(name).is_some() || self.top.contains_key(name.as_str()) {
                    return self.fail("E0308", format!("`{name}` is not a task"), callee.span);
                }
                self.fail("E0301", format!("name `{name}` is not defined"), callee.span)
            }
            _ => self.unsupported("this call", span),
        }
    }

    /// Named arguments follow positional ones, in parameter order, so that the
    /// order of evaluation is always the order on the page.
    fn task_call(&mut self, id: usize, name: &str, args: &'a [ast::Arg], span: Span) -> Expr {
        let count = self.signatures[id].params.len();
        let required = self.signatures[id].params.iter().filter(|p| p.default.is_none()).count();
        let mut values: Vec<Option<Expr>> = vec![None; count];
        let mut next = 0;
        let mut named = false;
        for arg in args {
            if arg.change {
                self.unsupported("a `change` argument", arg.value.span);
            }
            let params = &self.signatures[id].params;
            let want = match &arg.name {
                None => params.get(next),
                Some(label) => params.iter().find(|p| p.name == label.text),
            };
            let value = self.expected(&arg.value, Some(want.map_or(Ty::Error, |p| p.ty)));
            let slot = match &arg.name {
                None if named => Err("a positional argument cannot follow a named one".into()),
                None if next >= required => {
                    Err(format!("`{name}` has too many positional arguments"))
                }
                None => Ok(next),
                Some(label) => {
                    named = true;
                    let params = &self.signatures[id].params;
                    match params.iter().position(|p| p.name == label.text) {
                        None => Err(format!("`{name}` has no parameter `{}`", label.text)),
                        Some(slot) if params[slot].default.is_none() => {
                            Err(format!("`{}` has no default, so pass it by position", label.text))
                        }
                        Some(slot) if slot < next => {
                            Err(format!("`{}` is out of parameter order", label.text))
                        }
                        Some(slot) => Ok(slot),
                    }
                }
            };
            match slot {
                Ok(slot) => {
                    self.expect(&value, self.signatures[id].params[slot].ty, "this argument");
                    values[slot] = Some(value);
                    next = slot + 1;
                }
                Err(message) => {
                    self.fail("E0307", message, arg.value.span);
                }
            }
        }
        let mut lowered = Vec::with_capacity(count);
        for (slot, value) in values.into_iter().enumerate() {
            let param = &self.signatures[id].params[slot];
            match value.or_else(|| param.default.clone()) {
                Some(value) => lowered.push(value),
                None => {
                    let message = format!("`{name}` is missing argument `{}`", param.name);
                    self.fail("E0307", message, span);
                }
            }
        }
        Expr { kind: ExprKind::Call(id, lowered), ty: self.signatures[id].returns, span }
    }
}
