//! Ownership: gives every new counted value an owner. See "Counting" in mir.rs.
//!
//! This is the naive stage of docs/memory-model.md: each new value goes to a
//! temporary local, and each store counts again. Nothing depends on a later
//! optimization, so removing one only costs speed.

use crate::mir::{Expr, ExprKind, Program, Stmt, Task, Ty};

pub fn run(program: &mut Program) {
    for task in program.tasks.iter_mut().chain([&mut program.main]) {
        let Task { locals, body, .. } = task;
        stmts(locals, body);
    }
}

fn stmts(locals: &mut Vec<Ty>, stmts: &mut [Stmt]) {
    for stmt in stmts {
        match stmt {
            Stmt::Set(_, value) | Stmt::Return(value) | Stmt::Print(value) | Stmt::Expr(value) => {
                expr(locals, value);
            }
            Stmt::If { branches, otherwise } => {
                for (condition, then) in branches {
                    expr(locals, condition);
                    self::stmts(locals, then);
                }
                self::stmts(locals, otherwise);
            }
            Stmt::While { condition, body } => {
                expr(locals, condition);
                self::stmts(locals, body);
            }
            Stmt::For { start, end, body, .. } => {
                expr(locals, start);
                expr(locals, end);
                self::stmts(locals, body);
            }
            Stmt::Break | Stmt::Continue | Stmt::PrintText(_) | Stmt::Placeholder(_) => {}
        }
    }
}

fn expr(locals: &mut Vec<Ty>, value: &mut Expr) {
    match &mut value.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::None
        | ExprKind::Local(_)
        | ExprKind::Text(_) => {}
        ExprKind::Call(_, values) | ExprKind::Join(values) => {
            values.iter_mut().for_each(|value| expr(locals, value));
        }
        ExprKind::Neg(inner)
        | ExprKind::Not(inner)
        | ExprKind::ToFloat(inner)
        | ExprKind::ToInt(inner)
        | ExprKind::ToText(inner)
        | ExprKind::TextLen(inner)
        | ExprKind::TextIsEmpty(inner) => expr(locals, inner),
        ExprKind::Binary(_, left, right) => {
            expr(locals, left);
            expr(locals, right);
        }
        ExprKind::If(condition, then, otherwise) => {
            expr(locals, condition);
            expr(locals, then);
            expr(locals, otherwise);
        }
        ExprKind::Temp(..) => unreachable!("only this pass adds a `Temp`"),
    }
    let new = matches!(
        value.kind,
        ExprKind::Text(_) | ExprKind::ToText(_) | ExprKind::Join(_) | ExprKind::Call(..)
    );
    if new && value.ty.is_counted() {
        locals.push(value.ty);
        let (local, span, ty) = (locals.len() - 1, value.span, value.ty);
        let inner = std::mem::replace(value, Expr { kind: ExprKind::None, ty, span });
        value.kind = ExprKind::Temp(local, inner.into());
    }
}
