//! MIR: typed, structured, and free of names. Both backends read only this.
//! See "Key decision" in docs/compiler.md.

use crate::ast::BinaryOp;
use crate::diag::Span;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    None,
    /// A counted heap value. See "Counting" below.
    Text,
    /// Already reported. Hides follow-on errors, and never reaches a backend.
    Error,
}

impl Ty {
    pub fn name(self) -> &'static str {
        match self {
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Bool => "bool",
            Ty::None => "none",
            Ty::Text => "text",
            Ty::Error => "unknown",
        }
    }

    pub fn is_counted(self) -> bool {
        self == Ty::Text
    }
}

/// An index into `Task::locals`. Parameters come first.
pub type Local = usize;

// Counting: a counted expression yields a borrowed reference, which stays valid
// for its whole statement because a local or `Temp` owns it. Each counted local
// owns one count, and starts empty. `Set` retains the new value and releases the
// old one. A parameter is borrowed from the caller, and is never `Set`. `Return`
// retains its value, then releases every counted local that is not a parameter.

pub struct Program {
    pub tasks: Vec<Task>,
    /// The top-level statements of the entry file.
    pub main: Task,
}

pub struct Task {
    pub params: usize,
    pub returns: Ty,
    pub locals: Vec<Ty>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

pub enum Stmt {
    Set(Local, Expr),
    /// Branches stay in one flat list, so an `else if` chain of any length
    /// costs no recursion. MAX_DEPTH does not limit its length.
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        otherwise: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    For {
        local: Local,
        start: Expr,
        end: Expr,
        inclusive: bool,
        body: Vec<Stmt>,
    },
    Return(Expr),
    Break,
    Continue,
    Print(Expr),
    PrintText(String),
    Placeholder(Span),
    Expr(Expr),
}

#[derive(Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    None,
    Local(Local),
    /// An index into `Program::tasks`, and one argument per parameter.
    Call(usize, Vec<Expr>),
    /// Float only. Integer negation is `0 - x`, which checks for overflow.
    Neg(Box<Expr>),
    Not(Box<Expr>),
    /// Both sides have one type. Only arithmetic and comparison operators,
    /// and `Div` only on floats.
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    /// Also expresses `and` and `or`, which skip their right side.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    ToFloat(Box<Expr>),
    ToInt(Box<Expr>),
    /// A new text.
    Text(String),
    /// A new text from an `int`, `float`, `bool`, or `none`.
    ToText(Box<Expr>),
    /// A new text from these text values, in order.
    Join(Vec<Expr>),
    /// The number of Unicode characters.
    TextLen(Box<Expr>),
    TextIsEmpty(Box<Expr>),
    /// Evaluates a new value, and stores its count in this local, whose old
    /// value is released. Added by `ownership.rs`.
    Temp(Local, Box<Expr>),
}
