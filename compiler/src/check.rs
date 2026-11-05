//! Binds names, checks types, and lowers the syntax tree to MIR in one pass.
//! There are no modules or generics yet, so nothing needs separate stages.

use crate::ast::{self, BinaryOp, ExprKind as Ast, StmtKind, TextPart, TypeKind, UnaryOp};
use crate::diag::{Diagnostic, Span};
use crate::mir::{Expr, ExprKind, Local, Program, Stmt, Task, Ty};
use std::collections::HashMap;

/// Names of the spec's built-in types that this milestone cannot represent.
const LATER_TYPES: [&str; 15] = [
    "text", "byte", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "list", "map",
    "set", "pointer",
];

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
enum Top {
    Var,
    /// A `let` with a literal value, which is known before any statement runs.
    Literal(Expr),
    Let,
}

#[derive(Default)]
struct Checker<'a> {
    diags: Vec<Diagnostic>,
    task_ids: HashMap<&'a str, usize>,
    signatures: Vec<Signature<'a>>,
    top: HashMap<&'a str, Top>,
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
    matches!(expr.kind, Ast::Int(_) | Ast::Float(_) | Ast::Bool(_))
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
                ([name], []) => name.text.as_str(),
                _ => "",
            },
            _ => "",
        };
        match name {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "bool" => Ty::Bool,
            "" => self.unsupported("this type", ty.span).ty,
            name if LATER_TYPES.contains(&name) => {
                self.unsupported(&format!("type `{name}`"), ty.span).ty
            }
            name => self.fail("E0302", format!("type `{name}` is not defined"), ty.span).ty,
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
                    (false, true) => {
                        // Its own statement reports any error, later.
                        let reported = self.diags.len();
                        let value = self.expr(value);
                        self.diags.truncate(reported);
                        Top::Literal(value)
                    }
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
                let value = self.expr(value);
                let ty = ty.as_ref().map_or(value.ty, |ty| self.ty(ty));
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
                return self.for_loop(*change, name, iterable, body);
            }
            StmtKind::Return(value) => {
                let none = Expr { kind: ExprKind::None, ty: Ty::None, span };
                let value = value.as_ref().map_or(none, |value| self.expr(value));
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
                    _ => return self.later("text interpolation", value.span),
                },
                _ => Stmt::Print(self.expr(value)),
            },
            StmtKind::Break | StmtKind::Continue if self.loops == 0 => {
                self.fail("E0311", "`break` and `continue` need a loop".into(), span);
                return None;
            }
            StmtKind::Break => Stmt::Break,
            StmtKind::Continue => Stmt::Continue,
            StmtKind::Placeholder => Stmt::Placeholder(span),
            StmtKind::Expr(value) => Stmt::Expr(self.expr(value)),
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
        let Ast::Name(name) = &target.kind else {
            return self.later("assignment to a field or index", target.span);
        };
        if self.lookup(name).is_none() && self.task_ids.contains_key(name.as_str()) {
            self.expr(value);
            self.fail("E0305", format!("`{name}` cannot change"), target.span);
            return None;
        }
        let place = self.name(name, target.span);
        if place.ty == Ty::Error {
            self.expr(value);
            return None;
        }
        let ExprKind::Local(local) = place.kind else {
            self.expr(value);
            self.fail("E0305", format!("`{name}` cannot change"), target.span);
            return None;
        };
        if !self.mutable[local] {
            let diag = Diagnostic::new("E0305", format!("`{name}` cannot change"), target.span);
            self.report(diag.note("only a name declared with `var` can change"));
        }
        let op = match op {
            ast::AssignOp::Set => None,
            ast::AssignOp::Add => Some(BinaryOp::Add),
            ast::AssignOp::Sub => Some(BinaryOp::Sub),
            ast::AssignOp::Mul => Some(BinaryOp::Mul),
            ast::AssignOp::Div => Some(BinaryOp::Div),
        };
        let value = match op {
            Some(op) => self.binary(op, target, value, span),
            None => self.expr(value),
        };
        self.expect(&value, self.locals[local], "this assignment");
        Some(Stmt::Set(local, value))
    }

    fn for_loop(
        &mut self,
        change: bool,
        name: &'a ast::Name,
        iterable: &'a ast::Expr,
        body: &'a [ast::Stmt],
    ) -> Option<Stmt> {
        let range = match &iterable.kind {
            Ast::Binary(op @ (BinaryOp::Range | BinaryOp::RangeInclusive), start, end) => {
                let start = self.expr(start);
                let end = self.expr(end);
                self.expect(&start, Ty::Int, "a range");
                self.expect(&end, Ty::Int, "a range");
                if change {
                    self.fail("E0303", "a range has nothing to change".into(), iterable.span);
                }
                Some((start, end, *op == BinaryOp::RangeInclusive))
            }
            _ => {
                let value = self.expr(iterable);
                if value.ty != Ty::Error {
                    let message = format!("`for` cannot loop over `{}`", value.ty.name());
                    self.fail("E0303", message, value.span);
                }
                None
            }
        };
        self.scopes.push(HashMap::new());
        let ty = if range.is_some() { Ty::Int } else { Ty::Error };
        let local = self.bind(name, ty, false);
        self.loops += 1;
        let body = self.stmts(body);
        self.loops -= 1;
        self.scopes.pop();
        let (start, end, inclusive) = range?;
        Some(Stmt::For { local, start, end, inclusive, body })
    }

    fn name(&mut self, name: &str, span: Span) -> Expr {
        if let Some(local) = self.lookup(name) {
            return Expr { kind: ExprKind::Local(local), ty: self.locals[local], span };
        }
        if self.returns.is_some() {
            match self.top.get(name) {
                Some(Top::Literal(value)) => return Expr { span, ..value.clone() },
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
            Ast::If { condition, then, otherwise } => {
                let condition = self.condition(condition, "`if`");
                let then = self.expr(then);
                let otherwise = self.expr(otherwise);
                self.expect(&otherwise, then.ty, "`else`, to match `then`,");
                let ty = then.ty;
                (ExprKind::If(condition.into(), then.into(), otherwise.into()), ty)
            }
            Ast::Call { callee, args } => return self.call(callee, args, span),
            Ast::Text(_) => return self.unsupported("a text value", span),
            Ast::List(_) => return self.unsupported("a list", span),
            Ast::Map(_) => return self.unsupported("a map", span),
            Ast::Is { .. } => return self.unsupported("`is`", span),
            Ast::Lambda { .. } => return self.unsupported("a lambda", span),
            Ast::Index { .. } => return self.unsupported("indexing", span),
            Ast::Field { .. } => return self.unsupported("a field", span),
            Ast::Match { .. } => return self.unsupported("`match`", span),
            Ast::SelfValue => return self.unsupported("`self`", span),
        };
        Expr { kind, ty, span }
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
            BinaryOp::In | BinaryOp::NotIn => return self.unsupported("`in`", span),
            BinaryOp::Range | BinaryOp::RangeInclusive => {
                return self.unsupported("a range outside `for`", span);
            }
            _ => {}
        }
        let left = self.expr(left);
        let right = self.expr(right);
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
            let note = "conversions are explicit: use `.to_float()` or `.to_int()`";
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
            _ => false,
        };
        if !valid {
            let needs = if int_only { "`int`" } else { "a number" };
            let message = format!("`{name}` needs {needs}, found `{}`", ty.name());
            return self.fail("E0303", message, left.span);
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
                    (ty, "to_float" | "to_int") => {
                        let message = format!("`{}` does not work on `{}`", name.text, ty.name());
                        self.fail("E0303", message, span)
                    }
                    _ => self.unsupported("this dot call", span),
                }
            }
            Ast::Name(name) => {
                if let Some(&id) = self.task_ids.get(name.as_str())
                    && self.lookup(name).is_none()
                    && !self.top.contains_key(name.as_str())
                {
                    return self.task_call(id, name, args, span);
                }
                for arg in args {
                    self.expr(&arg.value);
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
            let value = self.expr(&arg.value);
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
