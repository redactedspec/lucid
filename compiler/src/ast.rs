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

/// A declared name without its module, as `resolve.rs` adds it.
pub fn short_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
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

#[derive(Clone, Copy, Debug)]
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

impl TaskSig {
    /// True for a task called with a dot, which has `self` first.
    pub fn is_method(&self) -> bool {
        self.params.first().is_some_and(|param| param.name.text == "self")
    }
}

pub struct TaskDef {
    pub public: bool,
    pub sig: TaskSig,
    /// `None` for `unsafe extern task`.
    pub body: Option<Vec<Stmt>>,
}

pub struct TypeParam {
    pub name: Name,
    pub bounds: Vec<Type>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

/// One read or write of a variable, or of a path into one.
pub struct Use<'a> {
    pub root: &'a str,
    /// The fields after `root`, up to the first index.
    pub path: Vec<&'a str>,
    pub span: Span,
    /// The whole variable is passed as `change`.
    pub change: bool,
}

/// True if a change to one path can reach the other.
pub fn overlap(a: &[&str], b: &[&str]) -> bool {
    a.iter().zip(b).all(|(x, y)| x == y)
}

impl Expr {
    /// The variable that this path starts from, with its fields, up to the
    /// first index. A slice is a new value, so it is no path.
    pub fn path(&self) -> Option<(&str, Vec<&str>)> {
        self.path_and_end().map(|(root, path, _)| (root, path))
    }

    fn path_and_end(&self) -> Option<(&str, Vec<&str>, bool)> {
        match &self.kind {
            ExprKind::Name(name) => Some((name, Vec::new(), false)),
            ExprKind::SelfValue => Some(("self", Vec::new(), false)),
            ExprKind::Field { base, name } => {
                let (root, mut path, ended) = base.path_and_end()?;
                if !ended {
                    path.push(&name.text);
                }
                Some((root, path, ended))
            }
            ExprKind::Index { base, indices } => {
                let range = |index: &Expr| {
                    let range = BinaryOp::Range;
                    matches!(index.kind, ExprKind::Binary(op, ..)
                        if op == range || op == BinaryOp::RangeInclusive)
                };
                if indices.iter().any(range) {
                    return None;
                }
                let (root, path, _) = base.path_and_end()?;
                Some((root, path, true))
            }
            _ => None,
        }
    }

    /// Adds every use of a variable in this expression. A lambda adds none,
    /// since it cannot capture a variable that can change.
    pub fn uses<'a>(&'a self, out: &mut Vec<Use<'a>>) {
        self.uses_as(false, out);
    }

    fn uses_as<'a>(&'a self, change: bool, out: &mut Vec<Use<'a>>) {
        if let Some((root, path)) = self.path() {
            let change = change && matches!(self.kind, ExprKind::Name(_) | ExprKind::SelfValue);
            out.push(Use { root, path, span: self.span, change });
            self.index_uses(out);
            return;
        }
        match &self.kind {
            ExprKind::Text(parts) => {
                for part in parts {
                    if let TextPart::Expr(value) = part {
                        value.uses(out);
                    }
                }
            }
            ExprKind::List(items) => items.iter().for_each(|item| item.uses(out)),
            ExprKind::Map(entries) => {
                for (key, value) in entries {
                    key.uses(out);
                    value.uses(out);
                }
            }
            ExprKind::Unary(_, value)
            | ExprKind::Optional(value)
            | ExprKind::Is { value, .. }
            | ExprKind::Field { base: value, .. } => value.uses(out),
            ExprKind::Binary(_, left, right) => {
                left.uses(out);
                right.uses(out);
            }
            ExprKind::If { condition, then, otherwise } => {
                condition.uses(out);
                then.uses(out);
                otherwise.uses(out);
            }
            ExprKind::Call { callee, args } => {
                match &callee.kind {
                    // A method may read all of its receiver, so its name is no field.
                    ExprKind::Field { base, .. } => base.uses(out),
                    _ => callee.uses(out),
                }
                args.iter().for_each(|arg| arg.value.uses_as(arg.change, out));
            }
            ExprKind::Index { base, indices } => {
                base.uses(out);
                indices.iter().for_each(|index| index.uses(out));
            }
            _ => {}
        }
    }

    /// The uses inside the indexes of a path.
    fn index_uses<'a>(&'a self, out: &mut Vec<Use<'a>>) {
        match &self.kind {
            ExprKind::Field { base, .. } => base.index_uses(out),
            ExprKind::Index { base, indices } => {
                base.index_uses(out);
                indices.iter().for_each(|index| index.uses(out));
            }
            _ => {}
        }
    }

    /// This value, split at a `match`, whose parts run one after another.
    fn parts<'a>(&'a self, out: &mut Vec<&'a Expr>) {
        let ExprKind::Match { subject, arms } = &self.kind else {
            out.push(self);
            return;
        };
        subject.parts(out);
        for arm in arms {
            if let ArmBody::Expr(value) = &arm.body {
                value.parts(out);
            }
        }
    }
}

impl Stmt {
    /// What this statement writes to, or calls a task on.
    pub fn target(&self) -> Option<&Expr> {
        match &self.kind {
            StmtKind::Assign { target, .. } => Some(target),
            StmtKind::Expr(Expr { kind: ExprKind::Call { callee, .. }, .. }) => {
                match &callee.kind {
                    ExprKind::Field { base, .. } => Some(base),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The values that this statement itself holds, not those of its blocks.
    /// Each group runs to its end before the next one starts.
    pub fn values(&self) -> Vec<Vec<&Expr>> {
        let mut out = Vec::new();
        match &self.kind {
            StmtKind::Let { value, .. }
            | StmtKind::While { condition: value, .. }
            | StmtKind::For { iterable: value, .. }
            | StmtKind::Return(Some(value))
            | StmtKind::Print(value)
            | StmtKind::Expect(value)
            | StmtKind::Expr(value) => value.parts(&mut out),
            StmtKind::Assign { target, value, .. } => {
                let mut group = vec![target];
                value.parts(&mut group);
                return vec![group];
            }
            StmtKind::If { branches, .. } => {
                branches.iter().for_each(|(condition, _)| condition.parts(&mut out));
            }
            _ => {}
        }
        out.into_iter().map(|value| vec![value]).collect()
    }
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
    /// `T?`, which is only a type: the checker rejects it outside type arguments.
    Optional(Box<Expr>),
    /// `any T` or a `task(...)` type, which the same rule covers.
    Type(Type),
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
    BitNot,
}

pub use crate::mir::BinaryOp;

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
    Name(Name),
    Variant {
        name: Name,
        bindings: Vec<Name>,
    },
    Typed {
        name: Name,
        ty: Type,
    },
}
