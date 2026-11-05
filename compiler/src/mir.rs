//! MIR: typed, structured, and free of names. Both backends read only this.
//! See "Key decision" in docs/compiler.md.

use crate::ast::BinaryOp;
use crate::diag::Span;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    /// A `type` or `choice`, by its name, its index in `Program::datas`, and
    /// its type arguments. A counted record, whatever its fields. See "Runtime"
    /// in docs/compiler.md.
    Data(&'static str, usize, &'static [Ty]),
    /// `A or B`: a counted record whose one slot holds the value, and whose tag
    /// is the value's type id. Members are flat, and in type id order, so two
    /// spellings of a union are equal. Tags mean the same in every union, so a
    /// value keeps its record when it moves to a wider or narrower union.
    Union(&'static [Ty]),
    /// A type parameter, by its name and its position in the list of the task
    /// or type that declares it. `mono.rs` replaces every one.
    Param(&'static str, usize),
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
            Ty::Data(name, _, []) => name.into(),
            Ty::Data(name, _, args) => {
                let args: Vec<String> = args.iter().map(|ty| ty.name()).collect();
                format!("{name}[{}]", args.join(", "))
            }
            Ty::Param(name, _) => name.into(),
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

    /// A parameter may turn out to be counted, so the checker keeps a place
    /// for it. `mono.rs` drops what the real type does not need.
    pub fn is_counted(self) -> bool {
        self == Ty::Text || self.is_compound() || matches!(self, Ty::Param(..))
    }

    /// True if a type parameter appears anywhere in this type.
    pub fn has_param(self) -> bool {
        match self {
            Ty::Param(..) => true,
            Ty::List(item) | Ty::Set(item) => item.has_param(),
            Ty::Map(key, value) => key.has_param() || value.has_param(),
            Ty::Data(_, _, args) | Ty::Union(args) => args.iter().any(|ty| ty.has_param()),
            _ => false,
        }
    }

    /// How deep type arguments and elements nest.
    pub fn depth(self) -> usize {
        let deepest = |types: &[Ty]| types.iter().map(|ty| ty.depth()).max().unwrap_or(0);
        match self {
            Ty::List(item) | Ty::Set(item) => 1 + item.depth(),
            Ty::Map(key, value) => 1 + key.depth().max(value.depth()),
            Ty::Data(_, _, args) | Ty::Union(args) => 1 + deepest(args),
            _ => 0,
        }
    }

    /// True for a value made of other values, whose comparing and showing are
    /// derived tasks. Hashing one waits for the derived `Hash`.
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

/// The table of union members, and the operations that need it, since a union
/// keeps its members flat and in table order.
#[derive(Default)]
pub struct Types {
    /// Every member of a union. A position here is the tag of that type.
    ids: Vec<Ty>,
}

impl Types {
    /// The tag of a member of a union.
    pub fn id(&self, ty: Ty) -> usize {
        self.ids.iter().position(|&known| known == ty).expect("a union lists its members")
    }

    /// Flattens and orders members into one type. One member is that member.
    pub fn union(&mut self, members: impl IntoIterator<Item = Ty>) -> Ty {
        let mut flat = Vec::new();
        for member in members {
            if member == Ty::Error {
                return Ty::Error;
            }
            for &ty in member.members() {
                if !flat.contains(&ty) {
                    flat.push(ty);
                }
                if !self.ids.contains(&ty) {
                    self.ids.push(ty);
                }
            }
        }
        flat.sort_by_key(|&ty| self.id(ty));
        match flat.as_slice() {
            [] => Ty::Error,
            &[one] => one,
            _ => Ty::Union(flat.leak()),
        }
    }

    /// Replaces each `Ty::Param(_, i)` with `args[i]`, in one pass, so that a
    /// parameter inside an argument stays as it is.
    pub fn subst(&mut self, ty: Ty, args: &[Ty]) -> Ty {
        if !ty.has_param() {
            return ty;
        }
        match ty {
            Ty::Param(_, index) => args[index],
            Ty::List(item) => Ty::List(inner(self.subst(*item, args))),
            Ty::Set(item) => Ty::Set(inner(self.subst(*item, args))),
            Ty::Map(key, value) => {
                Ty::Map(inner(self.subst(*key, args)), inner(self.subst(*value, args)))
            }
            Ty::Data(name, id, own) => Ty::Data(name, id, self.subst_all(own, args)),
            Ty::Union(members) => {
                let members: Vec<Ty> = members.iter().map(|&m| self.subst(m, args)).collect();
                self.union(members)
            }
            _ => ty,
        }
    }

    pub fn subst_all(&mut self, types: &[Ty], args: &[Ty]) -> &'static [Ty] {
        types.iter().map(|&ty| self.subst(ty, args)).collect::<Vec<_>>().leak()
    }

    /// Fills `known` with what `ty` shows about the parameters of `pattern`.
    /// A parameter keeps its first answer, so a later value that disagrees
    /// fails as a plain type error. Only `pattern` holds variables: a
    /// parameter in `ty` belongs to the caller, and is a type like any other.
    pub fn unify(&mut self, pattern: Ty, ty: Ty, known: &mut [Option<Ty>]) {
        match (pattern, ty) {
            (_, Ty::Error) => {}
            (Ty::Param(_, index), _) => {
                known[index].get_or_insert(ty);
            }
            (Ty::List(a), Ty::List(b)) | (Ty::Set(a), Ty::Set(b)) => self.unify(*a, *b, known),
            (Ty::Map(a, b), Ty::Map(c, d)) => {
                self.unify(*a, *c, known);
                self.unify(*b, *d, known);
            }
            (Ty::Data(_, a, own), Ty::Data(_, b, args)) if a == b => {
                own.iter().zip(args).for_each(|(&p, &t)| self.unify(p, t, known));
            }
            (Ty::Union(members), _) => {
                let rest: Vec<Ty> =
                    ty.members().iter().copied().filter(|m| !members.contains(m)).collect();
                let open: Vec<Ty> = members.iter().copied().filter(|m| m.has_param()).collect();
                match open.as_slice() {
                    [Ty::Param(..)] if !rest.is_empty() => {
                        let rest = self.union(rest);
                        self.unify(open[0], rest, known);
                    }
                    [Ty::Param(..)] => {}
                    _ => {
                        for &member in &open {
                            rest.iter().for_each(|&t| self.unify(member, t, known));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// The shape of a `type` or `choice`. Field types use its own parameters.
pub struct Shape {
    pub variants: Vec<ShapeVariant>,
}

/// A `type` has one, with its own name.
pub struct ShapeVariant {
    pub name: String,
    pub fields: Vec<(String, Ty)>,
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
    pub types: Types,
    pub datas: Vec<Shape>,
    /// The tasks with a `self` parameter, by name.
    pub methods: HashMap<String, Vec<usize>>,
}

#[derive(Clone)]
pub struct Task {
    pub params: usize,
    /// How many type parameters it declares. After `mono.rs`, none.
    pub type_params: usize,
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
    /// Slot, value. Before `mono.rs`, a field position.
    FieldSet,
    /// Slot 0, value: what a narrowed union holds. `mono.rs` replaces this.
    UnionSet,
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
    /// An index into `Program::tasks`, its type arguments, and one argument
    /// per parameter. After `mono.rs` the type arguments are empty.
    Call(usize, &'static [Ty], Vec<Expr>),
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
    /// A new text: this text as a literal, with quotes and escapes.
    TextQuote(Box<Expr>),
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
    /// Before `mono.rs`, each position is a field and `counted` means nothing.
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
    /// A `type` or variant field, by its position as written. `mono.rs` makes
    /// this a `Field`, since what is counted depends on the type arguments.
    Member {
        value: Box<Expr>,
        variant: usize,
        field: usize,
    },
    /// A value as this expression's wider union. `mono.rs` makes this a `New`,
    /// or nothing if the value is a union already.
    Wrap(Box<Expr>),
    /// A union as this expression's narrower type. `mono.rs` makes this a
    /// `Field`, or nothing if the result is a union or the same type.
    View(Box<Expr>),
    /// True if a union holds one of these types. `mono.rs` makes this a `TagIn`.
    Holds(Box<Expr>, Vec<Ty>),
    /// A task from a trait bound. `mono.rs` makes this a `Call` to the task
    /// with this name for the type of the first argument.
    Method(&'static str, Vec<Expr>),
    /// Runs these statements, which only `Set` hidden locals, then gives a value.
    Seq(Vec<Stmt>, Box<Expr>),
    /// Evaluates a new value, and stores its count in this local, whose old
    /// value is released. Added by `ownership.rs`.
    Temp(Local, Box<Expr>),
}

/// Visits what a statement or expression holds. Each pass decides what to do
/// before or after it calls these.
pub trait Visit {
    fn expr(&mut self, value: &mut Expr);
    fn stmts(&mut self, stmts: &mut [Stmt]);

    fn stmt_parts(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Set(_, value) | Stmt::Return(value) | Stmt::Print(value) | Stmt::Expr(value) => {
                self.expr(value);
            }
            Stmt::Update { args, .. } => args.iter_mut().for_each(|arg| self.expr(arg)),
            Stmt::Block(body) => self.stmts(body),
            Stmt::If { branches, otherwise } => {
                for (condition, then) in branches {
                    self.expr(condition);
                    self.stmts(then);
                }
                self.stmts(otherwise);
            }
            Stmt::While { condition, body } => {
                self.expr(condition);
                self.stmts(body);
            }
            Stmt::For { start, end, body, .. } => {
                self.expr(start);
                self.expr(end);
                self.stmts(body);
            }
            Stmt::Clear(_)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::PrintText(_)
            | Stmt::Placeholder(_) => {}
        }
    }

    fn expr_parts(&mut self, value: &mut Expr) {
        match &mut value.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::None
            | ExprKind::Local(_)
            | ExprKind::Text(_) => {}
            ExprKind::Call(_, _, values)
            | ExprKind::Method(_, values)
            | ExprKind::Join(values)
            | ExprKind::ListOf(values) => {
                values.iter_mut().for_each(|value| self.expr(value));
            }
            ExprKind::New { fields, .. } => {
                fields.iter_mut().for_each(|(_, value)| self.expr(value));
            }
            ExprKind::MapOf(entries) => {
                for (key, value) in entries {
                    self.expr(key);
                    self.expr(value);
                }
            }
            ExprKind::Neg(inner)
            | ExprKind::Not(inner)
            | ExprKind::ToFloat(inner)
            | ExprKind::ToInt(inner)
            | ExprKind::ToText(inner)
            | ExprKind::TextQuote(inner)
            | ExprKind::TextLen(inner)
            | ExprKind::TextIsEmpty(inner)
            | ExprKind::TextByteLen(inner)
            | ExprKind::TextBytes(inner)
            | ExprKind::SetOf(inner)
            | ExprKind::Field(inner, _)
            | ExprKind::Tag(inner)
            | ExprKind::TagIn(inner, _)
            | ExprKind::Member { value: inner, .. }
            | ExprKind::Wrap(inner)
            | ExprKind::View(inner)
            | ExprKind::Holds(inner, _)
            | ExprKind::Temp(_, inner)
            | ExprKind::Len(inner) => self.expr(inner),
            ExprKind::Binary(_, left, right)
            | ExprKind::TextCharAt(left, right)
            | ExprKind::ListGet(left, right)
            | ExprKind::ListHas(left, right)
            | ExprKind::MapGet(left, right)
            | ExprKind::MapHas(left, right)
            | ExprKind::MapNext(left, right)
            | ExprKind::MapKey(left, right) => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::If(first, second, third)
            | ExprKind::TextSlice(first, second, third)
            | ExprKind::ListSlice(first, second, third) => {
                self.expr(first);
                self.expr(second);
                self.expr(third);
            }
            ExprKind::Seq(setup, inner) => {
                self.stmts(setup);
                self.expr(inner);
            }
        }
    }
}
