//! Binds names, checks types, and lowers the syntax tree to MIR in one pass.
//! There are no modules or generics yet, so nothing needs separate stages.
//! List, map, and set tasks are built in until generics can express them.

use crate::ast::{self, BinaryOp, ExprKind as Ast, StmtKind, TextPart, TypeKind, UnaryOp};
use crate::diag::{Diagnostic, Span};
use crate::mir::{Expr, ExprKind, Local, Program, Stmt, Task, Ty, Update, inner};
use std::collections::HashMap;

/// Names of the spec's built-in types that this milestone cannot represent.
const LATER_TYPES: [&str; 10] =
    ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "pointer"];

/// Dot calls that change their receiver, so they only work as statements.
const CHANGING_CALLS: [&str; 4] = ["push", "swap", "add", "remove"];

const JOIN_TEXT: &str = "join text with interpolation: \"{a}{b}\"";

/// Checks a file. A program using an unbuilt feature gets only E0901, because
/// names and types that feature would supply make any other error unreliable.
pub fn check(file: &ast::File) -> (Program, Vec<Diagnostic>) {
    let mut checker = Checker::default();
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

    fn ty(&mut self, ty: &ast::Type) -> Ty {
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
            name => self.fail("E0302", format!("type `{name}` is not defined"), ty.span).ty,
        }
    }

    fn collection_type(&mut self, name: &str, args: &[ast::Type], span: Span) -> Ty {
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
            _ if ty.is_collection() => {
                self.unsupported("a collection as a key", span);
            }
            _ if ty.is_key() => {}
            _ => {
                let message = format!("`{}` cannot be a key", ty.name());
                self.fail("E0303", message, span);
            }
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
            if self.task_ids.contains_key(name) {
                self.fail("E0304", format!("`{name}` is already defined"), def.sig.name.span);
            } else {
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
            let default = param.default.as_ref().map(|value| {
                if !is_literal(value) {
                    return self.fail("E0313", "a default must be a literal".into(), value.span);
                }
                let value = self.expr(value);
                self.expect(&value, ty, "this default");
                value
            });
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
        if scope.contains_key(text) || (top_level && self.task_ids.contains_key(text)) {
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
                let value = value.as_ref().map_or(none, |value| self.expected(value, self.returns));
                match self.returns {
                    Some(returns) => self.expect(&value, returns, "this `return`"),
                    None => {
                        self.fail("E0310", "`return` is only allowed in a task".into(), span);
                    }
                }
                Stmt::Return(value)
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
            StmtKind::Expr(value) => match &value.kind {
                Ast::Call { callee, args } => match &callee.kind {
                    Ast::Field { base, name }
                        if CHANGING_CALLS.contains(&name.text.as_str())
                            && self.is_collection(base) =>
                    {
                        return self.changing_call(base, name, args, span);
                    }
                    _ => Stmt::Expr(self.expr(value)),
                },
                _ => Stmt::Expr(self.expr(value)),
            },
            StmtKind::Use { .. } => return self.later("`use`", span),
            StmtKind::Test { .. } => return self.later("`test`", span),
            StmtKind::Type(_) => return self.later("`type`", span),
            StmtKind::Choice(_) => return self.later("`choice`", span),
            StmtKind::Trait(_) => return self.later("`trait`", span),
            StmtKind::Expect(_) => return self.later("`expect`", span),
            StmtKind::Unsafe(_) => return self.later("`unsafe`", span),
        })
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
            _ => return self.later("assignment to a field", target.span),
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
                let value = self.new_local(item, true);
                place.setup.push(Stmt::Set(value, read));
                let span = target.span;
                let empty = Expr { kind: ExprKind::None, ty: Ty::None, span };
                place.take.push(self.write(place.local, self.read(key, span), empty, span));
                let stored = self.read(value, span);
                let write = self.write(place.local, self.read(key, span), stored, span);
                place.finish.insert(0, write);
                place.finish.push(Stmt::Clear(value));
                place.local = value;
                Some(place)
            }
            _ => {
                let diag = Diagnostic::new("E0305", "this value cannot change", target.span);
                self.report(diag.note("only a name declared with `var` can change"));
                None
            }
        }
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
            Ast::Field { .. } => return self.unsupported("a field", span),
            Ast::Match { .. } => return self.unsupported("`match`", span),
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
        if value.ty.is_collection() {
            return self.unsupported("showing a collection", value.span);
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
        if list && item.is_collection() {
            return self.unsupported("`in` on a list of collections", right.span);
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
            ty if ty.is_collection() && equality => {
                return self.unsupported("`==` on a collection", span);
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
