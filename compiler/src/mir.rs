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
    /// `u8`. Every integer is held in an `int`-sized value, extended from its
    /// own width, so a comparison or store needs no special case. `U64` shares
    /// its bits with `int`, and is told apart by its type.
    Byte,
    I8,
    I16,
    I32,
    U16,
    U32,
    U64,
    F32,
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
    /// `any Trait`: the same record as a union, so a union or a member becomes
    /// one with no new code. `mono.rs` lists the types that each one holds.
    Any(&'static str),
    /// `task(A) -> B`: a counted record, whose tag is the address of its code
    /// and whose slots hold its captures. Its code takes the record first.
    /// See "Task values" in docs/compiler.md.
    Task(&'static [Ty], &'static Ty),
    /// A type parameter, by its name and its position in the list of the task
    /// or type that declares it. `mono.rs` replaces every one.
    Param(&'static str, usize),
    /// Already reported. Hides follow-on errors, and never reaches a backend.
    Error,
}

/// See `Ty::too_large`.
pub const MAX_TYPE_PARTS: usize = 256;

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
            Ty::I8 => "i8".into(),
            Ty::I16 => "i16".into(),
            Ty::I32 => "i32".into(),
            Ty::U16 => "u16".into(),
            Ty::U32 => "u32".into(),
            Ty::U64 => "u64".into(),
            Ty::F32 => "f32".into(),
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
            Ty::Any(name) => format!("any {name}"),
            Ty::Task(params, returns) => {
                let params: Vec<String> = params.iter().map(|ty| ty.name()).collect();
                let params = params.join(", ");
                match returns {
                    Ty::None => format!("task({params})"),
                    _ => format!("task({params}) -> {}", returns.name()),
                }
            }
            Ty::Union(members) => {
                // A task's return type would otherwise take in what follows it.
                let member = |ty: &Ty| match ty {
                    Ty::Task(..) => format!("({})", ty.name()),
                    _ => ty.name(),
                };
                let mut names: Vec<String> =
                    members.iter().filter(|&&ty| ty != Ty::None).map(member).collect();
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
        self == Ty::Text || self.is_compound() || matches!(self, Ty::Param(..) | Ty::Task(..))
    }

    /// True if `part` appears anywhere in this type.
    pub fn mentions(self, part: Ty) -> bool {
        self == part
            || match self {
                Ty::List(item) | Ty::Set(item) => item.mentions(part),
                Ty::Map(key, value) => key.mentions(part) || value.mentions(part),
                Ty::Data(_, _, args) | Ty::Union(args) => args.iter().any(|a| a.mentions(part)),
                Ty::Task(params, returns) => {
                    returns.mentions(part) || params.iter().any(|a| a.mentions(part))
                }
                _ => false,
            }
    }

    /// True if a type parameter appears anywhere in this type.
    pub fn has_param(self) -> bool {
        match self {
            Ty::Param(..) => true,
            Ty::List(item) | Ty::Set(item) => item.has_param(),
            Ty::Map(key, value) => key.has_param() || value.has_param(),
            Ty::Data(_, _, args) | Ty::Union(args) => args.iter().any(|ty| ty.has_param()),
            Ty::Task(params, returns) => {
                returns.has_param() || params.iter().any(|ty| ty.has_param())
            }
            _ => false,
        }
    }

    /// True if this type has over `MAX_TYPE_PARTS` parts. Comparing and hashing
    /// a `Ty` walks it as a tree, and `Pair[T, T]` doubles at each level, so
    /// a limit on depth alone would let one type take exponential time.
    pub fn too_large(self) -> bool {
        fn spend(ty: Ty, left: &mut usize) -> bool {
            if *left == 0 {
                return false;
            }
            *left -= 1;
            match ty {
                Ty::List(item) | Ty::Set(item) => spend(*item, left),
                Ty::Map(key, value) => spend(*key, left) && spend(*value, left),
                Ty::Data(_, _, args) | Ty::Union(args) => args.iter().all(|&a| spend(a, left)),
                Ty::Task(params, returns) => {
                    params.iter().all(|&a| spend(a, left)) && spend(*returns, left)
                }
                _ => true,
            }
        }
        !spend(self, &mut MAX_TYPE_PARTS.clone())
    }

    /// True if one value could have both types, with the parameters of each
    /// free. It ignores a parameter used twice, so `Pair[T, T]` counts as
    /// overlapping `Pair[int, text]`. A false yes only asks for another name.
    pub fn may_overlap(self, other: Ty) -> bool {
        match (self, other) {
            (Ty::Param(..), _) | (_, Ty::Param(..)) => true,
            (Ty::Union(_), Ty::Union(_)) => self == other || self.has_param() || other.has_param(),
            // `T?` is not a union for `T = none`.
            (Ty::Union(members), single) | (single, Ty::Union(members)) => {
                members.iter().any(|m| matches!(m, Ty::Param(..)) && !single.has_param())
                    || (single.has_param() && members.iter().any(|m| m.may_overlap(single)))
            }
            (Ty::List(a), Ty::List(b)) | (Ty::Set(a), Ty::Set(b)) => a.may_overlap(*b),
            (Ty::Map(a, b), Ty::Map(c, d)) => a.may_overlap(*c) && b.may_overlap(*d),
            (Ty::Data(_, a, own), Ty::Data(_, b, args)) => {
                a == b && own.iter().zip(args).all(|(x, &y)| x.may_overlap(y))
            }
            (Ty::Task(a, b), Ty::Task(c, d)) => {
                a.len() == c.len()
                    && b.may_overlap(*d)
                    && a.iter().zip(c).all(|(x, &y)| x.may_overlap(y))
            }
            _ => self == other,
        }
    }

    /// True for a value made of other values, whose comparing, showing, and
    /// hashing are derived tasks.
    pub fn is_compound(self) -> bool {
        self.is_collection() || matches!(self, Ty::Data(..) | Ty::Union(_) | Ty::Any(_))
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
        self.is_integer() || matches!(self, Ty::Bool | Ty::Text)
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            Ty::Int | Ty::Byte | Ty::I8 | Ty::I16 | Ty::I32 | Ty::U16 | Ty::U32 | Ty::U64
        )
    }

    pub fn is_float(self) -> bool {
        matches!(self, Ty::Float | Ty::F32)
    }

    pub fn is_number(self) -> bool {
        self.is_integer() || self.is_float()
    }

    /// The lowest and highest value of an integer type.
    pub fn int_range(self) -> (i128, i128) {
        match self {
            Ty::Byte => (0, u8::MAX.into()),
            Ty::I8 => (i8::MIN.into(), i8::MAX.into()),
            Ty::I16 => (i16::MIN.into(), i16::MAX.into()),
            Ty::I32 => (i32::MIN.into(), i32::MAX.into()),
            Ty::U16 => (0, u16::MAX.into()),
            Ty::U32 => (0, u32::MAX.into()),
            Ty::U64 => (0, u64::MAX.into()),
            _ => (i64::MIN.into(), i64::MAX.into()),
        }
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

    /// The tag of a type that an `any` holds, which may be in no union.
    pub fn tag(&mut self, ty: Ty) -> usize {
        if !self.ids.contains(&ty) {
            self.ids.push(ty);
        }
        self.id(ty)
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
            Ty::Task(params, returns) => {
                Ty::Task(self.subst_all(params, args), inner(self.subst(*returns, args)))
            }
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

    /// A type with what is known so far of the type parameters in it, or
    /// `None` if one is still open.
    pub fn partial(&mut self, pattern: Ty, known: &[Option<Ty>]) -> Option<Ty> {
        let args: Option<Vec<Ty>> = known.iter().copied().collect();
        if let Some(args) = args {
            return Some(self.subst(pattern, &args));
        }
        if !pattern.has_param() {
            return Some(pattern);
        }
        let open = Ty::Param("", usize::MAX);
        let args: Vec<Ty> = known.iter().map(|ty| ty.unwrap_or(open)).collect();
        let found = self.subst(pattern, &args);
        (!found.mentions(open)).then_some(found)
    }

    /// What `ty` shows about `count` type parameters, if it fits a receiver of
    /// type `pattern`. Both stages use this, so they pick the same task.
    pub fn fit(&mut self, pattern: Ty, ty: Ty, count: usize) -> Option<Vec<Option<Ty>>> {
        let mut known = vec![None; count];
        self.unify(pattern, ty, &mut known);
        (self.partial(pattern, &known) == Some(ty)).then_some(known)
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
            (Ty::Task(a, b), Ty::Task(c, d)) if a.len() == c.len() => {
                a.iter().zip(c).for_each(|(&p, &t)| self.unify(p, t, known));
                self.unify(*b, *d, known);
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

// Counting: most counted expressions yield a borrowed reference, which stays
// valid for its whole statement because a local or `Temp` owns it. Each counted
// local owns one count, and starts empty. A parameter is borrowed from the
// caller, and is never `Set`, except a `change` parameter, which holds the
// caller's count for the length of the call. A `take` parameter of a task in
// `Task::take` arrives with a count, which the first statement moves to a local.
//
// What keeps a value receives one count: `Set`, `Return`, a field of `New` or
// `Closure`, an element of `ListOf` or `MapOf`, an argument of `Stmt::Update`
// except a `MapRemove` key, and an argument to a `take` parameter. After
// `ownership.rs` each of those holds a new value, a `Move`, or a `Retain`, and a
// backend adds no count of its own. `Set` releases the old value after it reads
// the new one. `Return` runs its value, then releases every counted local that
// is not a parameter. A local holds null after `Move` and `Clear`. A null is
// safe to release, and `ownership.rs` proves that nothing reads one.
//
// A list, map, set, or record in a local changes only through `Stmt::Update`, which gives
// the runtime that local's count, and stores the value that comes back. So a
// counted expression stays valid across its statement. Its elements stay valid
// too, since their container cannot change until the statement ends.
//
// A write through a path takes the element out of its slot, which holds null
// until the write ends, and the count moves with it. The checker reads every
// index and value before that, so no expression sees the null. A trap on the
// way ends the process.
//
// A statement that passes a local by `change` may replace its value, so a
// borrow of that local would not stay valid. The exclusivity checks in the
// checker reject any such borrow. See "Parameters" in docs/compiler.md.
//
// A `Return` inside an expression is the one way that a statement can stop
// half done. `ownership.rs` first moves each sibling of such an expression
// that a keeper holds to a local, so no count is outside a local at that point.

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
    /// What a lambda or nested task holds, in the record that is its first
    /// parameter. Any other task has none.
    pub captures: Vec<Ty>,
    /// The parameters passed by `change`, in order.
    pub change: Vec<Local>,
    /// The `take` parameters whose caller gives a count. A task that may run
    /// through a task value or the runtime has a borrowed one instead, and a
    /// first statement that retains it, since those callers pass no count.
    pub take: Vec<Local>,
    /// Counted locals that hold no count, in order. Only `Stmt::Bind` sets one.
    /// Added by `ownership.rs`.
    pub borrowed: Vec<Local>,
}

#[derive(Clone)]
pub enum Stmt {
    Set(Local, Expr),
    /// Holds a borrow with no count, which no exit releases. `ownership.rs`
    /// adds this for what stays valid for the whole task. `check.rs` adds this
    /// before `Update::Take`, which gives the local the slot's own count until
    /// `Update::Put` hands it back.
    Bind(Local, Expr),
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
    /// Prints this text to stderr, and ends the program with exit code 1.
    Exit(Expr),
    Break,
    Continue,
    Print(Expr),
    PrintText(String),
    Placeholder(Span),
    /// Traps with this message.
    Trap(&'static str, Span),
    /// A path that no value can take. Reaching it is a compiler bug.
    Unreachable,
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
    /// Index or slot, of a list or record. Makes this value unique, and leaves
    /// the slot null, with its count going to whoever read it first. The read
    /// proved the index, and nothing ran since.
    Take,
    /// Index or slot, value. Fills a slot that `Take` left, with no count
    /// change. A value in a slot that `Take` did not leave would leak.
    Put,
}

/// The tasks that hash a compound key and compare two, which a map keeps and
/// the runtime calls. `mono.rs` fills this in. Any other key needs none.
pub type KeyTasks = Option<(usize, usize)>;

#[derive(Clone, Copy)]
pub enum HashOp {
    /// An `int`-sized value of any type.
    Int,
    Text,
    /// Another hash, in a way that does not depend on order.
    Sum,
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
    /// A local passed to a `change` parameter. Only a `Call` holds this, as an
    /// argument. The local holds what the task left there when the call ends.
    Change(Local),
    /// An index into `Program::tasks`, its type arguments, and one argument
    /// per parameter. After `mono.rs` the type arguments are empty.
    Call(usize, &'static [Ty], Vec<Expr>),
    /// Calls a task value with one argument per parameter.
    CallValue(Box<Expr>, Vec<Expr>),
    /// A new task value: a record whose tag is the address of a task from
    /// `Program::tasks`, with these type arguments. Values run in this order,
    /// each going to its slot, and slots below `counted` hold counted values.
    /// Before `mono.rs`, each position is a capture and `counted` means nothing.
    Closure {
        task: usize,
        args: &'static [Ty],
        counted: usize,
        fields: Vec<(usize, Expr)>,
    },
    /// What this task captured, by position. `mono.rs` makes this a `Field` of
    /// its first parameter.
    Capture(usize),
    /// Float only. Integer negation is `0 - x`, which checks for overflow.
    Neg(Box<Expr>),
    /// Float only, and NaN below zero, as IEEE 754 says.
    Sqrt(Box<Expr>),
    Not(Box<Expr>),
    /// Both sides have one type. Only arithmetic and comparison operators,
    /// and `Div` only on floats.
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    /// Also expresses `and` and `or`, which skip their right side.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A number as this expression's number type. Traps if an integer result
    /// cannot hold it. A `float` only rounds, as IEEE 754 says.
    Convert(Box<Expr>),
    /// `Add`, `Sub`, or `Mul` on two integers of one type, which wraps.
    Wrapping(BinaryOp, Box<Expr>, Box<Expr>),
    /// A text in static data.
    Text(String),
    /// A new text from an `int`, `float`, `bool`, or `none`.
    ToText(Box<Expr>),
    /// A new text from these text values, in order. If the first is a `Move`,
    /// the text may grow in place.
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
    MapOf(Vec<(Expr, Expr)>, KeyTasks),
    /// A new set from a list.
    SetOf(Box<Expr>, KeyTasks),
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
    /// The value at a position from `MapNext` or `MapFind`. Reading by position
    /// runs no key task, so a derived task stays linear in nested maps.
    MapValue(Box<Expr>, Box<Expr>),
    /// The position of a key, or -1. The map comes first.
    MapFind(Box<Expr>, Box<Expr>),
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
    /// A task from a trait. `mono.rs` makes this a `Call` to the task with this
    /// name for the type of the first argument, which may look at what an
    /// `any` holds.
    Method(&'static str, Vec<Expr>),
    /// Adds the second value to the `int` hash state in the first.
    Hash(HashOp, Box<Expr>, Box<Expr>),
    /// Runs these statements, then gives a value. They only touch hidden
    /// locals, or a place that a `change` argument took out and puts back.
    /// A `try` may also return from here, or exit.
    Seq(Vec<Stmt>, Box<Expr>),
    /// Evaluates a new value, and stores its count in this local, whose old
    /// value is released. Added by `ownership.rs`.
    Temp(Local, Box<Expr>),
    /// Gives this local's count away, and leaves it null. Added by `ownership.rs`.
    Move(Local),
    /// Adds a count to a borrowed value. Added by `ownership.rs`.
    Retain(Box<Expr>),
}

/// Visits what a statement or expression holds. Each pass decides what to do
/// before or after it calls these.
pub trait Visit {
    fn expr(&mut self, value: &mut Expr);
    fn stmts(&mut self, stmts: &mut [Stmt]);

    fn stmt_parts(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Set(_, value)
            | Stmt::Bind(_, value)
            | Stmt::Return(value)
            | Stmt::Exit(value)
            | Stmt::Print(value)
            | Stmt::Expr(value) => self.expr(value),
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
            | Stmt::Placeholder(_)
            | Stmt::Trap(..)
            | Stmt::Unreachable => {}
        }
    }

    fn expr_parts(&mut self, value: &mut Expr) {
        match &mut value.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::None
            | ExprKind::Local(_)
            | ExprKind::Change(_)
            | ExprKind::Capture(_)
            | ExprKind::Move(_)
            | ExprKind::Text(_) => {}
            ExprKind::Call(_, _, values)
            | ExprKind::Method(_, values)
            | ExprKind::Join(values)
            | ExprKind::ListOf(values) => {
                values.iter_mut().for_each(|value| self.expr(value));
            }
            ExprKind::CallValue(callee, values) => {
                self.expr(callee);
                values.iter_mut().for_each(|value| self.expr(value));
            }
            ExprKind::New { fields, .. } | ExprKind::Closure { fields, .. } => {
                fields.iter_mut().for_each(|(_, value)| self.expr(value));
            }
            ExprKind::MapOf(entries, _) => {
                for (key, value) in entries {
                    self.expr(key);
                    self.expr(value);
                }
            }
            ExprKind::Neg(inner)
            | ExprKind::Sqrt(inner)
            | ExprKind::Not(inner)
            | ExprKind::Convert(inner)
            | ExprKind::ToText(inner)
            | ExprKind::TextQuote(inner)
            | ExprKind::TextLen(inner)
            | ExprKind::TextIsEmpty(inner)
            | ExprKind::TextByteLen(inner)
            | ExprKind::TextBytes(inner)
            | ExprKind::SetOf(inner, _)
            | ExprKind::Field(inner, _)
            | ExprKind::Tag(inner)
            | ExprKind::TagIn(inner, _)
            | ExprKind::Member { value: inner, .. }
            | ExprKind::Wrap(inner)
            | ExprKind::View(inner)
            | ExprKind::Holds(inner, _)
            | ExprKind::Temp(_, inner)
            | ExprKind::Retain(inner)
            | ExprKind::Len(inner) => self.expr(inner),
            ExprKind::Binary(_, left, right)
            | ExprKind::Wrapping(_, left, right)
            | ExprKind::Hash(_, left, right)
            | ExprKind::TextCharAt(left, right)
            | ExprKind::ListGet(left, right)
            | ExprKind::ListHas(left, right)
            | ExprKind::MapGet(left, right)
            | ExprKind::MapHas(left, right)
            | ExprKind::MapNext(left, right)
            | ExprKind::MapKey(left, right)
            | ExprKind::MapValue(left, right)
            | ExprKind::MapFind(left, right) => {
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
