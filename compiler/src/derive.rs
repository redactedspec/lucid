//! Derived tasks: showing, comparing, and searching values made of other values.
//! Each is ordinary MIR for one concrete type, so `ownership.rs` and the
//! backends need nothing new. See decision 69.
//!
//! ponytail: `Show` joins text one part at a time, so it is quadratic in the
//! length of its output. A text builder can follow if a program needs one.

use crate::ast::{BinaryOp, short_name};
use crate::diag::{Span, plain};
use crate::mir::{Expr, ExprKind, HashOp, Local, Stmt, Task, Ty, inner, json_tags};
use crate::mono::Mono;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub enum Derived {
    /// `x: T`, gives `text`.
    Show,
    /// `a: T, b: T`, gives `bool`.
    Equal,
    /// `a: T, b: T`, gives a negative `int`, zero, or a positive one.
    Compare,
    /// `value: T, list: list[T]`, gives `bool`.
    Has,
    /// `x: T`, gives `text`, with no quotes on a text that `x` holds directly.
    ShowAlone,
    /// `x: T, state: int`, gives `state` with `x` added.
    Hash,
    /// `x: T`, gives its JSON `text`. Only a `type`, `choice`, or union has this.
    JsonWrite,
    /// `value: Value`, gives `T or JsonError`, in the terms of `std/json.lcd`.
    /// Only a `type`, `choice`, or union has this.
    JsonRead,
    /// A task of the trait of an `any`: runs the one that its value has.
    ///
    /// ponytail: a linear test of each type held, with no new backend code. A
    /// table indexed by tag can follow if M4 measures a slow call.
    Task(&'static str),
}

/// What a derived task is building.
struct Body {
    locals: Vec<Ty>,
    span: Span,
}

fn boxed(value: Expr) -> Box<Expr> {
    Box::new(value)
}

impl Body {
    fn expr(&self, kind: ExprKind, ty: Ty) -> Expr {
        Expr { kind, ty, span: self.span }
    }

    fn int(&self, value: i64) -> Expr {
        self.expr(ExprKind::Int(value), Ty::Int)
    }

    fn flag(&self, value: bool) -> Expr {
        self.expr(ExprKind::Bool(value), Ty::Bool)
    }

    fn text(&self, text: &str) -> Expr {
        self.expr(ExprKind::Text(text.into()), Ty::Text)
    }

    fn read(&self, local: Local) -> Expr {
        self.expr(ExprKind::Local(local), self.locals[local])
    }

    fn new_local(&mut self, ty: Ty) -> Local {
        self.locals.push(ty);
        self.locals.len() - 1
    }

    fn test(&self, op: BinaryOp, left: Expr, right: Expr) -> Expr {
        self.expr(ExprKind::Binary(op, boxed(left), boxed(right)), Ty::Bool)
    }

    fn plus_one(&self, value: Expr) -> Expr {
        self.expr(ExprKind::Binary(BinaryOp::Add, boxed(value), boxed(self.int(1))), Ty::Int)
    }

    fn hash(&self, op: HashOp, state: Expr, value: Expr) -> Expr {
        self.expr(ExprKind::Hash(op, boxed(state), boxed(value)), Ty::Int)
    }

    /// The value at a position in a map.
    fn value_at(&self, map: Local, position: Local, ty: Ty) -> Expr {
        let kind = ExprKind::MapValue(boxed(self.read(map)), boxed(self.read(position)));
        self.expr(kind, ty)
    }

    fn join(&self, parts: Vec<Expr>) -> Expr {
        self.expr(ExprKind::Join(parts), Ty::Text)
    }

    fn len(&self, value: Expr) -> Expr {
        self.expr(ExprKind::Len(boxed(value)), Ty::Int)
    }

    fn item(&self, list: Expr, index: Local, ty: Ty) -> Expr {
        self.expr(ExprKind::ListGet(boxed(list), boxed(self.read(index))), ty)
    }

    fn slot(&self, record: Local, slot: usize, ty: Ty) -> Expr {
        self.expr(ExprKind::Field(boxed(self.read(record)), slot), ty)
    }

    fn tag(&self, record: Local) -> Expr {
        self.expr(ExprKind::Tag(boxed(self.read(record))), Ty::Int)
    }

    fn holds(&self, record: Local, id: usize) -> Expr {
        self.expr(ExprKind::TagIn(boxed(self.read(record)), vec![id]), Ty::Bool)
    }

    /// `if condition: return value`.
    fn return_if(&self, condition: Expr, value: Expr) -> Stmt {
        Stmt::If { branches: vec![(condition, vec![Stmt::Return(value)])], otherwise: Vec::new() }
    }

    /// Visits the indexes of `list`. The body may return.
    fn each_item(&mut self, list: Local, body: impl FnOnce(&mut Self, Local) -> Vec<Stmt>) -> Stmt {
        let index = self.new_local(Ty::Int);
        let end = self.len(self.read(list));
        let body = body(self, index);
        Stmt::For { local: index, start: self.int(0), end, inclusive: false, body }
    }

    /// Visits the keys of a map or set in `map`, with their positions. The body
    /// may return.
    fn each_key(
        &mut self,
        map: Local,
        key: Ty,
        body: impl FnOnce(&mut Self, Local, Local) -> Vec<Stmt>,
    ) -> Vec<Stmt> {
        let position = self.new_local(Ty::Int);
        let held = self.new_local(key);
        let next = |this: &Self, at: Expr| {
            this.expr(ExprKind::MapNext(boxed(this.read(map)), boxed(at)), Ty::Int)
        };
        let first = next(self, self.int(0));
        let after = next(self, self.plus_one(self.read(position)));
        let at =
            self.expr(ExprKind::MapKey(boxed(self.read(map)), boxed(self.read(position))), key);
        let mut inner = vec![Stmt::Set(held, at)];
        inner.extend(body(self, held, position));
        inner.push(Stmt::Set(position, after));
        let condition = self.test(BinaryOp::Ge, self.read(position), self.int(0));
        vec![Stmt::Set(position, first), Stmt::While { condition, body: inner }]
    }
}

impl Mono<'_> {
    /// The text of a value inside a larger value, where text has quotes.
    pub fn shown(&mut self, value: Expr, span: Span) -> Expr {
        let ty = value.ty;
        match ty {
            Ty::Text => Expr { kind: ExprKind::TextQuote(boxed(value)), ty, span },
            // A task value or a thread shows its type: its code has no text.
            Ty::Task(..) | Ty::Spawned(_) | Ty::Channel(_) | Ty::Lock(_) => {
                let name = Expr { kind: ExprKind::Text(ty.name()), ty: Ty::Text, span };
                let kind = ExprKind::Seq(vec![Stmt::Expr(value)], boxed(name));
                Expr { kind, ty: Ty::Text, span }
            }
            _ if ty.is_compound() => {
                self.call_written_or(Derived::Show, "show", vec![value], Ty::Text, span)
            }
            _ => Expr { kind: ExprKind::ToText(boxed(value)), ty: Ty::Text, span },
        }
    }

    /// The text of a value by itself, which `print` and interpolation show.
    /// Text has no quotes, even when a union holds it.
    pub fn shown_alone(&mut self, value: Expr, span: Span) -> Expr {
        if value.ty == Ty::Text {
            return value;
        }
        let ty = value.ty;
        let written = ty.is_compound() && self.method("show", ty, span).is_some();
        let may_hold_text = ty.members().iter().any(|m| matches!(m, Ty::Text | Ty::Any(_)));
        if written || !may_hold_text {
            return self.shown(value, span);
        }
        let slot = self.derived(Derived::ShowAlone, ty, span);
        Expr { kind: ExprKind::Call(slot, &[], vec![value]), ty: Ty::Text, span }
    }

    /// `left == right` for any type.
    fn equal(&mut self, left: Expr, right: Expr, span: Span) -> Expr {
        match left.ty {
            Ty::None => Expr { kind: ExprKind::Bool(true), ty: Ty::Bool, span },
            ty if ty.is_compound() => {
                self.call_written_or(Derived::Equal, "equal", vec![left, right], Ty::Bool, span)
            }
            _ => {
                let kind = ExprKind::Binary(BinaryOp::Eq, boxed(left), boxed(right));
                Expr { kind, ty: Ty::Bool, span }
            }
        }
    }

    /// A negative `int`, zero, or a positive one. Both values are read twice
    /// for a number or text, so they must be plain reads. NaN gives zero.
    fn compared(&mut self, left: Expr, right: Expr, span: Span) -> Expr {
        if left.ty.is_compound() {
            let args = vec![left, right];
            return self.call_written_or(Derived::Compare, "compare", args, Ty::Int, span);
        }
        let int = |value| Expr { kind: ExprKind::Int(value), ty: Ty::Int, span };
        let test = |op, a: &Expr, b: &Expr| {
            let kind = ExprKind::Binary(op, boxed(a.clone()), boxed(b.clone()));
            Expr { kind, ty: Ty::Bool, span }
        };
        let (less, more) = (test(BinaryOp::Lt, &left, &right), test(BinaryOp::Gt, &left, &right));
        let later = ExprKind::If(boxed(more), boxed(int(1)), boxed(int(0)));
        let later = Expr { kind: later, ty: Ty::Int, span };
        Expr { kind: ExprKind::If(boxed(less), boxed(int(-1)), boxed(later)), ty: Ty::Int, span }
    }

    /// An operator on values made of other values. `ty` is its result.
    pub fn compare(&mut self, op: BinaryOp, left: Expr, right: Expr, ty: Ty, span: Span) -> Expr {
        let arithmetic = match op {
            BinaryOp::Add => Some("add"),
            BinaryOp::Sub => Some("sub"),
            BinaryOp::Mul => Some("mul"),
            BinaryOp::Div => Some("div"),
            _ => None,
        };
        if let Some(name) = arithmetic {
            let slot = self.method(name, left.ty, span);
            let slot = slot.expect("the checker found this task");
            return Expr { kind: ExprKind::Call(slot, &[], vec![left, right]), ty, span };
        }
        match op {
            BinaryOp::Eq => self.equal(left, right, span),
            BinaryOp::Ne => {
                let equal = self.equal(left, right, span);
                Expr { kind: ExprKind::Not(boxed(equal)), ty: Ty::Bool, span }
            }
            _ => {
                let order = self.compared(left, right, span);
                let zero = Expr { kind: ExprKind::Int(0), ty: Ty::Int, span };
                let kind = ExprKind::Binary(op, boxed(order), boxed(zero));
                Expr { kind, ty: Ty::Bool, span }
            }
        }
    }

    /// `state` with `value` added. Nothing is added for a `none`.
    fn hashed(&mut self, state: Expr, value: Expr, span: Span) -> Expr {
        let op = match value.ty {
            Ty::None => return state,
            Ty::Text => HashOp::Text,
            ty if ty.is_compound() => {
                let slot = self.derived(Derived::Hash, ty, span);
                let kind = ExprKind::Call(slot, &[], vec![value, state]);
                return Expr { kind, ty: Ty::Int, span };
            }
            _ => HashOp::Int,
        };
        Expr { kind: ExprKind::Hash(op, boxed(state), boxed(value)), ty: Ty::Int, span }
    }

    /// `value in list` for a type that the runtime cannot compare.
    pub fn list_has(&mut self, value: Expr, list: Expr, span: Span) -> Expr {
        let slot = self.derived(Derived::Has, value.ty, span);
        Expr { kind: ExprKind::Call(slot, &[], vec![value, list]), ty: Ty::Bool, span }
    }

    /// A call to `written_or`, on the type of the first argument.
    fn call_written_or(
        &mut self,
        kind: Derived,
        name: &str,
        args: Vec<Expr>,
        returns: Ty,
        span: Span,
    ) -> Expr {
        let ty = args[0].ty;
        let slot = self.written_or(kind, name, ty, span);
        Expr { kind: ExprKind::Call(slot, &[], args), ty: returns, span }
    }

    /// The task for a trait on a type: a hand-written one, or else the derived.
    pub fn written_or(&mut self, kind: Derived, name: &str, ty: Ty, span: Span) -> usize {
        match self.method(name, ty, span) {
            Some(slot) => slot,
            None => self.derived(kind, ty, span),
        }
    }

    /// Each field of a variant of `ty`: its name, type, and slot.
    fn fields_of(&mut self, ty: Ty, variant: usize) -> Vec<(String, Ty, usize)> {
        let Ty::Data(_, id, args) = ty else { unreachable!("only a `type` or `choice`") };
        let fields = self.datas[id].variants[variant].fields.clone();
        let mut found = Vec::with_capacity(fields.len());
        for (index, (name, field)) in fields.into_iter().enumerate() {
            let (slot, _) = self.layout(ty, variant, index);
            found.push((name, self.types.subst(field, args), slot));
        }
        found
    }

    fn variant_count(&self, ty: Ty) -> usize {
        let Ty::Data(_, id, _) = ty else { unreachable!("only a `type` or `choice`") };
        self.datas[id].variants.len()
    }

    /// Builds a derived task.
    pub fn derive(&mut self, kind: Derived, ty: Ty, span: Span) -> Task {
        let (locals, returns) = match kind {
            Derived::Show | Derived::ShowAlone => (vec![ty], Ty::Text),
            Derived::Equal => (vec![ty, ty], Ty::Bool),
            Derived::Compare => (vec![ty, ty], Ty::Int),
            Derived::Has => (vec![ty, Ty::List(inner(ty))], Ty::Bool),
            Derived::JsonWrite => (vec![ty], Ty::Text),
            Derived::JsonRead => {
                let (value, error) = self.json_types();
                (vec![value], self.types.union([ty, error]))
            }
            Derived::Hash => (vec![ty, Ty::Int], Ty::Int),
            Derived::Task(name) => self.calls[&(name, ty)].clone(),
        };
        let params = locals.len();
        let mut body = Body { locals, span };
        let stmts = match kind {
            Derived::Show => self.show_body(&mut body, ty),
            Derived::ShowAlone => {
                let mut stmts = Vec::new();
                for member in self.held(ty) {
                    let id = self.types.id(member);
                    let value = self.shown_alone(body.slot(0, 0, member), span);
                    stmts.push(body.return_if(body.holds(0, id), value));
                }
                stmts.push(Stmt::Unreachable);
                stmts
            }
            Derived::Equal => self.equal_body(&mut body, ty),
            Derived::Compare => self.compare_body(&mut body, ty),
            Derived::Hash => self.hash_body(&mut body, ty),
            Derived::JsonWrite => self.json_write_body(&body, ty),
            Derived::JsonRead => self.json_read_body(&mut body, ty, returns),
            Derived::Task(name) => {
                let mut stmts = Vec::new();
                for member in self.held(ty) {
                    let id = self.types.id(member);
                    let mut values = vec![body.slot(0, 0, member)];
                    values.extend((1..params).map(|local| body.read(local)));
                    let call = self.trait_call(name, values, returns, span);
                    stmts.push(body.return_if(body.holds(0, id), call));
                }
                // Every value in an `any` got there through a tag listed above.
                stmts.push(Stmt::Unreachable);
                stmts
            }
            Derived::Has => {
                let walk = body.each_item(1, |body, index| {
                    let item = body.item(body.read(1), index, ty);
                    let same = self.equal(body.read(0), item, span);
                    vec![body.return_if(same, body.flag(true))]
                });
                vec![walk, Stmt::Return(body.flag(false))]
            }
        };
        let locals = body.locals;
        let (captures, change, take, borrowed) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (type_params, body) = (0, stmts);
        let name = "";
        Task {
            name,
            params,
            type_params,
            returns,
            locals,
            body,
            span,
            captures,
            change,
            take,
            borrowed,
        }
    }

    fn show_body(&mut self, body: &mut Body, ty: Ty) -> Vec<Stmt> {
        const X: Local = 0;
        let span = body.span;
        match ty {
            Ty::List(item) => {
                let out = body.new_local(Ty::Text);
                let walk = body.each_item(X, |body, index| {
                    let value = body.item(body.read(X), index, *item);
                    let shown = self.shown(value, span);
                    let parts = comma_then(body, out, index, vec![shown]);
                    vec![Stmt::Set(out, body.join(parts))]
                });
                let last = body.join(vec![body.read(out), body.text("]")]);
                vec![Stmt::Set(out, body.text("[")), walk, Stmt::Return(last)]
            }
            Ty::Set(key) | Ty::Map(key, _) => {
                let (open, close) =
                    if matches!(ty, Ty::Set(_)) { ("set([", "])") } else { ("{", "}") };
                let out = body.new_local(Ty::Text);
                let count = body.new_local(Ty::Int);
                let walk = body.each_key(X, *key, |body, held, at| {
                    let key_text = self.shown(body.read(held), span);
                    let mut parts = vec![key_text];
                    if let Ty::Map(_, value) = ty {
                        let value = body.value_at(X, at, *value);
                        parts.push(body.text(": "));
                        parts.push(self.shown(value, span));
                    }
                    let parts = comma_then(body, out, count, parts);
                    vec![
                        Stmt::Set(out, body.join(parts)),
                        Stmt::Set(count, body.plus_one(body.read(count))),
                    ]
                });
                let mut stmts =
                    vec![Stmt::Set(out, body.text(open)), Stmt::Set(count, body.int(0))];
                stmts.extend(walk);
                stmts.push(Stmt::Return(body.join(vec![body.read(out), body.text(close)])));
                stmts
            }
            Ty::Union(_) | Ty::Any(_) => {
                let mut stmts = Vec::new();
                for member in self.held(ty) {
                    let id = self.types.id(member);
                    let value = self.shown(body.slot(X, 0, member), span);
                    stmts.push(body.return_if(body.holds(X, id), value));
                }
                stmts.push(Stmt::Return(body.text("")));
                stmts
            }
            Ty::Data(..) => {
                let mut stmts = Vec::new();
                for variant in 0..self.variant_count(ty) {
                    let name = self.variant_name(ty, variant);
                    let fields = self.fields_of(ty, variant);
                    let mut parts = vec![body.text(&name)];
                    for (index, (field, field_ty, slot)) in fields.iter().enumerate() {
                        let open = if index == 0 { "(" } else { ", " };
                        parts.push(body.text(&format!("{open}{field}: ")));
                        parts.push(self.shown(body.slot(X, *slot, *field_ty), span));
                    }
                    if !fields.is_empty() {
                        parts.push(body.text(")"));
                    }
                    let same = body.test(BinaryOp::Eq, body.tag(X), body.int(variant as i64));
                    stmts.push(body.return_if(same, body.join(parts)));
                }
                stmts.push(Stmt::Return(body.text("")));
                stmts
            }
            _ => unreachable!("only compound values derive `Show`"),
        }
    }

    fn equal_body(&mut self, body: &mut Body, ty: Ty) -> Vec<Stmt> {
        const A: Local = 0;
        const B: Local = 1;
        let span = body.span;
        let no = |body: &Body, same: Expr| {
            let differ = body.expr(ExprKind::Not(boxed(same)), Ty::Bool);
            body.return_if(differ, body.flag(false))
        };
        let mut stmts = Vec::new();
        match ty {
            Ty::List(item) => {
                let same = body.test(BinaryOp::Eq, body.len(body.read(A)), body.len(body.read(B)));
                stmts.push(no(body, same));
                stmts.push(body.each_item(A, |body, index| {
                    let left = body.item(body.read(A), index, *item);
                    let right = body.item(body.read(B), index, *item);
                    let same = self.equal(left, right, span);
                    vec![no(body, same)]
                }));
            }
            Ty::Set(key) | Ty::Map(key, _) => {
                let same = body.test(BinaryOp::Eq, body.len(body.read(A)), body.len(body.read(B)));
                stmts.push(no(body, same));
                stmts.extend(body.each_key(A, *key, |body, held, at| {
                    let found = body.new_local(Ty::Int);
                    let kind = ExprKind::MapFind(boxed(body.read(B)), boxed(body.read(held)));
                    let missing = body.test(BinaryOp::Lt, body.read(found), body.int(0));
                    let mut inner = vec![
                        Stmt::Set(found, body.expr(kind, Ty::Int)),
                        body.return_if(missing, body.flag(false)),
                    ];
                    if let Ty::Map(_, value) = ty {
                        let left = body.value_at(A, at, *value);
                        let same = self.equal(left, body.value_at(B, found, *value), span);
                        inner.push(no(body, same));
                    }
                    inner
                }));
            }
            Ty::Union(_) | Ty::Any(_) => {
                let same = body.test(BinaryOp::Eq, body.tag(A), body.tag(B));
                stmts.push(no(body, same));
                for member in self.held(ty) {
                    let id = self.types.id(member);
                    let (left, right) = (body.slot(A, 0, member), body.slot(B, 0, member));
                    let same = self.equal(left, right, span);
                    stmts.push(body.return_if(body.holds(A, id), same));
                }
            }
            Ty::Data(..) => {
                let same = body.test(BinaryOp::Eq, body.tag(A), body.tag(B));
                stmts.push(no(body, same));
                for variant in 0..self.variant_count(ty) {
                    let mut then = Vec::new();
                    for (_, field, slot) in self.fields_of(ty, variant) {
                        let (left, right) = (body.slot(A, slot, field), body.slot(B, slot, field));
                        let same = self.equal(left, right, span);
                        then.push(no(body, same));
                    }
                    then.push(Stmt::Return(body.flag(true)));
                    let here = body.test(BinaryOp::Eq, body.tag(A), body.int(variant as i64));
                    stmts.push(Stmt::If { branches: vec![(here, then)], otherwise: Vec::new() });
                }
            }
            _ => unreachable!("only compound values derive `Equal`"),
        }
        stmts.push(Stmt::Return(body.flag(true)));
        stmts
    }

    fn compare_body(&mut self, body: &mut Body, ty: Ty) -> Vec<Stmt> {
        const A: Local = 0;
        const B: Local = 1;
        let span = body.span;
        let order = body.new_local(Ty::Int);
        let decide = |body: &Body, value: Expr| {
            let differ = body.test(BinaryOp::Ne, body.read(order), body.int(0));
            vec![Stmt::Set(order, value), body.return_if(differ, body.read(order))]
        };
        let mut stmts = Vec::new();
        match ty {
            Ty::List(item) => {
                let short = body.new_local(Ty::Int);
                let (a_len, b_len) = (body.len(body.read(A)), body.len(body.read(B)));
                let fewer = body.test(BinaryOp::Lt, a_len.clone(), b_len.clone());
                let kind = ExprKind::If(boxed(fewer), boxed(a_len), boxed(b_len));
                stmts.push(Stmt::Set(short, body.expr(kind, Ty::Int)));
                let index = body.new_local(Ty::Int);
                let left = body.item(body.read(A), index, *item);
                let right = body.item(body.read(B), index, *item);
                let value = self.compared(left, right, span);
                let inner = decide(body, value);
                let (start, end) = (body.int(0), body.read(short));
                stmts.push(Stmt::For { local: index, start, end, inclusive: false, body: inner });
                let (a_len, b_len) = (body.len(body.read(A)), body.len(body.read(B)));
                stmts.push(Stmt::Return(self.compared(a_len, b_len, span)));
            }
            Ty::Data(..) => {
                let tags = self.compared(body.tag(A), body.tag(B), span);
                stmts.extend(decide(body, tags));
                for variant in 0..self.variant_count(ty) {
                    let mut then = Vec::new();
                    for (_, field, slot) in self.fields_of(ty, variant) {
                        let (left, right) = (body.slot(A, slot, field), body.slot(B, slot, field));
                        let value = self.compared(left, right, span);
                        then.extend(decide(body, value));
                    }
                    let here = body.test(BinaryOp::Eq, body.tag(A), body.int(variant as i64));
                    stmts.push(Stmt::If { branches: vec![(here, then)], otherwise: Vec::new() });
                }
                stmts.push(Stmt::Return(body.int(0)));
            }
            _ => unreachable!("the checker allows `Order` on no other compound value"),
        }
        stmts
    }

    /// Lengths and tags go in first, so that no two different values read
    /// the same. Only equal maps and sets may differ in order, so their entries
    /// add up, as CPython's `frozenset` joins its entries with XOR.
    fn hash_body(&mut self, body: &mut Body, ty: Ty) -> Vec<Stmt> {
        const X: Local = 0;
        const STATE: Local = 1;
        let span = body.span;
        if let Some((slot, result)) = self.method_and_result("hash", ty, span) {
            let kind = ExprKind::Call(slot, &[], vec![body.read(X)]);
            let written = body.expr(kind, result);
            return vec![Stmt::Return(self.hashed(body.read(STATE), written, span))];
        }
        let out = body.new_local(Ty::Int);
        let start = |body: &Body, value: Expr| {
            Stmt::Set(out, body.hash(HashOp::Int, body.read(STATE), value))
        };
        let mut stmts = Vec::new();
        match ty {
            Ty::List(item) => {
                stmts.push(start(body, body.len(body.read(X))));
                stmts.push(body.each_item(X, |body, index| {
                    let value = body.item(body.read(X), index, *item);
                    vec![Stmt::Set(out, self.hashed(body.read(out), value, span))]
                }));
            }
            Ty::Set(key) | Ty::Map(key, _) => {
                let sum = body.new_local(Ty::Int);
                stmts.push(Stmt::Set(sum, body.int(0)));
                stmts.extend(body.each_key(X, *key, |body, held, at| {
                    let mut entry = self.hashed(body.int(0), body.read(held), span);
                    if let Ty::Map(_, value) = ty {
                        entry = self.hashed(entry, body.value_at(X, at, *value), span);
                    }
                    vec![Stmt::Set(sum, body.hash(HashOp::Sum, body.read(sum), entry))]
                }));
                stmts.push(start(body, body.read(sum)));
            }
            Ty::Union(members) => {
                stmts.push(start(body, body.tag(X)));
                for &member in members {
                    let id = self.types.id(member);
                    let value = self.hashed(body.read(out), body.slot(X, 0, member), span);
                    stmts.push(body.return_if(body.holds(X, id), value));
                }
            }
            Ty::Data(..) => {
                stmts.push(start(body, body.tag(X)));
                for variant in 0..self.variant_count(ty) {
                    let mut then = Vec::new();
                    for (_, field, slot) in self.fields_of(ty, variant) {
                        let value = self.hashed(body.read(out), body.slot(X, slot, field), span);
                        then.push(Stmt::Set(out, value));
                    }
                    then.push(Stmt::Return(body.read(out)));
                    let here = body.test(BinaryOp::Eq, body.tag(X), body.int(variant as i64));
                    stmts.push(Stmt::If { branches: vec![(here, then)], otherwise: Vec::new() });
                }
            }
            _ => unreachable!("only compound values derive `Hash`"),
        }
        stmts.push(Stmt::Return(body.read(out)));
        stmts
    }

    /// A task of `std/json.lcd`, by name.
    fn json_task(&mut self, name: &str, args: &[Ty], span: Span) -> usize {
        let full = format!("@std.json.{name}");
        let id = self.source.iter().position(|task| task.name == full);
        self.instance(id.expect("`std/json.lcd` has this task"), args.to_vec().leak(), span)
    }

    /// The types `Value` and `JsonError`.
    fn json_types(&self) -> (Ty, Ty) {
        let wrong = self.source.iter().find(|task| task.name == "@std.json.wrong");
        let wrong = wrong.expect("`std/json.lcd` has this task");
        (wrong.locals[0], wrong.returns)
    }

    /// The task that gives the JSON text of a `ty`.
    pub fn json_writer(&mut self, ty: Ty, span: Span) -> usize {
        match ty {
            Ty::None => self.json_task("write_none", &[], span),
            Ty::Text => self.json_task("write_text", &[], span),
            Ty::Float => self.json_task("write_float", &[], span),
            Ty::F32 => self.json_task("write_f32", &[], span),
            Ty::List(item) => self.json_task("write_list", &[*item], span),
            Ty::Set(item) => self.json_task("write_set", &[*item], span),
            Ty::Map(_, value) => self.json_task("write_map", &[*value], span),
            Ty::Data(..) | Ty::Union(_) => self.derived(Derived::JsonWrite, ty, span),
            _ => self.json_task("write_plain", &[ty], span),
        }
    }

    /// The task that reads a `ty` from a JSON tree.
    pub fn json_reader(&mut self, ty: Ty, span: Span) -> usize {
        match ty {
            Ty::List(item) => self.json_task("read_list", &[*item], span),
            Ty::Set(item) => self.json_task("read_set", &[*item], span),
            Ty::Map(_, value) => self.json_task("read_map", &[*value], span),
            Ty::Data(..) | Ty::Union(_) => self.derived(Derived::JsonRead, ty, span),
            _ => self.json_task(&format!("read_{}", ty.name()), &[], span),
        }
    }

    fn json_written(&mut self, value: Expr, span: Span) -> Expr {
        let slot = self.json_writer(value.ty, span);
        Expr { kind: ExprKind::Call(slot, &[], vec![value]), ty: Ty::Text, span }
    }

    /// A `value` of a tree read as a `ty`, which gives `ty or JsonError`.
    fn json_read(&mut self, value: Expr, ty: Ty, span: Span) -> Expr {
        let (_, error) = self.json_types();
        let slot = self.json_reader(ty, span);
        let result = self.types.union([ty, error]);
        Expr { kind: ExprKind::Call(slot, &[], vec![value]), ty: result, span }
    }

    /// A call to a task of `std/json.lcd` that takes no type arguments.
    fn json_call(&mut self, name: &str, args: Vec<Expr>, ty: Ty, span: Span) -> Expr {
        let slot = self.json_task(name, &[], span);
        Expr { kind: ExprKind::Call(slot, &[], args), ty, span }
    }

    /// `value.wrong(wanted)`, as a value of `result`.
    fn json_wrong(&mut self, body: &Body, value: Expr, wanted: &str, result: Ty) -> Expr {
        let (_, error) = self.json_types();
        let wanted = body.text(wanted);
        let found = self.json_call("wrong", vec![value, wanted], error, body.span);
        self.wrap(found, result, body.span)
    }

    /// A type as an error message at run time names it: with no module.
    fn json_wanted(ty: Ty) -> String {
        match ty {
            Ty::Data(name, ..) => short_name(name).to_string(),
            Ty::Union(members) => {
                let names: Vec<String> = members.iter().map(|&m| Self::json_wanted(m)).collect();
                names.join(" or ")
            }
            _ => plain(&ty.name()),
        }
    }

    fn json_write_body(&mut self, body: &Body, ty: Ty) -> Vec<Stmt> {
        const X: Local = 0;
        let span = body.span;
        let mut stmts = Vec::new();
        let choice = matches!(ty, Ty::Data(_, id, _) if self.datas[id].choice);
        match ty {
            Ty::Union(members) => {
                for &member in members {
                    let id = self.types.id(member);
                    let text = self.json_written(body.slot(X, 0, member), span);
                    stmts.push(body.return_if(body.holds(X, id), text));
                }
            }
            _ => {
                for variant in 0..self.variant_count(ty) {
                    let fields = self.fields_of(ty, variant);
                    let name = self.variant_name(ty, variant);
                    let mut parts = Vec::new();
                    if choice && fields.is_empty() {
                        parts.push(body.text(&format!("\"{name}\"")));
                    } else {
                        let outer = if choice { format!("{{\"{name}\":") } else { String::new() };
                        parts.push(body.text(&format!("{outer}{{")));
                        for (index, (field, field_ty, slot)) in fields.iter().enumerate() {
                            let comma = if index == 0 { "" } else { "," };
                            parts.push(body.text(&format!("{comma}\"{field}\":")));
                            parts.push(self.json_written(body.slot(X, *slot, *field_ty), span));
                        }
                        parts.push(body.text(if choice { "}}" } else { "}" }));
                    }
                    let same = body.test(BinaryOp::Eq, body.tag(X), body.int(variant as i64));
                    stmts.push(body.return_if(same, body.join(parts)));
                }
            }
        }
        stmts.push(Stmt::Return(body.text("")));
        stmts
    }

    fn json_read_body(&mut self, body: &mut Body, ty: Ty, result: Ty) -> Vec<Stmt> {
        const VALUE: Local = 0;
        let span = body.span;
        let mut stmts = Vec::new();
        let datas = self.datas;
        let forms = |id: usize| datas[id].json_forms();
        match ty {
            Ty::Union(members) => {
                for &member in members {
                    let tags = json_tags(member, forms);
                    let test = ExprKind::TagIn(boxed(body.read(VALUE)), tags);
                    let test = body.expr(test, Ty::Bool);
                    let read = self.json_read(body.read(VALUE), member, span);
                    stmts.push(body.return_if(test, Expr { ty: result, ..read }));
                }
                let wrong = self.json_wrong(body, body.read(VALUE), &Self::json_wanted(ty), result);
                stmts.push(Stmt::Return(wrong));
            }
            Ty::Data(_, id, _) if datas[id].choice => {
                let (_, error) = self.json_types();
                let error_id = self.types.id(error);
                let wanted = body.text(&Self::json_wanted(ty));
                let named = self.types.union([Ty::Text, error]);
                let got = body.new_local(named);
                let name = body.new_local(Ty::Text);
                let (value, _) = self.json_types();
                let inside = body.new_local(value);
                let call =
                    self.json_call("variant_name", vec![body.read(VALUE), wanted], named, span);
                stmts.push(Stmt::Set(got, call));
                let failed = self.view(body.read(got), error, span);
                let failed = self.wrap(failed, result, span);
                stmts.push(body.return_if(body.holds(got, error_id), failed));
                stmts.push(Stmt::Set(name, self.view(body.read(got), Ty::Text, span)));
                let call = self.json_call("variant_fields", vec![body.read(VALUE)], value, span);
                stmts.push(Stmt::Set(inside, call));
                for variant in 0..self.variant_count(ty) {
                    let here = body.text(&self.variant_name(ty, variant));
                    let here = body.test(BinaryOp::Eq, body.read(name), here);
                    let step = format!(".{}", self.variant_name(ty, variant));
                    // `"Red"` alone, or `{"Circle": {...}}`, and no other form.
                    let (whole, tag) = if self.fields_of(ty, variant).is_empty() {
                        (VALUE, 4)
                    } else {
                        (inside, 6)
                    };
                    let fits = ExprKind::TagIn(boxed(body.read(whole)), vec![tag]);
                    let fits = body.expr(fits, Ty::Bool);
                    let unfit = body.expr(ExprKind::Not(boxed(fits)), Ty::Bool);
                    let wanted = Self::json_wanted(ty);
                    let wrong = self.json_wrong(body, body.read(VALUE), &wanted, result);
                    let mut then = vec![body.return_if(unfit, wrong)];
                    then.extend(self.json_read_fields(body, ty, variant, inside, &step, result));
                    stmts.push(Stmt::If { branches: vec![(here, then)], otherwise: Vec::new() });
                }
                let wrong = self.json_wrong(body, body.read(VALUE), &Self::json_wanted(ty), result);
                stmts.push(Stmt::Return(wrong));
            }
            _ => {
                let object = ExprKind::TagIn(boxed(body.read(VALUE)), json_tags(ty, forms));
                let not_object =
                    body.expr(ExprKind::Not(boxed(body.expr(object, Ty::Bool))), Ty::Bool);
                let wrong = self.json_wrong(body, body.read(VALUE), &Self::json_wanted(ty), result);
                stmts.push(body.return_if(not_object, wrong));
                stmts.extend(self.json_read_fields(body, ty, 0, VALUE, "", result));
            }
        }
        stmts
    }

    /// Reads each field of a variant from the object in `object`, and returns
    /// the value. A field with a default may be left out.
    fn json_read_fields(
        &mut self,
        body: &mut Body,
        ty: Ty,
        variant: usize,
        object: Local,
        step: &str,
        result: Ty,
    ) -> Vec<Stmt> {
        let Ty::Data(_, id, args) = ty else { unreachable!("only a `type` or `choice`") };
        let span = body.span;
        let (value_ty, error) = self.json_types();
        let error_id = self.types.id(error);
        let defaults = self.datas[id].variants[variant].defaults.clone();
        let (_, counted) = self.layout(ty, variant, usize::MAX);
        let mut stmts = Vec::new();
        let mut built = Vec::new();
        for ((name, field, slot), default) in self.fields_of(ty, variant).into_iter().zip(defaults)
        {
            let local = body.new_local(field);
            let result_of_field = self.types.union([field, error]);
            let got = body.new_local(result_of_field);
            let text_name = body.text(&name);
            let args_of = vec![body.read(object), text_name.clone()];
            let tree = self.json_call("field_value", args_of, value_ty, span);
            let read = self.json_read(tree, field, span);
            let failed = self.view(body.read(got), error, span);
            let step = body.text(&format!("{step}.{name}"));
            let failed = self.json_call("at", vec![failed, step], error, span);
            let failed = self.wrap(failed, result, span);
            let reading = vec![
                Stmt::Set(got, read),
                body.return_if(body.holds(got, error_id), failed),
                Stmt::Set(local, self.view(body.read(got), field, span)),
            ];
            if let Some(mut default) = default {
                self.finish_default(&mut default, args);
                let args_of = vec![body.read(object), text_name];
                let has = self.json_call("field_has", args_of, Ty::Bool, span);
                let otherwise = vec![Stmt::Set(local, default)];
                stmts.push(Stmt::If { branches: vec![(has, reading)], otherwise });
            } else {
                stmts.extend(reading);
            }
            built.push((slot, body.read(local)));
        }
        let kind = ExprKind::New { tag: variant, counted, fields: built };
        let made = body.expr(kind, ty);
        stmts.push(Stmt::Return(self.wrap(made, result, span)));
        stmts
    }

    fn variant_name(&self, ty: Ty, variant: usize) -> String {
        let Ty::Data(_, id, _) = ty else { unreachable!("only a `type` or `choice`") };
        self.datas[id].variants[variant].name.clone()
    }
}

/// The parts that add `parts` to `out`, after a comma unless `position` is 0.
fn comma_then(body: &Body, out: Local, position: Local, parts: Vec<Expr>) -> Vec<Expr> {
    let first = body.test(BinaryOp::Eq, body.read(position), body.int(0));
    let kind = ExprKind::If(boxed(first), boxed(body.text("")), boxed(body.text(", ")));
    let mut all = vec![body.read(out), body.expr(kind, Ty::Text)];
    all.extend(parts);
    all
}
