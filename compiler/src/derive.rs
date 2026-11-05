//! Derived tasks: showing, comparing, and searching values made of other values.
//! Each is ordinary MIR for one concrete type, so `ownership.rs` and the
//! backends need nothing new. See decision 69.
//!
//! ponytail: `Show` joins text one part at a time, so it is quadratic in the
//! length of its output. A text builder can follow if a program needs one.

use crate::ast::BinaryOp;
use crate::diag::Span;
use crate::mir::{Expr, ExprKind, Local, Stmt, Task, Ty};
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

    /// Visits the keys of a map or set in `map`. The body may return.
    fn each_key(
        &mut self,
        map: Local,
        key: Ty,
        body: impl FnOnce(&mut Self, Local) -> Vec<Stmt>,
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
        let mut inner = vec![Stmt::Set(held, at), Stmt::Set(position, after)];
        inner.extend(body(self, held));
        let condition = self.test(BinaryOp::Ge, self.read(position), self.int(0));
        vec![Stmt::Set(position, first), Stmt::While { condition, body: inner }]
    }
}

impl Mono<'_> {
    /// The text of a value inside a larger value, where text has quotes.
    fn shown(&mut self, value: Expr, span: Span) -> Expr {
        let ty = value.ty;
        match ty {
            Ty::Text => Expr { kind: ExprKind::TextQuote(boxed(value)), ty, span },
            _ if ty.is_compound() => {
                self.written_or(Derived::Show, "show", vec![value], Ty::Text, span)
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
        if !value.ty.members().contains(&Ty::Text) {
            return self.shown(value, span);
        }
        let held = Expr { kind: ExprKind::Field(boxed(value.clone()), 0), ty: Ty::Text, span };
        let id = self.types.id(Ty::Text);
        let is_text =
            Expr { kind: ExprKind::TagIn(boxed(value.clone()), vec![id]), ty: Ty::Bool, span };
        let shown = self.shown(value, span);
        let kind = ExprKind::If(boxed(is_text), boxed(held), boxed(shown));
        Expr { kind, ty: Ty::Text, span }
    }

    /// `left == right` for any type.
    fn equal(&mut self, left: Expr, right: Expr, span: Span) -> Expr {
        match left.ty {
            Ty::None => Expr { kind: ExprKind::Bool(true), ty: Ty::Bool, span },
            ty if ty.is_compound() => {
                self.written_or(Derived::Equal, "equal", vec![left, right], Ty::Bool, span)
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
            return self.written_or(Derived::Compare, "compare", vec![left, right], Ty::Int, span);
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

    /// `value in list` for a type that the runtime cannot compare.
    pub fn list_has(&mut self, value: Expr, list: Expr, span: Span) -> Expr {
        let slot = self.derived(Derived::Has, value.ty, span);
        Expr { kind: ExprKind::Call(slot, &[], vec![value, list]), ty: Ty::Bool, span }
    }

    /// A hand-written task for the type of the first argument, or else the
    /// derived one.
    fn written_or(
        &mut self,
        kind: Derived,
        name: &str,
        args: Vec<Expr>,
        returns: Ty,
        span: Span,
    ) -> Expr {
        let ty = args[0].ty;
        let slot = match self.method(name, ty, span) {
            Some(slot) => slot,
            None => self.derived(kind, ty, span),
        };
        Expr { kind: ExprKind::Call(slot, &[], args), ty: returns, span }
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
            Derived::Show => (vec![ty], Ty::Text),
            Derived::Equal => (vec![ty, ty], Ty::Bool),
            Derived::Compare => (vec![ty, ty], Ty::Int),
            Derived::Has => (vec![ty, Ty::List(crate::mir::inner(ty))], Ty::Bool),
        };
        let params = locals.len();
        let mut body = Body { locals, span };
        let stmts = match kind {
            Derived::Show => self.show_body(&mut body, ty),
            Derived::Equal => self.equal_body(&mut body, ty),
            Derived::Compare => self.compare_body(&mut body, ty),
            Derived::Has => {
                let walk = body.each_item(1, |body, index| {
                    let item = body.item(body.read(1), index, ty);
                    let same = self.equal(body.read(0), item, span);
                    vec![body.return_if(same, body.flag(true))]
                });
                vec![walk, Stmt::Return(body.flag(false))]
            }
        };
        Task { params, type_params: 0, returns, locals: body.locals, body: stmts, span }
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
                let walk = body.each_key(X, *key, |body, held| {
                    let key_text = self.shown(body.read(held), span);
                    let mut parts = vec![key_text];
                    if let Ty::Map(key_ty, value) = ty {
                        let _ = key_ty;
                        let kind = ExprKind::MapGet(boxed(body.read(X)), boxed(body.read(held)));
                        let value = body.expr(kind, *value);
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
            Ty::Union(members) => {
                let mut stmts = Vec::with_capacity(members.len());
                for &member in members {
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
                stmts.extend(body.each_key(A, *key, |body, held| {
                    let (a, b) = (body.read(A), body.read(B));
                    let has = ExprKind::MapHas(boxed(body.read(held)), boxed(b.clone()));
                    let mut inner = vec![no(body, body.expr(has, Ty::Bool))];
                    if let Ty::Map(_, value) = ty {
                        let get = |map: Expr| {
                            let kind = ExprKind::MapGet(boxed(map), boxed(body.read(held)));
                            body.expr(kind, *value)
                        };
                        let same = self.equal(get(a), get(b), span);
                        inner.push(no(body, same));
                    }
                    inner
                }));
            }
            Ty::Union(members) => {
                let same = body.test(BinaryOp::Eq, body.tag(A), body.tag(B));
                stmts.push(no(body, same));
                for &member in members {
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
