//! Ownership: gives every new counted value an owner. See "Counting" in mir.rs.
//!
//! This is the naive stage of docs/memory-model.md: each new value goes to a
//! temporary local, and each store counts again. Nothing depends on a later
//! optimization, so removing one only costs speed.

use crate::mir::{Expr, ExprKind, Program, Stmt, Task, Ty, Visit};

pub fn run(program: &mut Program) {
    for task in program.tasks.iter_mut().chain([&mut program.main]) {
        let Task { locals, body, .. } = task;
        Owners { locals }.stmts(body);
    }
}

struct Owners<'a> {
    locals: &'a mut Vec<Ty>,
}

impl Visit for Owners<'_> {
    fn stmts(&mut self, stmts: &mut [Stmt]) {
        stmts.iter_mut().for_each(|stmt| self.stmt_parts(stmt));
    }

    fn expr(&mut self, value: &mut Expr) {
        self.expr_parts(value);
        let new = matches!(
            value.kind,
            ExprKind::Text(_)
                | ExprKind::ToText(_)
                | ExprKind::TextQuote(_)
                | ExprKind::Join(_)
                | ExprKind::Call(..)
                | ExprKind::TextSlice(..)
                | ExprKind::ListSlice(..)
                | ExprKind::TextCharAt(..)
                | ExprKind::TextBytes(_)
                | ExprKind::ListOf(_)
                | ExprKind::MapOf(_)
                | ExprKind::SetOf(_)
                | ExprKind::New { .. }
        );
        if new && value.ty.is_counted() {
            self.locals.push(value.ty);
            let (local, span, ty) = (self.locals.len() - 1, value.span, value.ty);
            let inner = std::mem::replace(value, Expr { kind: ExprKind::None, ty, span });
            value.kind = ExprKind::Temp(local, inner.into());
        }
    }
}
