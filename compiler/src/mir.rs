//! MIR: typed, structured, and free of names. Both backends read only this.
//! See "Key decision" in docs/compiler.md.

use crate::ast::BinaryOp;
use crate::diag::Span;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    /// Held in an `int`-sized value, always from 0 to 255.
    Byte,
    None,
    /// A counted heap value, as are the three below. See "Counting".
    Text,
    List(&'static Ty),
    Map(&'static Ty, &'static Ty),
    Set(&'static Ty),
    /// A `type` or `choice`, by its name and its index in the checker's table.
    /// A counted record, whatever its fields. See "Runtime" in docs/compiler.md.
    Data(&'static str, usize),
    /// `A or B`: a counted record whose one slot holds the value, and whose tag
    /// is the value's type id. Members are flat, and in type id order, so two
    /// spellings of a union are equal. Tags mean the same in every union, so a
    /// value keeps its record when it moves to a wider or narrower union.
    Union(&'static [Ty]),
    /// Already reported. Hides follow-on errors, and never reaches a backend.
    Error,
}

/// Leaked, to keep `Ty` a small `Copy` value. A compile ends with the process,
/// and each type written in the source costs a few bytes.
pub fn inner(ty: Ty) -> &'static Ty {
    Box::leak(Box::new(ty))
}

impl Ty {
    pub fn name(self) -> String {
        match self {
            Ty::Int => "int".into(),
            Ty::Float => "float".into(),
            Ty::Bool => "bool".into(),
            Ty::Byte => "byte".into(),
            Ty::None => "none".into(),
            Ty::Text => "text".into(),
            Ty::List(item) => format!("list[{}]", item.name()),
            Ty::Map(key, value) => format!("map[{}, {}]", key.name(), value.name()),
            Ty::Set(item) => format!("set[{}]", item.name()),
            Ty::Data(name, _) => name.into(),
            Ty::Union(members) => {
                let mut names: Vec<String> =
                    members.iter().filter(|&&ty| ty != Ty::None).map(|ty| ty.name()).collect();
                if names.len() == members.len() {
                    return names.join(" or ");
                }
                if let [one] = names.as_slice() {
                    return format!("{one}?");
                }
                names.push("none".into());
                names.join(" or ")
            }
            Ty::Error => "unknown".into(),
        }
    }

    pub fn is_counted(self) -> bool {
        self == Ty::Text || self.is_compound()
    }

    /// True for a value made of other values. Comparing, showing, and hashing
    /// one waits for derived traits.
    pub fn is_compound(self) -> bool {
        self.is_collection() || matches!(self, Ty::Data(..) | Ty::Union(_))
    }

    pub fn is_collection(self) -> bool {
        matches!(self, Ty::List(_) | Ty::Map(..) | Ty::Set(_))
    }

    /// The members of a union, or the type itself.
    pub fn members(&self) -> &[Ty] {
        match self {
            Ty::Union(members) => members,
            ty => std::slice::from_ref(ty),
        }
    }

    /// True for a value that the runtime can hash and compare as a key.
    pub fn is_key(self) -> bool {
        matches!(self, Ty::Int | Ty::Bool | Ty::Byte | Ty::Text)
    }
}

/// An index into `Task::locals`. Parameters come first.
pub type Local = usize;

// Counting: a counted expression yields a borrowed reference, which stays valid
// for its whole statement because a local or `Temp` owns it. Each counted local
// owns one count, and starts empty. `Set` retains the new value and releases the
// old one. A parameter is borrowed from the caller, and is never `Set`. `Return`
// retains its value, then releases every counted local that is not a parameter.
//
// A list, map, set, or record in a local changes only through `Stmt::Update`, which gives
// the runtime that local's count, and stores the value that comes back. So a
// counted expression stays valid across its statement. Its elements stay valid
// too, since their container cannot change until the statement ends.
//
// A write through a path takes the element out of its slot, which holds null
// until the write ends. The checker reads every index and value before that,
// so no expression sees the null. A trap on the way ends the process.

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

#[derive(Clone)]
pub enum Stmt {
    Set(Local, Expr),
    /// Changes the list, map, set, or record in this local, which may then move.
    /// Traps at `span` if a list index is out of bounds.
    Update {
        local: Local,
        op: Update,
        args: Vec<Expr>,
        span: Span,
    },
    /// Releases a hidden counted local, which is then empty.
    Clear(Local),
    /// Statements that one source statement lowers to.
    Block(Vec<Stmt>),
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

/// The arguments that `Stmt::Update` holds for each operation.
#[derive(Clone, Copy, PartialEq)]
pub enum Update {
    /// Value.
    ListPush,
    /// Index, value.
    ListSet,
    /// Index, index.
    ListSwap,
    /// Key, value. A set uses this with a `none` value.
    MapSet,
    /// Key.
    MapRemove,
    /// Slot, value.
    FieldSet,
    /// Nothing. Drops the last value of a list that is not empty.
    ListPop,
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
    /// A new text from these characters, up to but not including `end`.
    /// Traps unless `0 <= start <= end <= len`.
    TextSlice(Box<Expr>, Box<Expr>, Box<Expr>),
    /// The number of UTF-8 bytes.
    TextByteLen(Box<Expr>),
    /// A new one-character text, from this byte offset of a text. The offset
    /// is below the byte length, and starts a character.
    TextCharAt(Box<Expr>, Box<Expr>),
    /// A new list of `int`-sized bytes.
    TextBytes(Box<Expr>),
    /// A new list.
    ListOf(Vec<Expr>),
    /// A new list from these elements, up to but not including `end`. Traps
    /// unless `0 <= start <= end <= len`.
    ListSlice(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A new map. A later repeat of a key wins.
    MapOf(Vec<(Expr, Expr)>),
    /// A new set from a list.
    SetOf(Box<Expr>),
    /// The length of a list, map, or set.
    Len(Box<Expr>),
    /// Traps if the index is out of bounds.
    ListGet(Box<Expr>, Box<Expr>),
    /// True if a list holds a value, with `Ty::is_key` true for its type. The
    /// value comes first, as on the page.
    ListHas(Box<Expr>, Box<Expr>),
    /// Traps if the key is missing.
    MapGet(Box<Expr>, Box<Expr>),
    /// True if a map or set holds a key. The key comes first, as on the page.
    MapHas(Box<Expr>, Box<Expr>),
    /// The first entry position at or after this one, or -1 at the end.
    MapNext(Box<Expr>, Box<Expr>),
    /// The key at a position from `MapNext`.
    MapKey(Box<Expr>, Box<Expr>),
    /// A new record. Values run in this order, each going to its slot, and
    /// together they fill every slot. Slots below `counted` hold counted values.
    New {
        tag: usize,
        counted: usize,
        fields: Vec<(usize, Expr)>,
    },
    /// The value in a slot of a record.
    Field(Box<Expr>, usize),
    /// The variant of a record.
    Tag(Box<Expr>),
    /// True if a union's tag is one of these.
    TagIn(Box<Expr>, Vec<usize>),
    /// Runs these statements, which only `Set` hidden locals, then gives a value.
    Seq(Vec<Stmt>, Box<Expr>),
    /// Evaluates a new value, and stores its count in this local, whose old
    /// value is released. Added by `ownership.rs`.
    Temp(Local, Box<Expr>),
}
