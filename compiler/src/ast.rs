//! The syntax tree. Later stages read all of it, so only spans and names are
//! kept here: meaning belongs to the resolver and type checker.

use crate::diag::Span;

pub struct File {
    pub stmts: Vec<Stmt>,
}

pub struct Name {
    pub text: String,
    pub span: Span,
}

pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

pub enum StmtKind {
    Use {
        relative: bool,
        path: Vec<Name>,
        alias: Option<Name>,
    },
    Test {
        name: Expr,
        body: Vec<Stmt>,
    },
    Task(TaskDef),
    Type(TypeDef),
    Choice(ChoiceDef),
    Trait(TraitDef),
    Let {
        mutable: bool,
        name: Name,
        ty: Option<Type>,
        value: Expr,
    },
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        otherwise: Option<Vec<Stmt>>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    For {
        change: bool,
        name: Name,
        iterable: Expr,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    Print(Expr),
    Expect(Expr),
    Unsafe(Vec<Stmt>),
    Break,
    Continue,
    /// The `...` statement.
    Placeholder,
    Expr(Expr),
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
    },
}

#[derive(Clone, Copy)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
}

pub struct TaskSig {
    pub name: Name,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub returns: Option<Type>,
}

pub struct TaskDef {
    pub public: bool,
    pub sig: TaskSig,
    /// `None` for `unsafe extern task`.
    pub body: Option<Vec<Stmt>>,
}

pub struct TypeParam {
    pub name: Name,
    pub bound: Option<Type>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ParamMode {
    Read,
    Change,
    Take,
}

pub struct Param {
    pub mode: ParamMode,
    /// The name is `self` for a receiver, whose type may be left out.
    pub name: Name,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

pub struct TypeDef {
    pub public: bool,
    pub name: Name,
    pub type_params: Vec<TypeParam>,
    pub body: TypeBody,
}

pub enum TypeBody {
    Alias(Type),
    Fields(Vec<Field>),
}

pub struct Field {
    pub name: Name,
    pub ty: Type,
    pub default: Option<Expr>,
}

pub struct ChoiceDef {
    pub public: bool,
    pub name: Name,
    pub type_params: Vec<TypeParam>,
    pub variants: Vec<Variant>,
}

pub struct Variant {
    pub name: Name,
    pub params: Vec<Param>,
}

pub struct TraitDef {
    pub public: bool,
    pub name: Name,
    pub type_params: Vec<TypeParam>,
    pub tasks: Vec<TaskSig>,
}

pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

pub enum TypeKind {
    Named { path: Vec<Name>, args: Vec<Type> },
    None,
    Task { params: Vec<Type>, returns: Option<Box<Type>> },
    Union(Vec<Type>),
    Optional(Box<Type>),
    Shared(Box<Type>),
    Weak(Box<Type>),
    Any(Box<Type>),
}

pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

pub enum ExprKind {
    Name(String),
    SelfValue,
    None,
    Bool(bool),
    /// The lexeme. The type checker reads the value and checks its range.
    Int(String),
    Float(String),
    Text(Vec<TextPart>),
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Is {
        value: Box<Expr>,
        negated: bool,
        target: IsTarget,
    },
    If {
        condition: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    Lambda {
        params: Vec<Name>,
        body: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Arg>,
    },
    Index {
        base: Box<Expr>,
        indices: Vec<Expr>,
    },
    Field {
        base: Box<Expr>,
        name: Name,
    },
    /// Only in value position: after `let`, `var`, and `return`, or as a statement.
    Match {
        subject: Box<Expr>,
        arms: Vec<Arm>,
    },
}

pub enum TextPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    Try,
    Spawn,
    Shared,
    Weak,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinaryOp {
    /// `x else y`
    Fallback,
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
    Range,
    RangeInclusive,
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Rem,
}

pub enum IsTarget {
    None,
    Empty,
    Type(Type),
}

pub struct Arg {
    pub name: Option<Name>,
    pub change: bool,
    pub value: Expr,
}

pub struct Arm {
    pub pattern: Pattern,
    pub body: ArmBody,
}

pub enum ArmBody {
    Expr(Expr),
    /// A `print` or `return` statement.
    Stmt(Box<Stmt>),
    Block(Vec<Stmt>),
}

pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

pub enum PatternKind {
    Wildcard,
    None,
    Literal(Expr),
    /// A variant with no parentheses, or a name that binds the value.
    Name(String),
    Variant {
        name: String,
        bindings: Vec<Name>,
    },
    Typed {
        name: Name,
        ty: Type,
    },
}
