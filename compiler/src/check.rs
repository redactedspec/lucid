//! Binds names, checks types, and lowers the syntax tree to MIR in one pass.
//! A generic body is checked once, with its parameters as opaque types, and
//! `mono.rs` makes the concrete tasks. `resolve.rs` has already joined the
//! modules into one tree. List, map, and set tasks are built in.

use crate::ast::{
    self, BinaryOp, ExprKind as Ast, StmtKind, TextPart, TypeBody, TypeKind, UnaryOp,
};
use crate::diag::{Diagnostic, Source, SourceFile, Span, file_index};
use crate::mir::{
    Expr, ExprKind, Local, Program, Shape, ShapeVariant, Stmt, Task, TestCase, Ty, Types, Update,
    Visit, inner, json_tags,
};
use crate::native::find_symbol;
use std::collections::{HashMap, HashSet};

/// The traits that `resolve::PRELUDE` declares, which no module can redefine.
pub const PRELUDE_TRAITS: [&str; 1] = ["Error"];

/// Built-in type names with one form of their own, which `named` checks.
const LATER_TYPES: [&str; 1] = ["pointer"];

/// Type names that a program cannot define for itself, beyond `number_type`.
const BUILT_IN_TYPES: [&str; 8] =
    ["bool", "text", "list", "map", "set", "spawned", "channel", "lock"];

/// The same limit as nesting in the parser, for a chain of aliases.
const MAX_ALIASES: usize = 128;

/// How many instances of generic types the E0315 check follows, and how many
/// questions one trait check asks. For E0315 a type past this limit counts as
/// finite, and `mono.rs` stops any that really grows.
const MAX_FINITE_CHECK: usize = 256;

/// Traits that the compiler knows, and that a bound can name.
const BUILT_IN_TRAITS: [&str; 9] =
    ["Equal", "Order", "Show", "Hash", "Add", "Sub", "Mul", "Div", "Json"];

/// Traits in the spec that are not built yet.
const LATER_TRAITS: [&str; 1] = ["Copy"];

/// Dot calls that change their receiver, so they only work as statements, or
/// for `pop`, as the whole value of one.
const CHANGING_CALLS: [&str; 5] = ["push", "swap", "add", "remove", "pop"];

/// Dot calls on a channel or a lock that must be statements, since each one
/// waits for another thread, or, for `receive`, must be the whole value of
/// one, as `pop` must. None changes the name it is called on, since copies
/// share the channel or lock.
const THREAD_CALLS: [&str; 4] = ["send", "receive", "close", "update"];

const JOIN_TEXT: &str = "join text with interpolation: \"{a}{b}\"";
const TEST_UNION: &str = "test it first, as in `if x is not none`. Only a name narrows, so name a field or call with `let`";

/// Checks a file. A program using an unbuilt feature gets only E0901, because
/// names and types that feature would supply make any other error unreliable.
/// With `keep_tests` false, `test` blocks are dropped unread.
pub fn check(file: &ast::File, source: &Source, keep_tests: bool) -> (Program, Vec<Diagnostic>) {
    let mut checker = Checker {
        src: &source.text,
        files: &source.files,
        no_unsafe: &source.no_unsafe,
        ..Checker::default()
    };
    checker.declare_types(file);
    checker.declare(file);
    let written: Vec<_> = file
        .stmts
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Task(def) => Some((def, def.body.as_ref(), stmt.span)),
            _ => None,
        })
        .collect();
    let blocks: Vec<_> = file
        .stmts
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Test { name, body } if keep_tests => Some((name, body, stmt.span)),
            _ => None,
        })
        .collect();
    let first_test = written.len();
    checker.first_lifted = first_test + blocks.len();
    let mut tasks: Vec<Task> = written
        .into_iter()
        .enumerate()
        .map(|(id, (def, body, span))| match body {
            Some(body) => checker.task(id, def, body, span),
            None => checker.foreign_task(id, def, span),
        })
        .collect();
    let mut tests = Vec::with_capacity(blocks.len());
    for (name, body, span) in blocks {
        let name = checker.test_name(name, &tests);
        let task = first_test + tests.len();
        tasks.push(checker.test(&name, body, span));
        tests.push(TestCase { name, task, span });
    }
    let main = checker.main(file);
    tasks.append(&mut checker.lifted);

    let datas = checker.datas.iter().map(Data::shape).collect();
    let methods = checker.methods.iter().map(|(&name, ids)| (name.into(), ids.clone())).collect();
    let mut diags = checker.diags;
    if diags.iter().any(|d| d.code == "E0901") {
        diags.retain(|d| d.code == "E0901");
    }
    let unsafe_sites = checker.unsafe_sites;
    (Program { tasks, tests, unsafe_sites, main, types: checker.table, datas, methods }, diags)
}

/// Type parameters in scope, with the type that each one stands for.
type TypeEnv<'a> = Vec<(&'a str, Ty)>;

/// What belongs to one task while it is checked. A lambda or nested task gets
/// a new one, and the one around it waits in `Checker::frames`.
#[derive(Default)]
struct Frame<'a> {
    scopes: Vec<HashMap<&'a str, Local>>,
    locals: Vec<Ty>,
    mutable: Vec<bool>,
    /// The return type, or `None` at top level.
    returns: Option<Ty>,
    /// True in an `unsafe` block.
    in_unsafe: bool,
    /// True in a `test` block, where a failed `try` ends the program, as it
    /// does at top level.
    in_test: bool,
    /// True in a lambda whose result type comes from its own body.
    open_result: bool,
    /// The hidden locals of `try` and `else` in the statement being lowered,
    /// which let go of their value when it ends.
    held: Vec<Local>,
    /// Each name bound to an error that nothing has read yet.
    kept_errors: Vec<(Local, Span)>,
    /// The first scope of each open loop, its loop name, if it has one, and the
    /// length of `held` at its body. A loop name stays out of `Frame::leaving`,
    /// since `for` lets go of it.
    loop_scopes: Vec<(usize, Option<Local>, usize)>,
    /// What tests have proved about locals, latest last. See `forget`.
    narrowed: Vec<(Local, Ty)>,
    /// What this lambda or nested task captured so far, with each value as the
    /// task around it reads it. A position here is a `Capture`.
    captures: Vec<(String, Expr)>,
    /// The name of this nested task, which reads as its own first parameter.
    own_name: Option<&'a str>,
    /// How many locals are parameters.
    params: usize,
    /// What the open `for` loops visit.
    busy: Vec<Busy<'a>>,
    /// What the statements being lowered pass as `change`.
    changed: Vec<Changed<'a>>,
    /// True while a `change` argument finds its place.
    opening: bool,
    /// The fields that the `change` arguments of a call have taken out: the
    /// local of the container, the slot, and the local that holds the field.
    /// A second path through the same field goes on from that local.
    opened: Vec<(Local, i64, Local)>,
    /// The hidden local that holds the threads this task spawned, and the
    /// first `spawn`. See `open_spawns`.
    spawns: Option<(Local, Span)>,
}

/// A path that a `for` visits. A plain `for` lets the body read it. `for change`
/// has taken it out, so the body cannot touch it at all.
struct Busy<'a> {
    local: Local,
    path: Vec<&'a str>,
    exclusive: bool,
}

/// A path passed as `change`, and the code that may use it in that statement.
struct Changed<'a> {
    root: &'a str,
    path: Vec<&'a str>,
    span: Span,
    /// Other arguments of the call that may overlap: the `change` arguments,
    /// checked by `check_overlap`, and those that give a plain value, which is
    /// done before the call starts and leaves no borrow behind.
    peers: Vec<Span>,
}

/// The traits that each type parameter in scope has, by position.
type Bounds<'a> = Vec<Vec<&'a str>>;

struct Signature<'a> {
    name: &'a ast::Name,
    public: bool,
    type_params: TypeEnv<'a>,
    bounds: Bounds<'a>,
    /// A task of a trait: it has no body, and `self` has no type here.
    in_trait: bool,
    params: Vec<Param<'a>>,
    returns: Ty,
}

struct Param<'a> {
    name: &'a str,
    ty: Ty,
    default: Option<Expr>,
    mode: ast::ParamMode,
}

/// A written type name.
#[derive(Clone, Copy)]
enum Named<'a> {
    Known(Ty),
    /// A `type` or `choice` with type parameters.
    Generic(usize),
    /// Replaced by `Known` once resolved, unless it has type parameters.
    Alias(&'a ast::Type, &'a [ast::TypeParam]),
}

/// A `type`, which has one variant with its own name, or a `choice`.
struct Data<'a> {
    name: &'a ast::Name,
    /// With its own parameters as type arguments.
    ty: Ty,
    type_params: TypeEnv<'a>,
    bounds: Bounds<'a>,
    choice: bool,
    variants: Vec<Variant<'a>>,
}

impl Data<'_> {
    fn shape(&self) -> Shape {
        let shape = |variant: &Variant| ShapeVariant {
            name: variant.name.into(),
            fields: variant.fields.iter().map(|f| (f.name.into(), f.ty)).collect(),
            defaults: variant.fields.iter().map(|f| f.default.clone()).collect(),
        };
        Shape { variants: self.variants.iter().map(shape).collect(), choice: self.choice }
    }
}

struct Variant<'a> {
    name: &'a str,
    /// In written order, which is what a pattern binds.
    fields: Vec<Field<'a>>,
}

struct Field<'a> {
    name: &'a str,
    /// Uses the type parameters of its `type` or `choice`.
    ty: Ty,
    default: Option<Expr>,
}

/// A field as written: its name, type, default, and whether it has the plain
/// form. A variant's fields use the task parameter grammar, which allows more.
type WrittenField<'a> = (&'a ast::Name, Option<&'a ast::Type>, Option<&'a ast::Expr>, bool);

/// What the arms of a `match` have covered so far.
#[derive(Default)]
struct Covered {
    /// An arm matches anything.
    all: bool,
    /// An arm is already reported, so a gap may be its doing.
    failed: bool,
    /// The variants of a `choice`, or the type ids of a union.
    variants: HashSet<usize>,
    literals: HashSet<String>,
}

/// The value before the dot of a call, what its type shows about the task's
/// type parameters, and for a `change` receiver, where it came from.
struct Receiver<'a> {
    value: Expr,
    known: Vec<Option<Ty>>,
    changed: Option<(&'a ast::Expr, Place)>,
}

/// What a task can see of a top-level `let` or `var`.
enum Top<'a> {
    Var,
    /// A `let` with a literal value, which is known before any statement runs,
    /// and its annotation.
    Literal(&'a ast::Expr, Option<&'a ast::Type>),
    Let,
}

#[derive(Default)]
struct Checker<'a> {
    /// The text of the files, where an `unsafe` block needs a comment.
    src: &'a str,
    files: &'a [SourceFile],
    /// See `Source::no_unsafe`.
    no_unsafe: &'a [(usize, String)],
    diags: Vec<Diagnostic>,
    /// The ids of `extern` tasks.
    foreign: HashSet<usize>,
    unsafe_sites: Vec<(&'static str, Span)>,
    task_ids: HashMap<&'a str, usize>,
    signatures: Vec<Signature<'a>>,
    types: HashMap<&'a str, Named<'a>>,
    /// The type parameters that a written name can reach here.
    type_params: TypeEnv<'a>,
    bounds: Bounds<'a>,
    /// Tasks with a `self` parameter. A name may have one per receiver type.
    methods: HashMap<&'a str, Vec<usize>>,
    /// The tasks that each trait requires, as indexes into `signatures`.
    traits: HashMap<&'a str, Vec<usize>>,
    /// True once every task is declared, so that bounds can be checked.
    ready: bool,
    /// Bounds on type arguments written in a declaration: the type, the trait,
    /// the span, and the bounds in scope there.
    deferred: Vec<(Ty, &'a str, Span, Bounds<'a>)>,
    datas: Vec<Data<'a>>,
    /// A type or variant name, with its data and variant index.
    constructors: HashMap<&'a str, (usize, usize)>,
    /// The same names as written, without their modules.
    constructor_names: HashSet<&'a str>,
    /// The form of each foreign symbol, which all of its declarations share.
    symbols: HashMap<&'a str, (Vec<Ty>, Ty)>,
    /// The types whose written `hash` a trait check is inside.
    written_hashes: Vec<Ty>,
    /// The aliases being resolved, innermost last.
    resolving: Vec<&'a str>,
    top: HashMap<&'a str, Top<'a>>,
    table: Types,
    /// The task being checked.
    frame: Frame<'a>,
    /// The tasks around it, outermost first.
    frames: Vec<Frame<'a>>,
    /// Lambdas, nested tasks, and the code behind tasks used as values.
    /// Their ids start at `first_lifted`, after the tasks of the file.
    lifted: Vec<Task>,
    first_lifted: usize,
    /// The id in `lifted` of the code behind each task used as a value.
    task_values: HashMap<usize, usize>,
}

/// The parameters that a declaration brings into scope, by position.
fn own_params(params: &[ast::TypeParam]) -> TypeEnv<'_> {
    (params.iter().enumerate())
        .map(|(index, p)| (p.name.text.as_str(), Ty::Param(p.name.text.clone().leak(), index)))
        .collect()
}

/// The number type of a written name. `i64` and `u8` are other names for `int`
/// and `byte`, so each size has one type. See decision 79.
fn number_type(name: &str) -> Option<Ty> {
    Some(match name {
        "int" | "i64" => Ty::Int,
        "float" => Ty::Float,
        "byte" | "u8" => Ty::Byte,
        "i8" => Ty::I8,
        "i16" => Ty::I16,
        "i32" => Ty::I32,
        "u16" => Ty::U16,
        "u32" => Ty::U32,
        "u64" => Ty::U64,
        "f32" => Ty::F32,
        _ => return None,
    })
}

/// The number type that a task such as `to_i32` gives, on another number type.
fn conversion(ty: Ty, name: &str) -> Option<Ty> {
    let target = number_type(name.strip_prefix("to_")?)?;
    (ty.is_number() && target != ty).then_some(target)
}

/// The operator of a task such as `wrapping_add`, on an integer type.
fn wrapping_op(ty: Ty, name: &str) -> Option<BinaryOp> {
    match name {
        "wrapping_add" if ty.is_integer() => Some(BinaryOp::Add),
        "wrapping_sub" if ty.is_integer() => Some(BinaryOp::Sub),
        "wrapping_mul" if ty.is_integer() => Some(BinaryOp::Mul),
        _ => None,
    }
}

/// True for a task that the language builds in for this type.
fn builtin_method(ty: Ty, name: &str) -> bool {
    match ty {
        ty if ty.is_float() && name == "sqrt" => true,
        ty if ty.is_number() => conversion(ty, name).is_some() || wrapping_op(ty, name).is_some(),
        Ty::Text => {
            matches!(name, "len" | "is_empty" | "bytes" | "split" | "upper" | "lower" | "to_int")
        }
        Ty::List(Ty::Byte) if name == "pointer" => true,
        Ty::List(_) => matches!(name, "len" | "is_empty" | "get" | "push" | "pop" | "swap"),
        Ty::Map(..) => matches!(name, "len" | "is_empty" | "get" | "remove"),
        Ty::Set(_) => matches!(name, "len" | "is_empty" | "add" | "remove"),
        Ty::Spawned(_) => name == "wait",
        Ty::Channel(_) => matches!(name, "send" | "receive" | "close"),
        Ty::Lock(_) => matches!(name, "read" | "update"),
        _ => false,
    }
}

fn poisoned(span: Span) -> Expr {
    Expr { kind: ExprKind::None, ty: Ty::Error, span }
}

/// True for a name that means a built-in type or trait everywhere.
pub fn is_built_in(name: &str) -> bool {
    BUILT_IN_TYPES.contains(&name)
        || number_type(name).is_some()
        || LATER_TYPES.contains(&name)
        || BUILT_IN_TRAITS.contains(&name)
        || LATER_TRAITS.contains(&name)
}

pub fn is_literal(expr: &ast::Expr) -> bool {
    match &expr.kind {
        Ast::Int(_) | Ast::Float(_) | Ast::Bool(_) | Ast::None => true,
        Ast::Text(parts) => parts.iter().all(|part| matches!(part, TextPart::Literal(_))),
        _ => false,
    }
}

/// One step of a place: a field, or the local that holds an index or key.
#[derive(Clone, Copy, PartialEq)]
enum Step {
    Slot(i64),
    Key(Local),
}

/// A place to write through, such as `x[i]` in `x[i][j] = 1`. `local` holds the
/// value, and `setup` fills it. `take` empties its slot in the container, so
/// that the local holds the only count and the write needs no copy. `finish`
/// stores it back. Nothing may read the container between `take` and `finish`.
struct Place {
    root: Local,
    path: Vec<Step>,
    local: Local,
    setup: Vec<Stmt>,
    take: Vec<Stmt>,
    finish: Vec<Stmt>,
}

impl Place {
    fn at(local: Local) -> Self {
        let (path, setup, take, finish) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        Place { root: local, path, local, setup, take, finish }
    }

    /// The whole write. `write` holds the value that the write changes.
    fn stmts(mut self, write: Stmt) -> Stmt {
        self.setup.append(&mut self.take);
        self.setup.push(write);
        self.setup.append(&mut self.finish);
        Stmt::Block(self.setup)
    }
}

/// True if `outer` holds all of `inner`.
fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// True if control never reaches the end of `stmts`. With `jumps`, leaving
/// through `break` or `continue` counts.
fn ends(stmts: &[Stmt], jumps: bool) -> bool {
    stmts.iter().any(|stmt| match stmt {
        Stmt::Return(_) | Stmt::Placeholder(_) | Stmt::Unreachable => true,
        Stmt::Break | Stmt::Continue => jumps,
        Stmt::Block(stmts) => ends(stmts, jumps),
        Stmt::If { branches, otherwise } => {
            branches.iter().all(|(_, then)| ends(then, jumps)) && ends(otherwise, jumps)
        }
        _ => false,
    })
}

/// True if the line of `start` ends in a comment, or the line above it is one.
fn has_comment(src: &str, start: usize) -> bool {
    let line_start = src[..start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = src[start..].find('\n').map_or(src.len(), |at| start + at);
    let above = src[..line_start].lines().next_back();
    src[start..line_end].contains('#') || above.is_some_and(|l| l.trim_start().starts_with('#'))
}

/// One side of a failed `expect`. Text has quotes, so that a space shows.
/// `mono.rs` picks the code, since text may be behind a parameter or a union.
fn shown_side(value: Expr) -> Expr {
    let span = value.span;
    Expr { kind: ExprKind::TextQuote(value.into()), ty: Ty::Text, span }
}

/// A union value as a narrower type.
fn view(value: Expr, ty: Ty) -> Expr {
    if ty == value.ty || ty == Ty::Error {
        return Expr { ty, ..value };
    }
    let span = value.span;
    Expr { kind: ExprKind::View(value.into()), ty, span }
}

/// Runs `back` before a `break` or `continue` of a `for change`, and `leave`
/// before any `return` in its body, whose value runs first. `try` puts a
/// `return` inside an expression, so this walks those too.
struct PutBack<'b> {
    locals: &'b mut Vec<Ty>,
    mutable: &'b mut Vec<bool>,
    back: &'b [Stmt],
    leave: &'b [Stmt],
    own_loop: bool,
}

impl Visit for PutBack<'_> {
    fn expr(&mut self, value: &mut Expr) {
        self.expr_parts(value);
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::Break | Stmt::Continue if self.own_loop => {
                    let mut block = self.back.to_vec();
                    block.push(stmt.clone());
                    *stmt = Stmt::Block(block);
                }
                Stmt::Return(value) => {
                    self.expr(value);
                    let (ty, span) = (value.ty, value.span);
                    self.locals.push(ty);
                    self.mutable.push(false);
                    let held = self.locals.len() - 1;
                    let read = Expr { kind: ExprKind::Local(held), ty, span };
                    let value = std::mem::replace(value, read.clone());
                    let mut block = vec![Stmt::Set(held, value)];
                    block.extend(self.leave.iter().cloned());
                    block.push(Stmt::Return(read));
                    *stmt = Stmt::Block(block);
                }
                Stmt::While { .. } | Stmt::For { .. } => {
                    let own_loop = std::mem::replace(&mut self.own_loop, false);
                    self.stmt_parts(stmt);
                    self.own_loop = own_loop;
                }
                _ => self.stmt_parts(stmt),
            }
        }
    }
}

/// True if reading `value` twice costs nothing and changes nothing.
fn repeatable(value: &Expr) -> bool {
    match &value.kind {
        ExprKind::Local(_) | ExprKind::Capture(_) | ExprKind::Int(_) | ExprKind::Bool(_) => true,
        ExprKind::Member { value: inner, .. } | ExprKind::View(inner) => repeatable(inner),
        ExprKind::ListGet(base, key) | ExprKind::MapGet(base, key) => {
            repeatable(base) && repeatable(key)
        }
        _ => false,
    }
}

/// True for a number with no other value in it, which takes its type from the
/// other side of an operator. So `2 * 3 + x` works for any number type of `x`.
fn literal_math(expr: &ast::Expr) -> bool {
    match &expr.kind {
        Ast::Int(_) | Ast::Float(_) => true,
        Ast::Unary(UnaryOp::Neg | UnaryOp::BitNot, value) => literal_math(value),
        Ast::Binary(op, left, right) => is_math(*op) && literal_math(left) && literal_math(right),
        _ => false,
    }
}

fn is_math(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::FloorDiv
            | BinaryOp::Rem
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::Shl
            | BinaryOp::Shr
    )
}

fn want_name(want: Option<Ty>) -> String {
    want.map_or(String::new(), Ty::name)
}

fn compound(op: ast::AssignOp) -> Option<BinaryOp> {
    match op {
        ast::AssignOp::Set => None,
        ast::AssignOp::Add => Some(BinaryOp::Add),
        ast::AssignOp::Sub => Some(BinaryOp::Sub),
        ast::AssignOp::Mul => Some(BinaryOp::Mul),
        ast::AssignOp::Div => Some(BinaryOp::Div),
    }
}

fn symbol(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::FloorDiv => "//",
        BinaryOp::Rem => "%",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::And => "and",
        BinaryOp::Or => "or",
        _ => unreachable!("reported as unsupported first"),
    }
}

impl<'a> Checker<'a> {
    fn report(&mut self, diag: Diagnostic) -> Expr {
        let span = diag.span;
        self.diags.push(diag);
        poisoned(span)
    }

    fn fail(&mut self, code: &'static str, message: String, span: Span) -> Expr {
        self.report(Diagnostic::new(code, message, span))
    }

    fn unsupported(&mut self, what: &str, span: Span) -> Expr {
        let message = format!("{what} parses, but cannot run yet");
        let note = "see docs/roadmap.md for the milestone that adds it";
        self.report(Diagnostic::new("E0901", message, span).note(note))
    }

    fn later(&mut self, what: &str, span: Span) -> Option<Stmt> {
        self.unsupported(what, span);
        None
    }

    /// The tag of a member of a union.
    fn type_id(&self, ty: Ty) -> usize {
        self.table.id(ty)
    }

    fn union(&mut self, members: impl IntoIterator<Item = Ty>) -> Ty {
        self.table.union(members)
    }

    /// Makes `value` a `want`, if `want` is a union or an `any` that accepts
    /// it. Any other value comes back as it was, for `expect` to report.
    fn widen(&mut self, value: Expr, want: Ty) -> Expr {
        let span = value.span;
        let wrap = |value: Expr, want| Expr { kind: ExprKind::Wrap(value.into()), ty: want, span };
        if value.ty == want || value.ty == Ty::Error {
            return value;
        }
        if matches!(want, Ty::Any(_)) {
            return if self.fits_any(value.ty, want) { wrap(value, want) } else { value };
        }
        let Ty::Union(members) = want else { return value };
        if value.ty.members().iter().all(|member| members.contains(member)) {
            return wrap(value, want);
        }
        // A union stays out: a test could not then find the members it holds.
        let single = !matches!(value.ty, Ty::Union(_));
        let boxes = members.iter().find(|&&member| single && self.fits_any(value.ty, member));
        match boxes {
            Some(&member) => wrap(wrap(value, member), want),
            None => value,
        }
    }

    /// True if a value of type `ty` can go in `target`, which must be an `any`.
    /// One `any` does not go in another.
    fn fits_any(&mut self, ty: Ty, target: Ty) -> bool {
        let Ty::Any(name) = target else { return false };
        !matches!(ty, Ty::Any(_)) && self.satisfies(ty, name)
    }

    /// Splits a union by a test for `want`: what a true test leaves, then what
    /// a false one leaves. `None` if the answer never depends on the value.
    fn split(&mut self, have: Ty, want: Ty) -> Option<(Ty, Ty)> {
        if !matches!(have, Ty::Union(_))
            || want.members().iter().any(|member| !have.members().contains(member))
        {
            return None;
        }
        let rest: Vec<Ty> =
            have.members().iter().copied().filter(|ty| !want.members().contains(ty)).collect();
        (!rest.is_empty()).then(|| (want, self.union(rest)))
    }

    /// The one member of an expected union that a literal can build, as with
    /// `list[int]?` for `[]`. Any other expected type comes back as it was.
    fn literal_want(&self, want: Option<Ty>, fits: fn(&Ty) -> bool) -> Option<Ty> {
        let Some(Ty::Union(members)) = want else { return want };
        match members.iter().filter(|member| fits(member)).collect::<Vec<_>>().as_slice() {
            [one] => Some(**one),
            _ => want,
        }
    }

    /// The type that this local holds here.
    fn current(&self, local: Local) -> Ty {
        let found = self.frame.narrowed.iter().rev().find(|(narrowed, _)| *narrowed == local);
        found.map_or(self.frame.locals[local], |&(_, ty)| ty)
    }

    /// Ends what tests proved about a local that was assigned. Entries stay on
    /// the stack, so that a block can still cut it back to its own length. So
    /// an assignment on any path ends the narrowing on all that follow.
    fn forget(&mut self, local: Local) {
        let declared = self.frame.locals[local];
        for entry in self.frame.narrowed.iter_mut().filter(|(narrowed, _)| *narrowed == local) {
            entry.1 = declared;
        }
    }

    /// Runs before a loop: its body runs again after its own assignments, and
    /// after each name it passes whole as `change`.
    fn forget_assigned(&mut self, stmts: &'a [ast::Stmt]) {
        for stmt in stmts {
            let mut uses = Vec::new();
            stmt.values().into_iter().flatten().for_each(|value| value.uses(&mut uses));
            for found in uses.iter().filter(|found| found.change) {
                if let Some(local) = self.lookup(found.root) {
                    self.forget(local);
                }
            }
            match &stmt.kind {
                StmtKind::Assign { target, value, .. } => {
                    let name = match &target.kind {
                        Ast::Name(name) => Some(name.as_str()),
                        Ast::SelfValue => Some("self"),
                        _ => None,
                    };
                    if let Some(local) = name.and_then(|name| self.lookup(name)) {
                        self.forget(local);
                    }
                    self.forget_in_arms(value);
                }
                StmtKind::If { branches, otherwise } => {
                    for (_, then) in branches {
                        self.forget_assigned(then);
                    }
                    self.forget_assigned(otherwise.as_deref().unwrap_or_default());
                }
                StmtKind::While { body, .. }
                | StmtKind::For { body, .. }
                | StmtKind::Unsafe(body) => {
                    self.forget_assigned(body);
                }
                StmtKind::Expr(value)
                | StmtKind::Let { value, .. }
                | StmtKind::Return(Some(value)) => self.forget_in_arms(value),
                _ => {}
            }
        }
    }

    /// Does the same for the statements in the arms of a `match`.
    fn forget_in_arms(&mut self, value: &'a ast::Expr) {
        let Ast::Match { arms, .. } = &value.kind else { return };
        for arm in arms {
            match &arm.body {
                ast::ArmBody::Block(body) => self.forget_assigned(body),
                ast::ArmBody::Stmt(stmt) => self.forget_assigned(std::slice::from_ref(&**stmt)),
                ast::ArmBody::Expr(value) => self.forget_in_arms(value),
            }
        }
    }

    /// What is true of names when `condition` gives `truth`. Only a plain name
    /// narrows, since nothing else is sure to hold the same value when read
    /// again. See decision 60.
    fn facts(&mut self, condition: &'a ast::Expr, truth: bool) -> Vec<(Local, Ty)> {
        match &condition.kind {
            Ast::Unary(UnaryOp::Not, inner) => self.facts(inner, !truth),
            Ast::Binary(op, left, right)
                if *op == if truth { BinaryOp::And } else { BinaryOp::Or } =>
            {
                // The right side runs knowing the left, so it splits what is left.
                let mut facts = self.facts(left, truth);
                let start = self.frame.narrowed.len();
                self.frame.narrowed.extend(facts.iter().copied());
                let more = self.facts(right, truth);
                self.frame.narrowed.truncate(start);
                facts.extend(more);
                facts
            }
            Ast::Is { value, negated, target } => {
                let name = match &value.kind {
                    Ast::Name(name) => name.as_str(),
                    Ast::SelfValue => "self",
                    _ => return Vec::new(),
                };
                let Some(local) = self.lookup(name) else { return Vec::new() };
                let reported = self.diags.len();
                let want = self.is_target(target);
                self.diags.truncate(reported);
                let split = want.and_then(|want| self.split(self.current(local), want));
                let Some((found, rest)) = split else { return Vec::new() };
                vec![(local, if truth != *negated { found } else { rest })]
            }
            _ => Vec::new(),
        }
    }

    /// The type that an `is` tests, or `None` for `empty`.
    fn is_target(&mut self, target: &'a ast::IsTarget) -> Option<Ty> {
        match target {
            ast::IsTarget::None => Some(Ty::None),
            ast::IsTarget::Empty => None,
            ast::IsTarget::Type(ty) => Some(self.ty(ty)),
        }
    }

    /// Reports E0303 unless `value` has type `want`.
    fn expect(&mut self, value: &Expr, want: Ty, what: &str) {
        if value.ty == want || value.ty == Ty::Error || want == Ty::Error {
            return;
        }
        let message = format!("{what} needs `{}`, found `{}`", want.name(), value.ty.name());
        let diag = Diagnostic::new("E0303", message, value.span);
        self.report(match (want, value.ty) {
            (want, have) if want.is_number() && have.is_number() => {
                diag.note(format!("conversions are explicit: use `.to_{}()`", want.name()))
            }
            _ => diag,
        });
    }

    fn ty(&mut self, ty: &'a ast::Type) -> Ty {
        match &ty.kind {
            TypeKind::None => Ty::None,
            TypeKind::Union(terms) => {
                let members: Vec<Ty> = terms.iter().map(|term| self.ty(term)).collect();
                self.union(members)
            }
            TypeKind::Optional(inner) => {
                let inner = self.ty(inner);
                self.union([inner, Ty::None])
            }
            TypeKind::Named { path, args } => match path.as_slice() {
                [name] => {
                    let args: Vec<Ty> = args.iter().map(|arg| self.ty(arg)).collect();
                    self.named(&name.text, args, ty.span)
                }
                _ => {
                    let names: Vec<&str> = path.iter().map(|n| n.text.as_str()).collect();
                    let message = format!("type `{}` is not defined", names.join("."));
                    self.fail("E0302", message, ty.span).ty
                }
            },
            TypeKind::Any(held) => self.any_type(held, ty.span),
            TypeKind::Task { params, returns } => {
                let params: Vec<Ty> = params.iter().map(|param| self.ty(param)).collect();
                let returns = returns.as_ref().map_or(Ty::None, |returns| self.ty(returns));
                Ty::Task(params.leak(), inner(returns))
            }
            _ => self.unsupported("this type", ty.span).ty,
        }
    }

    /// `any Trait`. A built-in trait other than `Show` asks for two values of
    /// one type, which two boxes may not be.
    fn any_type(&mut self, held: &'a ast::Type, span: Span) -> Ty {
        match self.trait_named(held) {
            Some(name) if !BUILT_IN_TRAITS.contains(&name) || name == "Show" => {
                Ty::Any(name.to_string().leak())
            }
            Some(name) => {
                let message = format!("`any {name}` cannot be a type");
                let note = "its tasks need two values of one type, so use a bound";
                self.report(Diagnostic::new("E0303", message, span).note(note)).ty
            }
            None => Ty::Error,
        }
    }

    /// Reads a type argument, which the parser kept as an expression. See
    /// decision 65.
    fn expr_type(&mut self, expr: &'a ast::Expr) -> Ty {
        let span = expr.span;
        match &expr.kind {
            Ast::None => Ty::None,
            Ast::Name(name) => self.named(name, Vec::new(), span),
            Ast::Index { base, indices } => {
                let args: Vec<Ty> = indices.iter().map(|arg| self.expr_type(arg)).collect();
                match &base.kind {
                    Ast::Name(name) => self.named(name, args, span),
                    _ => self.fail("E0322", "this is not a type".into(), span).ty,
                }
            }
            Ast::Binary(BinaryOp::Or, left, right) => {
                let (left, right) = (self.expr_type(left), self.expr_type(right));
                self.union([left, right])
            }
            Ast::Optional(inner) => {
                let inner = self.expr_type(inner);
                self.union([inner, Ty::None])
            }
            Ast::Type(ty) => self.ty(ty),
            _ => self.fail("E0322", "this is not a type".into(), span).ty,
        }
    }

    /// The type that a written name means, with its type arguments.
    fn named(&mut self, name: &str, args: Vec<Ty>, span: Span) -> Ty {
        if args.contains(&Ty::Error) {
            return Ty::Error;
        }
        if let Some(&(_, param)) = self.type_params.iter().find(|(known, _)| *known == name) {
            return if self.arity(name, 0, args.len(), span) { param } else { Ty::Error };
        }
        if matches!(name, "list" | "map" | "set" | "spawned" | "channel" | "lock") {
            return self.collection_type(name, &args, span);
        }
        let plain = match name {
            "bool" => Some(Ty::Bool),
            "text" => Some(Ty::Text),
            "socket"
                if self.files[file_index(self.files, span.start)].path.starts_with("@std/") =>
            {
                Some(Ty::Socket)
            }
            _ => number_type(name),
        };
        if let Some(plain) = plain {
            return if self.arity(name, 0, args.len(), span) { plain } else { Ty::Error };
        }
        if LATER_TYPES.contains(&name) {
            if args == [Ty::Byte] {
                return Ty::Pointer;
            }
            let diag = Diagnostic::new("E0331", "this pointer type does not exist", span);
            return self.report(diag.note("the only one is `pointer[byte]`")).ty;
        }
        if self.is_trait(name) {
            let message = format!("`{name}` is a trait, not a type");
            let note = format!("use it as a bound, as in `[T: {name}]`");
            return self.report(Diagnostic::new("E0303", message, span).note(note)).ty;
        }
        match self.types.get_key_value(name) {
            Some((_, &Named::Known(known))) => {
                if self.arity(name, 0, args.len(), span) {
                    known
                } else {
                    Ty::Error
                }
            }
            Some((_, &Named::Generic(id))) => {
                let want = self.datas[id].type_params.len();
                if !self.arity(name, want, args.len(), span) {
                    return Ty::Error;
                }
                self.check_data_bounds(id, &args, span);
                let Ty::Data(name, ..) = self.datas[id].ty else {
                    unreachable!("a data type has its own name")
                };
                Ty::Data(name, id, args.leak())
            }
            Some((&name, &Named::Alias(target, params))) => {
                if !self.arity(name, params.len(), args.len(), span) {
                    return Ty::Error;
                }
                self.alias(name, target, params, &args, span)
            }
            None => self.fail("E0302", format!("type `{name}` is not defined"), span).ty,
        }
    }

    /// Reports E0321 unless a name gets as many type arguments as it declares.
    fn arity(&mut self, name: &str, want: usize, given: usize, span: Span) -> bool {
        if want != given {
            let plural = if want == 1 { "" } else { "s" };
            let message = format!("`{name}` takes {want} type argument{plural}, found {given}");
            self.fail("E0321", message, span);
        }
        want == given
    }

    fn alias(
        &mut self,
        name: &'a str,
        target: &'a ast::Type,
        params: &'a [ast::TypeParam],
        args: &[Ty],
        span: Span,
    ) -> Ty {
        if self.resolving.contains(&name) {
            let message = format!("alias `{name}` is defined by itself");
            return self.fail("E0315", message, span).ty;
        }
        if self.resolving.len() == MAX_ALIASES {
            return self.fail("E0205", "aliases are nested too deep".into(), span).ty;
        }
        let env = params.iter().map(|p| p.name.text.as_str()).zip(args.iter().copied()).collect();
        let outer = std::mem::replace(&mut self.type_params, env);
        self.resolving.push(name);
        let ty = self.ty(target);
        self.resolving.pop();
        self.type_params = outer;
        if params.is_empty() {
            self.types.insert(name, Named::Known(ty));
        }
        ty
    }

    fn collection_type(&mut self, name: &str, args: &[Ty], span: Span) -> Ty {
        match (name, args) {
            ("list", &[item]) => Ty::List(inner(item)),
            ("spawned", &[item]) => Ty::Spawned(inner(item)),
            ("channel", &[item]) => Ty::Channel(inner(item)),
            ("lock", &[item]) => Ty::Lock(inner(item)),
            ("set", &[item]) => {
                self.check_key(item, span);
                Ty::Set(inner(item))
            }
            ("map", &[key, value]) => {
                self.check_key(key, span);
                Ty::Map(inner(key), inner(value))
            }
            _ => {
                let needs = if name == "map" { "two types" } else { "one type" };
                self.fail("E0303", format!("`{name}` needs {needs}"), span).ty
            }
        }
    }

    /// A compound key needs `Hash`, which depends on tasks that may not be
    /// declared yet. A `float` never has it, since NaN is not equal to itself.
    fn check_key(&mut self, ty: Ty, span: Span) {
        match ty {
            Ty::Error => {}
            _ if ty.is_compound() || matches!(ty, Ty::Param(..)) => {
                if self.ready {
                    self.require(ty, "Hash", span);
                } else {
                    self.deferred.push((ty, "Hash", span, self.bounds.clone()));
                }
            }
            _ if ty.is_key() => {}
            _ => {
                let message = format!("`{}` cannot be a key", ty.name());
                self.fail("E0303", message, span);
            }
        }
    }

    /// Reports E0304 and returns false if a type or task has this name. A
    /// `type`, `choice`, variant, and task share one namespace, so that a
    /// variant reads the same in a pattern and in a call.
    fn define(&mut self, name: &'a ast::Name) -> bool {
        let text = name.text.as_str();
        let taken = self.types.contains_key(text)
            || self.constructors.contains_key(text)
            || self.task_ids.contains_key(text);
        if taken {
            self.fail("E0304", format!("`{text}` is already defined"), name.span);
        }
        !taken
    }

    /// Like `define`, for a name that a type or `set(...)` already means.
    fn define_data(&mut self, name: &'a ast::Name) -> bool {
        let text = name.text.as_str();
        let short = ast::short_name(text);
        let reserved = is_built_in(short)
            || self.traits.contains_key(text)
            || (text != short && PRELUDE_TRAITS.contains(&short));
        if reserved {
            let trait_name = BUILT_IN_TRAITS.contains(&short)
                || LATER_TRAITS.contains(&short)
                || PRELUDE_TRAITS.contains(&short);
            let what = match (trait_name, is_built_in(short)) {
                (true, _) => "a built-in trait",
                (false, true) => "a built-in type",
                (false, false) => "a trait",
            };
            let message = format!("`{text}` is already defined as {what}");
            self.report(Diagnostic::new("E0304", message, name.span).note("pick another name"));
            return false;
        }
        self.define(name)
    }

    fn is_trait(&self, name: &str) -> bool {
        self.traits.contains_key(name) || BUILT_IN_TRAITS.contains(&name)
    }

    /// The traits that each parameter of a declaration has.
    fn bounds_of(&mut self, params: &'a [ast::TypeParam]) -> Bounds<'a> {
        let mut all = Vec::with_capacity(params.len());
        for param in params {
            let traits = param.bounds.iter().filter_map(|bound| self.trait_named(bound));
            all.push(traits.collect());
        }
        all
    }

    /// The trait that a bound or `any` names. Reports anything else.
    fn trait_named(&mut self, written: &'a ast::Type) -> Option<&'a str> {
        let span = written.span;
        let name = match &written.kind {
            TypeKind::Named { path, args } if args.is_empty() => match path.as_slice() {
                [name] => Some(name.text.as_str()),
                _ => None,
            },
            _ => None,
        };
        match name {
            Some(name) if self.is_trait(name) => return Some(name),
            Some(name) if LATER_TRAITS.contains(&name) => {
                self.unsupported(&format!("trait `{name}`"), span);
            }
            Some(name) if self.types.contains_key(name) => {
                self.fail("E0302", format!("`{name}` is a type, not a trait"), span);
            }
            Some(name) => {
                self.fail("E0302", format!("trait `{name}` is not defined"), span);
            }
            None => {
                self.fail("E0302", "this must name a trait".into(), span);
            }
        }
        None
    }

    /// Checks the bounds of a generic `type` or `choice`, now or once every
    /// task is declared.
    fn check_data_bounds(&mut self, id: usize, args: &[Ty], span: Span) {
        let bounds = self.datas[id].bounds.clone();
        for (&ty, traits) in args.iter().zip(bounds) {
            for name in traits {
                if self.ready {
                    self.require(ty, name, span);
                } else {
                    self.deferred.push((ty, name, span, self.bounds.clone()));
                }
            }
        }
    }

    /// Reports E0320 unless a type has a trait.
    fn require(&mut self, ty: Ty, name: &str, span: Span) -> bool {
        let found = self.satisfies(ty, name);
        if !found {
            let message = format!("`{}` does not have `{name}`", ty.name());
            let diag = Diagnostic::new("E0320", message, span);
            let written_equal = name == "Hash" && self.find_method("equal", ty).is_some();
            self.report(if let Ty::Param(param, _) = ty {
                diag.note(format!("add a bound, as in `[{param}: {name}]`"))
            } else if written_equal {
                diag.note("a hand-written `equal` needs a hand-written `hash` to match it")
            } else if name == "Json" && !self.json_union_clear(ty) {
                diag.note("two members would read from one kind of JSON value")
            } else {
                diag
            });
        }
        found
    }

    fn satisfies(&mut self, ty: Ty, name: &str) -> bool {
        self.satisfied(ty, name, &mut Vec::new())
    }

    /// True if a type has a trait. `seen` holds what was already asked: a
    /// recursive type has a trait if nothing else in it lacks one, and any
    /// `false` ends the whole walk. A type that keeps growing has no last
    /// answer, so past the limits the answer is no.
    fn satisfied(&mut self, ty: Ty, name: &str, seen: &mut Vec<(Ty, String)>) -> bool {
        if ty == Ty::Error {
            return true;
        }
        if ty == Ty::Pointer {
            return false;
        }
        if let Ty::Param(_, index) = ty {
            return self.bounds.get(index).is_some_and(|traits| traits.contains(&name));
        }
        if name == "Hash" && self.written_hashes.contains(&ty) {
            return false;
        }
        if let Ty::Any(held) = ty
            && (held == name || matches!(name, "Show" | "Equal"))
        {
            let tasks = self.traits.get(name).cloned().unwrap_or_default();
            return !tasks.into_iter().any(|task| self.changes(task));
        }
        if ty.too_large() || seen.len() == MAX_FINITE_CHECK {
            return false;
        }
        if seen.iter().any(|(known, trait_name)| *known == ty && trait_name == name) {
            return true;
        }
        seen.push((ty, name.into()));
        let written = match name {
            "Show" => "show",
            "Equal" => "equal",
            "Order" => "compare",
            "Add" => "add",
            "Sub" => "sub",
            "Mul" => "mul",
            "Div" => "div",
            _ => "",
        };
        if let Some(found) = self.written_task(written, ty, seen) {
            return found;
        }
        if name == "Hash"
            && let Some(found) = self.written_hash(ty, seen)
        {
            return found;
        }
        if !BUILT_IN_TRAITS.contains(&name) {
            let tasks = self.traits.get(name).cloned().unwrap_or_default();
            return tasks.into_iter().all(|task| self.fits_trait_task(ty, task, seen));
        }
        let parts: Vec<Ty> = match ty {
            Ty::List(item) | Ty::Set(item) => vec![*item],
            Ty::Map(key, value) => vec![*key, *value],
            Ty::Union(members) => members.to_vec(),
            Ty::Data(..) => self.variants_of(ty).into_iter().flatten().collect(),
            _ => Vec::new(),
        };
        if name == "Equal" && (matches!(ty, Ty::Task(..)) || ty.is_handle()) {
            return false;
        }
        let parts = if name == "Hash" {
            // A `none` inside a value hashes as nothing. Alone it is no key.
            parts.into_iter().filter(|&part| part != Ty::None).collect()
        } else {
            parts
        };
        // A read reports failure as a `JsonError`, so one cannot be a value read.
        let json_error = matches!(ty, Ty::Data(name, ..) if name == "@std.json.JsonError");
        if name == "Json" && (json_error || !self.json_union_clear(ty)) {
            return false;
        }
        let holds = match name {
            "Show" | "Equal" => true,
            "Json" => match ty {
                Ty::Map(key, _) => *key == Ty::Text,
                Ty::Any(_) | Ty::Task(..) => false,
                ty => !ty.is_handle(),
            },
            "Hash" => ty.is_key() || (ty.is_compound() && !matches!(ty, Ty::Any(_))),
            "Order" => ty.is_number() || matches!(ty, Ty::Text | Ty::List(_) | Ty::Data(..)),
            "Div" => return ty.is_float(),
            _ => return ty.is_number(),
        };
        if !holds {
            return false;
        }
        parts.into_iter().all(|part| self.satisfied(part, name, seen))
    }

    /// True unless two members of a union can read from one kind of JSON value.
    /// So a read never has to try a member and go back. A type parameter
    /// counts as clear: `mono.rs` then takes the first member, in type order,
    /// whose kind of JSON value fits, and tries no other.
    fn json_union_clear(&self, ty: Ty) -> bool {
        let mut seen = Vec::new();
        ty.members().len() < 2
            || ty.members().iter().all(|&member| {
                let tags = json_tags(member, |id| self.datas[id].shape().json_forms());
                let clear = tags.iter().all(|tag| !seen.contains(tag));
                seen.extend(tags);
                clear
            })
    }

    /// With a hand-written task for this type, true if its bounds hold.
    /// `None` if there is no such task.
    fn written_task(&mut self, name: &str, ty: Ty, seen: &mut Vec<(Ty, String)>) -> Option<bool> {
        let (id, known) = self.find_method(name, ty)?;
        let bounds = self.signatures[id].bounds.clone();
        for (arg, traits) in known.iter().zip(bounds) {
            for name in traits {
                if !arg.is_some_and(|arg| self.satisfied(arg, name, seen)) {
                    return Some(false);
                }
            }
        }
        Some(true)
    }

    /// Whether a type's own tasks give it `Hash`, or `None` if they say nothing.
    /// A written `hash` needs a result with `Hash`. A written `equal` alone
    /// gives no `Hash`, since a derived hash would not match it. A result that
    /// leads back to the same type would never end. See decision 77.
    fn written_hash(&mut self, ty: Ty, seen: &mut Vec<(Ty, String)>) -> Option<bool> {
        let Some(found) = self.written_task("hash", ty, seen) else {
            return self.find_method("equal", ty).map(|_| false);
        };
        let (id, known) = self.find_method("hash", ty)?;
        let args: Option<Vec<Ty>> = known.into_iter().collect();
        let (Some(args), true) = (args, found) else { return Some(false) };
        let result = self.table.subst(self.signatures[id].returns, &args);
        self.written_hashes.push(ty);
        let found = self.satisfied(result, "Hash", seen);
        self.written_hashes.pop();
        Some(found)
    }

    /// True if `ty` has a task that matches a task of a trait.
    fn fits_trait_task(&mut self, ty: Ty, task: usize, seen: &mut Vec<(Ty, String)>) -> bool {
        let name = &self.signatures[task].name.text;
        let Some(true) = self.written_task(name, ty, seen) else { return false };
        let Some((id, known)) = self.find_method(name, ty) else { return false };
        let Some(args) = known.into_iter().collect::<Option<Vec<Ty>>>() else { return false };
        let wanted: Vec<Ty> = self.signatures[task].params.iter().map(|p| p.ty).collect();
        let have: Vec<Ty> = self.signatures[id].params.iter().map(|p| p.ty).collect();
        let (want_returns, have_returns) =
            (self.signatures[task].returns, self.signatures[id].returns);
        let have: Vec<Ty> = have.into_iter().map(|p| self.table.subst(p, &args)).collect();
        let have_returns = self.table.subst(have_returns, &args);
        let modes = |task: &Signature| -> Vec<bool> {
            task.params.iter().map(|p| p.mode == ast::ParamMode::Change).collect()
        };
        wanted.len() == have.len()
            && wanted[1..] == have[1..]
            && want_returns == have_returns
            && modes(&self.signatures[task]) == modes(&self.signatures[id])
    }

    /// True if a task has a `change` parameter.
    fn changes(&self, task: usize) -> bool {
        self.signatures[task].params.iter().any(|p| p.mode == ast::ParamMode::Change)
    }

    /// The task with a `self` parameter that takes this type, if any, and what
    /// the type shows about its type parameters.
    fn find_method(&mut self, name: &str, ty: Ty) -> Option<(usize, Vec<Option<Ty>>)> {
        for id in self.methods.get(name).cloned().unwrap_or_default() {
            let pattern = self.signatures[id].params[0].ty;
            let count = self.signatures[id].type_params.len();
            if let Some(known) = self.table.fit(pattern, ty, count) {
                return Some((id, known));
            }
        }
        None
    }

    /// Reports a type parameter that reuses a name. A field or parameter could
    /// otherwise not tell it from the type that it hides.
    fn check_type_params(&mut self, params: &'a [ast::TypeParam]) {
        let mut seen: Vec<&str> = Vec::new();
        for param in params {
            let text = param.name.text.as_str();
            let taken = seen.contains(&text)
                || self.types.contains_key(text)
                || self.is_trait(text)
                || LATER_TRAITS.contains(&text)
                || BUILT_IN_TYPES.contains(&text)
                || number_type(text).is_some()
                || LATER_TYPES.contains(&text);
            if taken {
                self.fail("E0304", format!("`{text}` is already defined"), param.name.span);
            }
            seen.push(text);
        }
    }

    fn new_data(
        &mut self,
        name: &'a ast::Name,
        params: &'a [ast::TypeParam],
        choice: bool,
    ) -> usize {
        let id = self.datas.len();
        let type_params = own_params(params);
        let bounds = self.bounds_of(params);
        let args: Vec<Ty> = type_params.iter().map(|&(_, ty)| ty).collect();
        let generic = !args.is_empty();
        let ty = Ty::Data(name.text.clone().leak(), id, args.leak());
        self.datas.push(Data { name, ty, type_params, bounds, choice, variants: Vec::new() });
        let named = if generic { Named::Generic(id) } else { Named::Known(ty) };
        self.types.insert(&name.text, named);
        id
    }

    /// Runs before anything that uses a type: fields may name any type, so
    /// all names go in first, then aliases resolve, then fields.
    fn declare_types(&mut self, file: &'a ast::File) {
        let mut aliases = Vec::new();
        let mut headers = Vec::new();
        let mut written: Vec<(usize, usize, Vec<WrittenField<'a>>)> = Vec::new();
        for stmt in &file.stmts {
            if let StmtKind::Trait(def) = &stmt.kind {
                self.declare_trait(def);
            }
        }
        for stmt in &file.stmts {
            match &stmt.kind {
                StmtKind::Type(def) => {
                    if !self.define_data(&def.name) {
                        continue;
                    }
                    headers.push(def.type_params.as_slice());
                    match &def.body {
                        TypeBody::Alias(target) => {
                            let name = def.name.text.as_str();
                            let params = def.type_params.as_slice();
                            if let Some(bound) = params.iter().find_map(|p| p.bounds.first()) {
                                self.unsupported("a bound on an alias", bound.span);
                            }
                            self.types.insert(name, Named::Alias(target, params));
                            aliases.push((name, target, params, def.name.span));
                        }
                        TypeBody::Fields(fields) => {
                            let id = self.new_data(&def.name, &def.type_params, false);
                            let name = def.name.text.as_str();
                            let short = ast::short_name(name);
                            self.constructor_names.insert(short);
                            self.datas[id]
                                .variants
                                .push(Variant { name: short, fields: Vec::new() });
                            self.constructors.insert(name, (id, 0));
                            let fields = fields
                                .iter()
                                .map(|f| (&f.name, Some(&f.ty), f.default.as_ref(), true))
                                .collect();
                            written.push((id, 0, fields));
                        }
                    }
                }
                StmtKind::Choice(def) => {
                    if !self.define_data(&def.name) {
                        continue;
                    }
                    headers.push(def.type_params.as_slice());
                    let id = self.new_data(&def.name, &def.type_params, true);
                    for variant in &def.variants {
                        let name = variant.name.text.as_str();
                        let index = self.datas[id].variants.len();
                        if self.define_data(&variant.name) {
                            self.constructors.insert(name, (id, index));
                            self.constructor_names.insert(ast::short_name(name));
                        }
                        let short = ast::short_name(name);
                        self.datas[id].variants.push(Variant { name: short, fields: Vec::new() });
                        let fields = variant
                            .params
                            .iter()
                            .map(|p| {
                                let plain = p.mode == ast::ParamMode::Read && p.name.text != "self";
                                (&p.name, p.ty.as_ref(), p.default.as_ref(), plain)
                            })
                            .collect();
                        written.push((id, index, fields));
                    }
                }
                _ => {}
            }
        }
        for params in headers {
            self.check_type_params(params);
        }
        for (name, target, params, span) in aliases {
            if let Some(Named::Alias(..)) = self.types.get(name) {
                let args: Vec<Ty> = own_params(params).iter().map(|&(_, ty)| ty).collect();
                self.alias(name, target, params, &args, span);
            }
        }
        for (id, variant, fields) in written {
            self.type_params.clone_from(&self.datas[id].type_params);
            self.bounds.clone_from(&self.datas[id].bounds);
            let fields = self.fields(fields);
            self.type_params.clear();
            self.bounds.clear();
            self.datas[id].variants[variant].fields = fields;
        }
        self.check_finite();
    }

    fn fields(&mut self, written: Vec<WrittenField<'a>>) -> Vec<Field<'a>> {
        let mut fields: Vec<Field<'a>> = Vec::with_capacity(written.len());
        for (name, ty, default, plain) in written {
            if !plain {
                let message = "a field cannot be `self`, `change`, or `take`";
                self.fail("E0307", message.into(), name.span);
            }
            let ty = ty.map_or(Ty::Error, |ty| self.ty(ty));
            if fields.iter().any(|field| field.name == name.text) {
                self.fail("E0304", format!("`{}` is already defined", name.text), name.span);
            }
            let default = default.map(|value| self.default_value(value, ty));
            fields.push(Field { name: &name.text, ty, default });
        }
        fields
    }

    /// The field types of each variant of a `type` or `choice`, with its type
    /// arguments in place.
    fn variants_of(&mut self, ty: Ty) -> Vec<Vec<Ty>> {
        let Ty::Data(_, id, args) = ty else { return Vec::new() };
        let written = self.datas[id].shape().variants;
        let table = &mut self.table;
        let subst = |variant: ShapeVariant| {
            variant.fields.iter().map(|&(_, ty)| table.subst(ty, args)).collect()
        };
        written.into_iter().map(subst).collect()
    }

    /// Reports a type with no finite value. Recursion must pass through a
    /// variant that can avoid it, or a collection that can be empty. A generic
    /// type needs this per instance: `type R` with a `W[R]` field holds itself
    /// if `W[T]` holds a `T`.
    fn check_finite(&mut self) {
        fn holds(ty: Ty, finite: &HashSet<Ty>, explored: &HashSet<Ty>) -> bool {
            match ty {
                Ty::Data(..) => ty.too_large() || finite.contains(&ty) || !explored.contains(&ty),
                Ty::Union(members) => members.iter().any(|&m| holds(m, finite, explored)),
                _ => true,
            }
        }
        let mut instances: Vec<Ty> = self.datas.iter().map(|data| data.ty).collect();
        let mut listed: HashSet<Ty> = instances.iter().copied().collect();
        let mut shapes = Vec::new();
        let limit = MAX_FINITE_CHECK.max(instances.len());
        while shapes.len() < instances.len() && shapes.len() < limit {
            let variants = self.variants_of(instances[shapes.len()]);
            for &field in variants.iter().flatten() {
                for &member in field.members() {
                    let open = matches!(member, Ty::Data(..)) && !member.too_large();
                    if open && listed.insert(member) {
                        instances.push(member);
                    }
                }
            }
            shapes.push(variants);
        }
        instances.truncate(shapes.len());
        let explored: HashSet<Ty> = instances.iter().copied().collect();
        let mut finite = HashSet::new();
        let mut changed = true;
        while changed {
            changed = false;
            for (&ty, variants) in instances.iter().zip(&shapes) {
                let reachable = variants
                    .iter()
                    .any(|fields| fields.iter().all(|&f| holds(f, &finite, &explored)));
                changed |= reachable && finite.insert(ty);
            }
        }
        for id in 0..self.datas.len() {
            if !finite.contains(&self.datas[id].ty) {
                let name = self.datas[id].name;
                let message = format!("`{}` has no finite value", name.text);
                let note = "building it needs a type that holds itself: add a variant with no \
                            such field, or hold the value in a list or a union such as `T?`";
                self.report(Diagnostic::new("E0315", message, name.span).note(note));
            }
        }
    }

    /// A trait name goes in before anything that may name it. Its tasks wait
    /// for `declare`, since their types may be any type.
    fn declare_trait(&mut self, def: &'a ast::TraitDef) {
        if let Some(first) = def.type_params.first() {
            self.unsupported("a generic trait", first.name.span);
        }
        if self.define_data(&def.name) {
            self.traits.insert(&def.name.text, Vec::new());
        }
    }

    /// Reports what no receiver may do: overlap another one, take the name of
    /// a built-in task, or give a derived task the wrong signature.
    fn check_methods(&mut self) {
        for id in 0..self.signatures.len() {
            let signature = &self.signatures[id];
            if signature.in_trait || signature.params.first().is_none_or(|p| p.name != "self") {
                continue;
            }
            let (name, span, receiver) =
                (signature.name.text.as_str(), signature.name.span, signature.params[0].ty);
            if receiver == Ty::Error {
                continue;
            }
            if matches!(receiver, Ty::Param(..)) {
                let message = "`self` needs a type that is not a bare type parameter";
                self.fail("E0307", message.into(), span);
                continue;
            }
            let earlier: Vec<usize> =
                self.methods[name].iter().copied().filter(|&o| o < id).collect();
            let overlaps = earlier.into_iter().any(|other| self.overlap(id, other));
            if overlaps
                || builtin_method(receiver, name)
                || self.trait_task(receiver, name).is_some()
            {
                self.fail("E0304", format!("`{name}` is already defined"), span);
            }
            self.check_written_shape(id);
        }
    }

    /// True if one value could be the receiver of both tasks.
    fn overlap(&self, a: usize, b: usize) -> bool {
        self.signatures[a].params[0].ty.may_overlap(self.signatures[b].params[0].ty)
    }

    /// A task named after a derived one replaces it, so it must fit. See
    /// decision 73.
    fn check_written_shape(&mut self, id: usize) {
        let signature = &self.signatures[id];
        let (name, span) = (signature.name.text.as_str(), signature.name.span);
        let receiver = signature.params[0].ty;
        let (others, returns) = match name {
            "show" => (0, Ty::Text),
            "equal" => (1, Ty::Bool),
            "compare" => (1, Ty::Int),
            "add" | "sub" | "mul" | "div" => (1, receiver),
            // Any result with `Hash` will do, which the trait check tests.
            "hash" if signature.returns == Ty::None => (0, Ty::Int),
            "hash" => (0, signature.returns),
            _ => return,
        };
        let count = signature.type_params.len();
        let fits = signature.params.len() == others + 1
            && !self.changes(id)
            && signature.params[1..].iter().all(|p| p.ty == receiver)
            && signature.returns == returns
            && receiver.is_compound()
            && !matches!(receiver, Ty::Any(_));
        let mut known = vec![None; count];
        self.table.unify(receiver, receiver, &mut known);
        let fits = fits && known.iter().all(Option::is_some);
        if !fits {
            let other = if others == 1 { ", other: T" } else { "" };
            let message = format!("`{name}` replaces a derived task, so it needs this form");
            let note = format!(
                "write `task {name}(self: T{other}) -> {}`, for a `type`, `choice`, or collection `T`",
                match name {
                    "hash" => "int".into(),
                    _ if returns == receiver => "T".into(),
                    _ => returns.name(),
                }
            );
            self.report(Diagnostic::new("E0323", message, span).note(note));
        }
    }

    /// First pass: tasks and top-level values may be used before their line.
    fn declare(&mut self, file: &'a ast::File) {
        for stmt in &file.stmts {
            let StmtKind::Task(def) = &stmt.kind else { continue };
            let name = def.sig.name.text.as_str();
            let id = self.signatures.len();
            if def.sig.is_method() {
                self.methods.entry(name).or_default().push(id);
            } else if self.define(&def.sig.name) {
                self.task_ids.insert(name, id);
            }
            let mut signature = self.signature(&def.sig, false);
            signature.public = def.public;
            if def.body.is_none() {
                self.check_foreign(def, &signature);
                self.foreign.insert(id);
                self.unsafe_sites.push(("extern", stmt.span));
                self.need_permission("an `extern` task", stmt.span);
            }
            self.signatures.push(signature);
        }
        for stmt in &file.stmts {
            let StmtKind::Trait(def) = &stmt.kind else { continue };
            for sig in &def.tasks {
                if let Some(first) = sig.type_params.first() {
                    self.unsupported("a generic task in a trait", first.name.span);
                }
                if sig.params.first().is_none_or(|param| param.name.text != "self") {
                    let message = format!("`{}` needs `self` first", sig.name.text);
                    self.fail("E0307", message, sig.name.span);
                    continue;
                }
                let id = self.signatures.len();
                let signature = self.signature(sig, true);
                self.signatures.push(signature);
                if let Some(tasks) = self.traits.get_mut(def.name.text.as_str()) {
                    tasks.push(id);
                }
            }
        }
        for stmt in &file.stmts {
            if let StmtKind::Let { mutable, name, ty, value } = &stmt.kind {
                let top = match (*mutable, is_literal(value)) {
                    (true, _) => Top::Var,
                    (false, true) => Top::Literal(value, ty.as_ref()),
                    (false, false) => Top::Let,
                };
                self.top.insert(&name.text, top);
            }
        }
        self.check_methods();
        self.ready = true;
        for (ty, name, span, bounds) in std::mem::take(&mut self.deferred) {
            self.bounds = bounds;
            self.require(ty, name, span);
        }
        self.bounds.clear();
    }

    fn signature(&mut self, sig: &'a ast::TaskSig, in_trait: bool) -> Signature<'a> {
        self.check_type_params(&sig.type_params);
        let type_params = own_params(&sig.type_params);
        let bounds = self.bounds_of(&sig.type_params);
        self.type_params.clone_from(&type_params);
        self.bounds.clone_from(&bounds);
        let mut params: Vec<Param<'a>> = Vec::new();
        for param in &sig.params {
            let span = param.name.span;
            if param.mode == ast::ParamMode::Change && param.default.is_some() {
                self.fail("E0307", "a `change` parameter takes no default".into(), span);
            }
            let ty = param.ty.as_ref().map_or(Ty::Error, |ty| self.ty(ty));
            if param.name.text == "self" {
                if !params.is_empty() {
                    self.fail("E0307", "`self` must come first".into(), span);
                } else if in_trait && param.ty.is_some() {
                    self.fail("E0307", "`self` has no type in a trait".into(), span);
                } else if !in_trait && param.ty.is_none() {
                    let diag = Diagnostic::new("E0307", "`self` needs a type here", span);
                    self.report(diag.note("write `self: User`. Only a trait leaves it out"));
                }
            } else if params.iter().any(|p| p.name == param.name.text) {
                self.fail("E0304", format!("`{}` is already defined", param.name.text), span);
            }
            let default = param.default.as_ref().map(|value| self.default_value(value, ty));
            if default.is_none() && params.last().is_some_and(|p| p.default.is_some()) {
                let message =
                    format!("`{}` needs a default, like the one before it", param.name.text);
                self.fail("E0307", message, span);
            }
            params.push(Param { name: &param.name.text, ty, default, mode: param.mode });
        }
        let returns = sig.returns.as_ref().map_or(Ty::None, |ty| self.ty(ty));
        self.type_params.clear();
        self.bounds.clear();
        let (name, public) = (&sig.name, false);
        Signature { name, public, type_params, bounds, in_trait, params, returns }
    }

    fn default_value(&mut self, value: &'a ast::Expr, ty: Ty) -> Expr {
        let empty = match &value.kind {
            Ast::List(items) => items.is_empty(),
            Ast::Map(entries) => entries.is_empty(),
            _ => false,
        };
        if !is_literal(value) && !empty {
            return self.fail(
                "E0313",
                "a default must be a literal, or an empty `[]` or `{}`".into(),
                value.span,
            );
        }
        let value = self.expected(value, Some(ty));
        self.expect(&value, ty, "this default");
        value
    }

    fn task(
        &mut self,
        id: usize,
        def: &'a ast::TaskDef,
        body: &'a [ast::Stmt],
        span: Span,
    ) -> Task {
        let returns = self.signatures[id].returns;
        self.frame.returns = Some(returns);
        self.type_params.clone_from(&self.signatures[id].type_params);
        self.bounds.clone_from(&self.signatures[id].bounds);
        let params: Vec<(&'a str, Ty, ast::ParamMode)> =
            self.signatures[id].params.iter().map(|p| (p.name, p.ty, p.mode)).collect();
        let mut scope = HashMap::new();
        let mut change = Vec::new();
        // The runtime calls a written `equal` directly, and passes no count.
        let method = params.first().is_some_and(|param| param.0 == "self");
        let written = !(method && self.signatures[id].name.text == "equal");
        let mut take = Vec::new();
        for &(name, ty, mode) in &params {
            let changes = mode == ast::ParamMode::Change;
            let local = self.new_local(ty, changes);
            scope.insert(name, local);
            change.extend(changes.then_some(local));
            take.extend((written && mode == ast::ParamMode::Take).then_some(local));
        }
        self.frame.params = params.len();
        let mut lowered = Vec::new();
        for (param, &(name, _, mode)) in params.iter().enumerate() {
            if mode == ast::ParamMode::Take {
                let (own, set) = self.own_param(param, span);
                scope.insert(name, own);
                lowered.push(set);
            }
        }
        self.frame.scopes.push(scope);
        lowered.extend(self.stmts(body));
        let mut body = lowered;
        self.frame.scopes.pop();
        self.check_returns(&def.sig.name, returns, &mut body);
        self.open_spawns(&mut body);
        self.frame.mutable.clear();
        let type_params = self.type_params.len();
        self.type_params.clear();
        self.bounds.clear();
        let locals = std::mem::take(&mut self.frame.locals);
        let captures = Vec::new();
        self.frame.params = 0;
        let params = params.len();
        let borrowed = Vec::new();
        let name: &'static str = def.sig.name.text.clone().leak();
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

    /// A local holds a count of a `take` parameter, so the task can change it
    /// and the caller never sees. See `Task::take`.
    fn own_param(&mut self, param: Local, span: Span) -> (Local, Stmt) {
        let local = self.new_local(self.frame.locals[param], true);
        (local, Stmt::Set(local, self.read(param, span)))
    }

    /// Reports E0309 unless a body with a return type returns on every path.
    /// A task that gives only `none` or an error returns `none` at its end.
    fn check_returns(&mut self, name: &ast::Name, returns: Ty, body: &mut Vec<Stmt>) {
        self.check_errors_read();
        let failing = returns.members().contains(&Ty::None)
            && self.error_members(returns).len() + 1 == returns.members().len();
        if failing && returns != Ty::None && !ends(body, false) {
            let none = Expr { kind: ExprKind::None, ty: Ty::None, span: name.span };
            let none = self.widen(none, returns);
            body.push(Stmt::Return(none));
        } else if returns != Ty::None && returns != Ty::Error && !ends(body, false) {
            let message = format!("`{}` must return `{}` on every path", name.text, returns.name());
            self.fail("E0309", message, name.span);
        }
    }

    /// An `extern` task crosses to C, where only plain values make sense.
    fn check_foreign(&mut self, def: &'a ast::TaskDef, signature: &Signature) {
        let span = def.sig.name.span;
        let name = ast::short_name(&def.sig.name.text);
        // `std` alone reaches the runtime, and passes it counted values.
        let std = def.sig.name.text.starts_with("@std.");
        let plain = |ty: Ty| {
            ty.is_number() || matches!(ty, Ty::Bool | Ty::Pointer) || (std && ty.is_counted())
        };
        let problem = if name.starts_with("lucid_") && !std {
            Some("the runtime keeps names that start with `lucid_`")
        } else if !signature.type_params.is_empty() {
            Some("it cannot be generic")
        } else if signature.params.iter().any(|p| p.name == "self") {
            Some("it cannot take `self`")
        } else if signature.params.iter().any(|p| p.default.is_some()) {
            Some("a parameter cannot have a default")
        } else if signature.params.iter().any(|p| p.mode != ast::ParamMode::Read) {
            Some("a parameter cannot be `change` or `take`")
        } else if !signature.params.iter().all(|p| plain(p.ty) || p.ty == Ty::Error) {
            Some("a parameter must be a number, `bool`, or `pointer[byte]`")
        } else if !plain(signature.returns) && !matches!(signature.returns, Ty::None | Ty::Error) {
            Some("the result must be a number, `bool`, `pointer[byte]`, or nothing")
        } else if self.symbols.get(name).is_some_and(|(params, returns)| {
            *returns != signature.returns
                || !params.iter().copied().eq(signature.params.iter().map(|p| p.ty))
        }) {
            Some("another module declares this symbol with a different form")
        } else {
            let params = signature.params.iter().map(|p| p.ty).collect();
            self.symbols.insert(name, (params, signature.returns));
            if !std && find_symbol(name).is_none() {
                let message = format!("cannot find the foreign task `{name}`");
                let note = "it must be in the C runtime or `kernel32`";
                self.report(Diagnostic::new("E0904", message, span).note(note));
            }
            None
        };
        if let Some(problem) = problem {
            let diag = Diagnostic::new("E0331", format!("`{name}` cannot be `extern`"), span);
            self.report(diag.note(problem));
        }
    }

    /// The task behind an `extern`. It passes its parameters on, so a call
    /// goes through the same path as any task.
    fn foreign_task(&mut self, id: usize, def: &ast::TaskDef, span: Span) -> Task {
        let locals: Vec<Ty> = self.signatures[id].params.iter().map(|p| p.ty).collect();
        let returns = self.signatures[id].returns;
        let read = |(local, &ty)| Expr { kind: ExprKind::Local(local), ty, span };
        let args = locals.iter().enumerate().map(read).collect();
        let name: &'static str = def.sig.name.text.clone().leak();
        let symbol = ast::short_name(name);
        let call = Expr { kind: ExprKind::Foreign(symbol, args), ty: returns, span };
        let (captures, change, take, borrowed) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        Task {
            name,
            params: locals.len(),
            type_params: 0,
            returns,
            locals,
            body: vec![Stmt::Return(call)],
            span,
            captures,
            change,
            take,
            borrowed,
        }
    }

    /// Reports `what` outside an `unsafe` block.
    fn need_unsafe(&mut self, what: &str, span: Span) {
        if !self.frame.in_unsafe {
            let diag = Diagnostic::new("E0330", format!("{what} needs an `unsafe` block"), span);
            self.report(diag);
        }
    }

    /// Reports `unsafe` code in a package that lacks the `unsafe` permission.
    fn need_permission(&mut self, what: &str, span: Span) {
        let file = self.files[file_index(self.files, span.start)].start;
        if let Some((_, manifest)) = self.no_unsafe.iter().find(|(start, _)| *start == file) {
            let message = format!("{what} needs the `unsafe` permission");
            let note = format!("add it under `[permissions]` in `{manifest}`");
            self.report(Diagnostic::new("E0340", message, span).note(note));
        }
    }

    /// `unsafe`. The comment that security.md asks for comes first, or at the
    /// end of the line.
    fn unsafe_block(&mut self, body: &'a [ast::Stmt], span: Span) -> Stmt {
        self.unsafe_sites.push(("block", span));
        self.need_permission("an `unsafe` block", span);
        // Only this file: a package's manifest sits just before its entry file.
        let file = self.files[file_index(self.files, span.start)].start;
        if !has_comment(&self.src[file..], span.start - file) {
            let diag = Diagnostic::new("E0332", "this `unsafe` block has no comment", span);
            self.report(diag.note("say why it is sound, in a `#` line above it"));
        }
        let outer = std::mem::replace(&mut self.frame.in_unsafe, true);
        let body = self.block(body);
        self.frame.in_unsafe = outer;
        Stmt::Block(body)
    }

    /// The name of a `test`, which must be plain and new.
    fn test_name(&mut self, name: &ast::Expr, tests: &[TestCase]) -> String {
        let text = match &name.kind {
            Ast::Text(parts) => match parts.as_slice() {
                [] => Some(String::new()),
                [TextPart::Literal(text)] => Some(text.clone()),
                _ => None,
            },
            _ => None,
        };
        let Some(text) = text else {
            self.fail("E0329", "a `test` name cannot hold `{}`".into(), name.span);
            return String::new();
        };
        if tests.iter().any(|test| test.name == text) {
            self.fail("E0329", format!("there are two tests named \"{text}\""), name.span);
        }
        text
    }

    /// A `test` is a task with no parameters. It sees what a task sees.
    fn test(&mut self, name: &str, body: &'a [ast::Stmt], span: Span) -> Task {
        self.frame.returns = Some(Ty::None);
        self.frame.in_test = true;
        self.frame.scopes.push(HashMap::new());
        let mut body = self.stmts(body);
        self.frame.scopes.pop();
        self.check_errors_read();
        self.open_spawns(&mut body);
        self.frame.in_test = false;
        self.frame.mutable.clear();
        let name: &'static str = format!("test \"{name}\"").leak();
        let locals = std::mem::take(&mut self.frame.locals);
        let (captures, change, take) = (Vec::new(), Vec::new(), Vec::new());
        let (params, type_params, returns, borrowed) = (0, 0, Ty::None, Vec::new());
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

    fn main(&mut self, file: &'a ast::File) -> Task {
        self.frame.returns = None;
        self.frame.scopes.push(HashMap::new());
        let mut body = self.stmts(&file.stmts);
        self.check_errors_read();
        self.open_spawns(&mut body);
        let span = Span { start: 0, end: 0 };
        let locals = std::mem::take(&mut self.frame.locals);
        let (captures, change, take) = (Vec::new(), Vec::new(), Vec::new());
        let (params, type_params, returns, borrowed) = (0, 0, Ty::None, Vec::new());
        let name = "top level";
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

    fn new_local(&mut self, ty: Ty, mutable: bool) -> Local {
        self.frame.locals.push(ty);
        self.frame.mutable.push(mutable);
        self.frame.locals.len() - 1
    }

    /// Adds a name to the innermost scope. Shadowing there is an error.
    fn bind(&mut self, name: &'a ast::Name, ty: Ty, mutable: bool) -> Local {
        let text = name.text.as_str();
        let top_level = self.frame.returns.is_none() && self.frame.scopes.len() == 1;
        let scope = self.frame.scopes.last().expect("a scope is open");
        let global = self.task_ids.contains_key(text) || self.constructors.contains_key(text);
        if scope.contains_key(text) || (top_level && global) {
            self.fail("E0304", format!("`{text}` is already defined"), name.span);
        }
        let local = self.new_local(ty, mutable);
        self.frame.scopes.last_mut().expect("a scope is open").insert(text, local);
        local
    }

    /// `bind` for `let` and `var`, where an error must then be read.
    fn bind_value(&mut self, name: &'a ast::Name, ty: Ty, mutable: bool) -> Local {
        let local = self.bind(name, ty, mutable);
        // Tasks read a top-level literal from its source, not through this local.
        let top = self.frame.returns.is_none() && self.frame.scopes.len() == 1;
        let literal = top && matches!(self.top.get(name.text.as_str()), Some(Top::Literal(..)));
        if !literal && !self.error_members(ty).is_empty() {
            self.frame.kept_errors.push((local, name.span));
        }
        local
    }

    /// Adds a fix to E0301 if one name in sight is a close spelling. Two names
    /// at the same distance make the guess unsure, so neither is offered.
    fn similar(&self, diag: Diagnostic, name: &str, span: Span) -> Diagnostic {
        // Only names that would work here: a task reads a top-level literal
        // `let` and no other, and top-level code sees a top-level name only
        // once declared, so already in `scopes`.
        let in_task = self.frame.returns.is_some();
        let top = self.top.iter().filter(|(_, top)| in_task && matches!(top, Top::Literal(..)));
        let locals = self.frame.scopes.iter().flat_map(|scope| scope.keys().copied());
        let names = locals.chain(top.map(|(name, _)| *name)).chain(self.task_ids.keys().copied());
        let names = names.chain(self.constructors.keys().copied());
        // As rustc does: a third of the length, and at least one. A change to
        // every character is no longer a spelling.
        let length = name.chars().count();
        let limit = (length.max(3) / 3).min(length - 1);
        let mut best: Vec<(usize, &str)> = names
            .filter(|other| other.chars().all(|c| c.is_alphanumeric() || c == '_'))
            .map(|other| (crate::diag::edit_distance(name, other), other))
            .filter(|&(distance, _)| distance > 0 && distance <= limit)
            .collect();
        best.sort();
        best.dedup();
        match best.as_slice() {
            [(distance, other), rest @ ..]
                if rest.first().is_none_or(|next| next.0 > *distance) =>
            {
                diag.fix(format!("rename to `{other}`"), span, *other)
            }
            _ => diag,
        }
    }

    fn lookup(&self, name: &str) -> Option<Local> {
        self.frame.scopes.iter().rev().find_map(|scope| scope.get(name).copied())
    }

    fn block(&mut self, stmts: &'a [ast::Stmt]) -> Vec<Stmt> {
        self.frame.scopes.push(HashMap::new());
        let mut stmts = self.stmts(stmts);
        self.close_scope(&mut stmts, None);
        stmts
    }

    /// Ends the innermost scope. If control can reach its end, each name in it
    /// lets go of its value, newest first. `keep` stays, for a loop to handle.
    fn close_scope(&mut self, stmts: &mut Vec<Stmt>, keep: Option<Local>) {
        let scope = self.frame.scopes.pop().expect("a scope is open");
        if !ends(stmts, true) {
            let mut names: Vec<Local> = scope.into_values().filter(|&l| Some(l) != keep).collect();
            names.sort_unstable_by_key(|&local| std::cmp::Reverse(local));
            stmts.extend(names.into_iter().map(Stmt::Clear));
        }
    }

    /// An error bound to a name must be read somewhere in its task.
    fn check_errors_read(&mut self) {
        for (local, span) in std::mem::take(&mut self.frame.kept_errors) {
            self.unused_error(self.frame.locals[local], span);
        }
    }

    fn mark_read(&mut self, local: Local) {
        self.frame.kept_errors.retain(|&(kept, _)| kept != local);
    }

    fn unused_error(&mut self, ty: Ty, span: Span) {
        let message = format!("this `{}` is an error that nothing uses", ty.name());
        let note = "use `try`, `else`, or `match`";
        self.report(Diagnostic::new("E0328", message, span).note(note));
    }

    /// What a `break` or `continue` leaves: each name from the scopes of the
    /// open loop's body, newest first. `Stmt::Clear` of a plain number is
    /// dropped later.
    fn leaving(&self) -> Vec<Stmt> {
        let &(depth, keep, held) = self.frame.loop_scopes.last().expect("the checker found a loop");
        let names = self.frame.scopes[depth..].iter().flat_map(|scope| scope.values().copied());
        let mut names: Vec<Local> = names.filter(|&local| Some(local) != keep).collect();
        names.extend_from_slice(&self.frame.held[held..]);
        names.sort_unstable_by_key(|&local| std::cmp::Reverse(local));
        names.into_iter().map(Stmt::Clear).collect()
    }

    fn stmts(&mut self, stmts: &'a [ast::Stmt]) -> Vec<Stmt> {
        let start = self.frame.narrowed.len();
        let stmts = stmts.iter().filter_map(|stmt| self.stmt(stmt)).collect();
        self.frame.narrowed.truncate(start);
        stmts
    }

    fn condition(&mut self, value: &'a ast::Expr, what: &str) -> Expr {
        let value = self.expr(value);
        self.expect(&value, Ty::Bool, what);
        value
    }

    fn stmt(&mut self, stmt: &'a ast::Stmt) -> Option<Stmt> {
        self.check_busy_reads(stmt);
        let start = self.frame.changed.len();
        let held = self.frame.held.len();
        let lowered = self.statement(stmt);
        self.check_changed(stmt, start);
        let clears = self.frame.held.split_off(held);
        match lowered {
            Some(lowered) if !clears.is_empty() => Some(Stmt::Block(
                std::iter::once(lowered).chain(clears.into_iter().map(Stmt::Clear)).collect(),
            )),
            lowered => lowered,
        }
    }

    /// Nothing may touch what a `for change` has taken out.
    fn check_busy_reads(&mut self, stmt: &'a ast::Stmt) {
        if !self.frame.busy.iter().any(|busy| busy.exclusive) {
            return;
        }
        let mut uses = Vec::new();
        stmt.values().into_iter().flatten().for_each(|value| value.uses(&mut uses));
        for found in uses {
            let Some(local) = self.lookup(found.root) else { continue };
            let clash = |busy: &Busy| {
                busy.exclusive && busy.local == local && ast::overlap(&busy.path, &found.path)
            };
            if self.frame.busy.iter().any(clash) {
                let root = found.root;
                let message = format!("`{root}` cannot be used while `for change` visits it");
                let diag = Diagnostic::new("E0326", message, found.span);
                self.report(diag.note("use the loop name, or move this out of the loop"));
            }
        }
    }

    /// A value that a statement reads may be a borrow that lasts to its end, so
    /// no other part of it may use what a `change` argument changes.
    fn check_changed(&mut self, stmt: &'a ast::Stmt, start: usize) {
        let changed = self.frame.changed.split_off(start);
        if changed.is_empty() {
            return;
        }
        // A write reads its way down first, so a path beside it is not safe.
        let mut writes: Vec<Span> = changed.iter().map(|change| change.span).collect();
        writes.extend(stmt.target().map(|target| target.span));
        for group in stmt.values() {
            let mut uses = Vec::new();
            group.iter().for_each(|value| value.uses(&mut uses));
            let held = |change: &Changed| {
                uses.iter().any(|found| {
                    found.root == change.root
                        && change.peers.iter().any(|&peer| contains(peer, found.span))
                })
            };
            for change in changed.iter().filter(|change| held(change)) {
                let clash = uses.iter().find(|found| {
                    found.root == change.root
                        && (ast::overlap(&found.path, &change.path) || writes.contains(&found.span))
                        && !change.peers.iter().any(|&peer| contains(peer, found.span))
                });
                let Some(found) = clash else { continue };
                if self.diags.iter().any(|diag| diag.code == "E0326" && diag.span == found.span) {
                    continue;
                }
                let root = found.root;
                let message = format!("`{root}` is used here while this statement changes it");
                let diag = Diagnostic::new("E0326", message, found.span);
                self.report(diag.note("split it in two, and name the value with `let`"));
            }
        }
    }

    fn statement(&mut self, stmt: &'a ast::Stmt) -> Option<Stmt> {
        let span = stmt.span;
        Some(match &stmt.kind {
            StmtKind::Task(def) => {
                if self.frame.returns.is_some() || self.frame.scopes.len() > 1 {
                    return self.nested_task(def, span);
                }
                return None;
            }
            StmtKind::Let { mutable, name, ty, value } => {
                let want = ty.as_ref().map(|ty| self.ty(ty));
                if let Some((held, result)) = self.statement_value(value, want) {
                    let ty = want.unwrap_or(self.frame.locals[result]);
                    let read = self.held_value(result, ty, "this annotation", value.span);
                    let set = Stmt::Set(self.bind_value(name, ty, *mutable), read);
                    return Some(self.with_held(vec![held, set], result));
                }
                let value = self.expected(value, want);
                let ty = want.unwrap_or(value.ty);
                self.expect(&value, ty, "this annotation");
                Stmt::Set(self.bind_value(name, ty, *mutable), value)
            }
            StmtKind::Assign { target, op, value } => return self.assign(target, *op, value, span),
            StmtKind::If { branches, otherwise } => self.if_stmt(branches, otherwise.as_deref()),
            StmtKind::While { condition, body } => {
                self.forget_assigned(body);
                let start = self.frame.narrowed.len();
                let test = self.condition(condition, "`while`");
                let facts = self.facts(condition, true);
                self.frame.narrowed.extend(facts);
                let held = self.frame.held.len();
                self.frame.loop_scopes.push((self.frame.scopes.len(), None, held));
                let body = self.block(body);
                self.frame.loop_scopes.pop();
                self.frame.narrowed.truncate(start);
                Stmt::While { condition: test, body }
            }
            StmtKind::For { change, name, iterable, body } => {
                self.for_loop(*change, name, iterable, body)
            }
            StmtKind::Return(value) => {
                let none = Expr { kind: ExprKind::None, ty: Ty::None, span };
                let mut matched = None;
                let value = match value {
                    Some(value) => match self.statement_value(value, self.frame.returns) {
                        Some((stmt, result)) => {
                            matched = Some(stmt);
                            let read = self.read(result, value.span);
                            match self.frame.returns {
                                Some(want) => self.widen(read, want),
                                None => read,
                            }
                        }
                        None => self.expected(value, self.frame.returns),
                    },
                    None => none,
                };
                match self.frame.returns {
                    Some(returns) => self.expect(&value, returns, "this `return`"),
                    None => {
                        self.fail("E0310", "`return` is only allowed in a task".into(), span);
                    }
                }
                match matched {
                    Some(matched) => Stmt::Block(vec![matched, Stmt::Return(value)]),
                    None => Stmt::Return(value),
                }
            }
            StmtKind::Print(value) => match &value.kind {
                Ast::Text(parts) => match parts.as_slice() {
                    [] => Stmt::PrintText(String::new()),
                    [TextPart::Literal(text)] => Stmt::PrintText(text.clone()),
                    _ => Stmt::Print(self.expr(value)),
                },
                _ => {
                    let value = self.expr(value);
                    Stmt::Print(self.showable(value))
                }
            },
            StmtKind::Break | StmtKind::Continue if self.frame.loop_scopes.is_empty() => {
                self.fail("E0311", "`break` and `continue` need a loop".into(), span);
                return None;
            }
            StmtKind::Break | StmtKind::Continue => {
                let mut stmts = self.leaving();
                let breaks = matches!(stmt.kind, StmtKind::Break);
                stmts.push(if breaks { Stmt::Break } else { Stmt::Continue });
                Stmt::Block(stmts)
            }
            StmtKind::Placeholder => Stmt::Placeholder(span),
            StmtKind::Expr(value) => return self.expr_stmt(value, span),
            StmtKind::Use { .. } => return self.later("`use`", span),
            StmtKind::Test { .. }
            | StmtKind::Type(_)
            | StmtKind::Choice(_)
            | StmtKind::Trait(_) => return None,
            StmtKind::Expect(value) => self.expect_stmt(value, span),
            StmtKind::Unsafe(body) => self.unsafe_block(body, span),
        })
    }

    /// Starts a lambda or nested task. Local 0 is its record, of type `ty`,
    /// which a lambda with an open return type fills in at `end_closure`.
    fn start_closure(&mut self, ty: Ty, returns: Ty) {
        self.frames.push(std::mem::take(&mut self.frame));
        self.frame.returns = Some(returns);
        self.new_local(ty, false);
        self.frame.scopes.push(HashMap::new());
    }

    /// Moves each counted parameter of a task value to a local of its own,
    /// and so does a `take` one, whose local can change. A task value owns
    /// what it is passed, by decision 183, so `end_closure` lists each in
    /// `Task::take`. Gives the moves, which start the body.
    fn own_closure_params(&mut self, names: &[(&'a ast::Name, bool)]) -> Vec<Stmt> {
        let mut moves = Vec::new();
        for (index, &(name, take)) in names.iter().enumerate() {
            let param = index + 1;
            if take || self.frame.locals[param].is_counted() {
                let own = self.new_local(self.frame.locals[param], take);
                let scope = self.frame.scopes.last_mut().expect("a scope is open");
                scope.insert(&name.text, own);
                moves.push(Stmt::Set(own, self.read(param, name.span)));
            }
        }
        moves
    }

    /// Ends what `start_closure` began. Gives the new value, which the task
    /// around it builds from what the body captured.
    fn end_closure(&mut self, params: &[Ty], returns: Ty, mut body: Vec<Stmt>, span: Span) -> Expr {
        self.open_spawns(&mut body);
        let outer = self.frames.pop().expect("`start_closure` pushed a frame");
        let frame = std::mem::replace(&mut self.frame, outer);
        let ty = Ty::Task(params.to_vec().leak(), inner(returns));
        let mut locals = frame.locals;
        locals[0] = ty;
        let values: Vec<Expr> = frame.captures.into_iter().map(|(_, value)| value).collect();
        let captures = values.iter().map(|value| value.ty).collect();
        let (type_params, args) = self.own_type_args();
        let params = params.len() + 1;
        let take = (1..params).filter(|&param| locals[param].is_counted()).collect();
        let (change, borrowed) = (Vec::new(), Vec::new());
        let task = Task {
            name: "lambda",
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
        };
        self.lifted.push(task);
        let task = self.first_lifted + self.lifted.len() - 1;
        let fields = values.into_iter().enumerate().collect();
        Expr { kind: ExprKind::Closure { task, args, counted: 0, fields }, ty, span }
    }

    /// How many type parameters are in scope, and each as a type argument. A
    /// lifted task has those of the task around it.
    fn own_type_args(&self) -> (usize, &'static [Ty]) {
        let args: Vec<Ty> = self.type_params.iter().map(|&(_, ty)| ty).collect();
        (args.len(), args.leak())
    }

    /// A task inside a task. It is a value made at its line, under a name that
    /// cannot change, so it needs a name that is already bound. It reads its
    /// own name as its own record, which keeps it from holding itself.
    fn nested_task(&mut self, def: &'a ast::TaskDef, span: Span) -> Option<Stmt> {
        let Some(body) = &def.body else {
            let diag = Diagnostic::new("E0331", "an `extern` task must be at top level", span);
            self.report(diag);
            return None;
        };
        let sig = &def.sig;
        let wrong = "a nested task takes no type parameters, `self`, defaults, or `change`";
        let plain = |p: &ast::Param| {
            p.name.text != "self" && p.default.is_none() && p.mode != ast::ParamMode::Change
        };
        if !sig.type_params.is_empty() || !sig.params.iter().all(plain) {
            self.fail("E0307", wrong.into(), sig.name.span);
        }
        let mut params = Vec::with_capacity(sig.params.len());
        for param in &sig.params {
            params.push(param.ty.as_ref().map_or(Ty::Error, |ty| self.ty(ty)));
        }
        let returns = sig.returns.as_ref().map_or(Ty::None, |ty| self.ty(ty));
        let ty = Ty::Task(params.clone().leak(), inner(returns));
        self.start_closure(ty, returns);
        self.frame.own_name = Some(&sig.name.text);
        let mut lowered = Vec::new();
        for (param, &ty) in sig.params.iter().zip(&params) {
            self.bind(&param.name, ty, false);
        }
        self.frame.params = self.frame.locals.len();
        let names: Vec<(&'a ast::Name, bool)> =
            sig.params.iter().map(|p| (&p.name, p.mode == ast::ParamMode::Take)).collect();
        lowered.extend(self.own_closure_params(&names));
        lowered.extend(self.stmts(body));
        let mut body = lowered;
        self.check_returns(&sig.name, returns, &mut body);
        let value = self.end_closure(&params, returns, body, span);
        Some(Stmt::Set(self.bind(&sig.name, ty, false), value))
    }

    /// `x => x + 1`, with the types of its parameters and of its result from
    /// the context. A lambda with no such context is E0324.
    fn lambda(
        &mut self,
        params: &'a [ast::Name],
        body: &'a ast::Expr,
        want: Option<Ty>,
        span: Span,
    ) -> Expr {
        let want = self.literal_want(want, |ty| matches!(ty, Ty::Task(..)));
        match want {
            Some(Ty::Task(types, returns)) if types.len() == params.len() => {
                self.lambda_with(params, body, types, Some(*returns), span)
            }
            Some(Ty::Task(types, _)) => {
                let (want, found) = (types.len(), params.len());
                let message = format!("this lambda has {found} parameters, and needs {want}");
                self.fail("E0307", message, span)
            }
            Some(Ty::Error) => poisoned(span),
            _ => {
                let diag = Diagnostic::new("E0324", "a lambda needs a task type here", span);
                self.report(diag.note(
                    "write its type, as in `let f: task(int) -> int = x => x`, or pass it after the values that show its type",
                ))
            }
        }
    }

    fn lambda_with(
        &mut self,
        params: &'a [ast::Name],
        body_expr: &'a ast::Expr,
        types: &[Ty],
        returns: Option<Ty>,
        span: Span,
    ) -> Expr {
        self.start_closure(Ty::Error, returns.unwrap_or(Ty::Error));
        self.frame.open_result = returns.is_none();
        for (param, &ty) in params.iter().zip(types) {
            self.bind(param, ty, false);
        }
        let names: Vec<(&'a ast::Name, bool)> = params.iter().map(|name| (name, false)).collect();
        let mut body = self.own_closure_params(&names);
        let value = self.expected(body_expr, returns);
        let result = returns.unwrap_or(value.ty);
        self.expect(&value, result, "this lambda");
        body.push(Stmt::Return(value));
        self.end_closure(types, result, body, span)
    }

    /// True if a name of a local means something in the task being checked, or
    /// in one around it.
    fn visible(&self, name: &str) -> bool {
        let in_frame = |frame: &Frame| {
            frame.own_name == Some(name) || frame.scopes.iter().any(|s| s.contains_key(name))
        };
        self.lookup(name).is_some()
            || self.frame.own_name == Some(name)
            || self.frames.iter().any(in_frame)
    }

    /// Reads a name of a task around this one, which this task then captures.
    /// `None` if no such task has the name.
    fn captured(&mut self, name: &str, span: Span) -> Option<Expr> {
        let kind = ExprKind::Local(0);
        if self.frame.own_name == Some(name) {
            return Some(Expr { kind, ty: self.frame.locals[0], span });
        }
        if let Some(index) = self.frame.captures.iter().position(|(known, _)| known == name) {
            let ty = self.frame.captures[index].1.ty;
            return Some(Expr { kind: ExprKind::Capture(index), ty, span });
        }
        let outer = self.frames.pop()?;
        let inside = std::mem::replace(&mut self.frame, outer);
        let found = match self.lookup(name) {
            Some(local) if self.frame.mutable[local] => {
                let message = format!("`{name}` can change, so it cannot be captured");
                let note = format!("copy it first, with `let copy = {name}`");
                Some(self.report(Diagnostic::new("E0325", message, span).note(note)))
            }
            Some(local) => {
                self.mark_read(local);
                Some(view(self.read(local, span), self.current(local)))
            }
            None => self.captured(name, span),
        };
        self.frames.push(std::mem::replace(&mut self.frame, inside));
        let value = found?;
        if value.ty == Ty::Error {
            return Some(value);
        }
        let ty = value.ty;
        self.frame.captures.push((name.into(), value));
        Some(Expr { kind: ExprKind::Capture(self.frame.captures.len() - 1), ty, span })
    }

    /// A task used as a value. Its code is a small task that takes a record
    /// first, like a lambda, and calls the real one.
    fn task_value(&mut self, id: usize, name: &str, explicit: Option<&[Ty]>, span: Span) -> Expr {
        if self.foreign.contains(&id) {
            // A task type says nothing of `unsafe`, so a value could leave the block.
            let message = format!("`{name}` is an `extern` task, which is not a value");
            let diag = Diagnostic::new("E0331", message, span);
            self.report(diag.note("call it in an `unsafe` block"));
        }
        let params: Vec<&str> =
            self.signatures[id].type_params.iter().map(|&(name, _)| name).collect();
        let known: Vec<Option<Ty>> = match explicit {
            Some(types) => types.iter().copied().map(Some).collect(),
            None => vec![None; params.len()],
        };
        let Some(args) = self.solved(name, &params, &known, self.diags.len(), span) else {
            return poisoned(span);
        };
        let bounds = self.signatures[id].bounds.clone();
        for (&ty, traits) in args.iter().zip(bounds) {
            for name in traits {
                self.require(ty, name, span);
            }
        }
        if self.changes(id) {
            let message = format!("`{name}` has a `change` parameter, so it has no value form");
            return self.fail("E0307", message, span);
        }
        let signature = &self.signatures[id];
        let types: Vec<Ty> = signature.params.iter().map(|p| p.ty).collect();
        let returns = signature.returns;
        let ty = Ty::Task(types.clone().leak(), inner(returns));
        let task = match self.task_values.get(&id) {
            Some(&task) => task,
            None => {
                let own: Vec<Ty> = signature.type_params.iter().map(|&(_, ty)| ty).collect();
                // Each counted parameter moves to a local, which the call
                // borrows or takes, by decision 183.
                let mut locals = vec![ty];
                locals.extend(&types);
                let params = locals.len();
                let (mut body, mut values, mut take) = (Vec::new(), Vec::new(), Vec::new());
                for (index, &ty) in types.iter().enumerate() {
                    let mut value = Expr { kind: ExprKind::Local(index + 1), ty, span };
                    if ty.is_counted() {
                        take.push(index + 1);
                        locals.push(ty);
                        body.push(Stmt::Set(locals.len() - 1, value));
                        value = Expr { kind: ExprKind::Local(locals.len() - 1), ty, span };
                    }
                    values.push(value);
                }
                let kind = ExprKind::Call(id, own.clone().leak(), values);
                let call = Expr { kind, ty: returns, span };
                let type_params = own.len();
                let (captures, change, borrowed) = (Vec::new(), Vec::new(), Vec::new());
                body.push(Stmt::Return(call));
                let code = Task {
                    name: "",
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
                };
                self.lifted.push(code);
                let task = self.first_lifted + self.lifted.len() - 1;
                self.task_values.insert(id, task);
                task
            }
        };
        let kind = ExprKind::Closure { task, args, counted: 0, fields: Vec::new() };
        Expr { kind, ty: self.table.subst(ty, args), span }
    }

    /// Calls a task value. Its type has no names, so arguments are positional.
    fn call_value(&mut self, value: Expr, args: &'a [ast::Arg], span: Span) -> Expr {
        let Ty::Task(params, returns) = value.ty else {
            self.poison_args(args, span);
            if value.ty == Ty::Error {
                return poisoned(span);
            }
            let message = format!("`{}` is not a task", value.ty.name());
            return self.fail("E0308", message, value.span);
        };
        if args.len() != params.len() || args.iter().any(|a| a.change || a.name.is_some()) {
            self.poison_args(args, span);
            let (count, plural) = (params.len(), if params.len() == 1 { "" } else { "s" });
            let message = format!("this task takes {count} positional argument{plural}");
            return self.fail("E0307", message, span);
        }
        let mut values = Vec::with_capacity(args.len());
        for (arg, &want) in args.iter().zip(params) {
            let value = self.expected(&arg.value, Some(want));
            self.expect(&value, want, "this argument");
            values.push(value);
        }
        Expr { kind: ExprKind::CallValue(value.into(), values), ty: *returns, span }
    }

    /// Later branches, and any code after an `if` whose branches all leave,
    /// know that the earlier conditions were false.
    fn if_stmt(
        &mut self,
        branches: &'a [(ast::Expr, Vec<ast::Stmt>)],
        otherwise: Option<&'a [ast::Stmt]>,
    ) -> Stmt {
        let start = self.frame.narrowed.len();
        let mut lowered = Vec::with_capacity(branches.len());
        for (condition, then) in branches {
            let test = self.condition(condition, "`if`");
            let (found, rest) = (self.facts(condition, true), self.facts(condition, false));
            let outer = self.frame.narrowed.len();
            self.frame.narrowed.extend(found);
            let then = self.block(then);
            self.frame.narrowed.truncate(outer);
            self.frame.narrowed.extend(rest);
            lowered.push((test, then));
        }
        let otherwise = otherwise.map_or(Vec::new(), |body| self.block(body));
        if !lowered.iter().all(|(_, then)| ends(then, true)) {
            self.frame.narrowed.truncate(start);
        }
        Stmt::If { branches: lowered, otherwise }
    }

    /// `expect`. A comparison keeps both sides in locals, to show them if it
    /// fails. What passes narrows what follows, like an `if` that ends.
    fn expect_stmt(&mut self, value: &'a ast::Expr, span: Span) -> Stmt {
        let test = self.condition(value, "`expect`");
        let facts = self.facts(value, true);
        self.frame.narrowed.extend(facts);
        let mut stmts = Vec::new();
        let shows = |checker: &mut Self, a: &Expr, b: &Expr| {
            checker.satisfies(a.ty, "Show") && checker.satisfies(b.ty, "Show")
        };
        let (test, failed) = match test.kind {
            ExprKind::Binary(op, left, right) if shows(self, &left, &right) => {
                let left = self.keep(*left, &mut stmts);
                let right = self.keep(*right, &mut stmts);
                let word = format!(" {} ", symbol(op));
                let text = |text: String| Expr { kind: ExprKind::Text(text), ty: Ty::Text, span };
                let parts = vec![
                    text("expect failed: ".into()),
                    shown_side(left.clone()),
                    text(word),
                    shown_side(right.clone()),
                ];
                let message = Expr { kind: ExprKind::Join(parts), ty: Ty::Text, span };
                let kind = ExprKind::Binary(op, left.into(), right.into());
                (Expr { kind, ..test }, Stmt::Fail(message, span))
            }
            kind => (Expr { kind, ..test }, Stmt::Trap("expect failed", span)),
        };
        stmts.push(Stmt::If { branches: vec![(test, Vec::new())], otherwise: vec![failed] });
        Stmt::Block(stmts)
    }

    /// Runs `value` into a hidden local that lasts for this statement.
    fn keep(&mut self, value: Expr, stmts: &mut Vec<Stmt>) -> Expr {
        let (ty, span) = (value.ty, value.span);
        let local = self.new_local(ty, false);
        self.frame.held.push(local);
        stmts.push(Stmt::Set(local, value));
        Expr { kind: ExprKind::Local(local), ty, span }
    }

    /// Lowers a value that needs its own statements first, which only the
    /// whole value of a statement can have: a `match`, or a `pop`, which must
    /// not change a list while the same statement borrows from it. Gives those
    /// statements, and the local that holds the value.
    fn statement_value(&mut self, value: &'a ast::Expr, want: Option<Ty>) -> Option<(Stmt, Local)> {
        match &value.kind {
            Ast::Match { subject, arms } => Some(self.matching(subject, arms, Some(want))),
            Ast::Call { callee, args } => match &callee.kind {
                Ast::Field { base, name } if name.text == "pop" && self.is_builtin(base, "pop") => {
                    Some(self.pop(base, args, value.span))
                }
                Ast::Field { base, name }
                    if name.text == "receive" && self.is_builtin(base, "receive") =>
                {
                    let channel = self.expr(base);
                    Some(self.receive(channel, args, value.span))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Reads the local from `statement_value` as a `want`.
    fn held_value(&mut self, result: Local, want: Ty, what: &str, span: Span) -> Expr {
        let read = self.widen(self.read(result, span), want);
        self.expect(&read, want, what);
        read
    }

    /// Runs `stmts`, then lets go of the local from `statement_value`.
    fn with_held(&self, mut stmts: Vec<Stmt>, result: Local) -> Stmt {
        stmts.extend(self.frame.locals[result].is_counted().then_some(Stmt::Clear(result)));
        Stmt::Block(stmts)
    }

    fn expr_stmt(&mut self, value: &'a ast::Expr, span: Span) -> Option<Stmt> {
        Some(match &value.kind {
            Ast::Match { subject, arms } => self.matching(subject, arms, None).0,
            Ast::Call { callee, args } => match &callee.kind {
                Ast::Field { base, name }
                    if CHANGING_CALLS.contains(&name.text.as_str())
                        && self.is_builtin(base, &name.text) =>
                {
                    return self.changing_call(base, name, args, span);
                }
                Ast::Field { base, name }
                    if THREAD_CALLS.contains(&name.text.as_str())
                        && self.is_builtin(base, &name.text) =>
                {
                    self.thread_call(base, name, args, span)
                }
                _ => self.discarded(value),
            },
            _ => self.discarded(value),
        })
    }

    /// A value that a statement drops, which must not be an error.
    fn discarded(&mut self, value: &'a ast::Expr) -> Stmt {
        let value = self.expr(value);
        if !self.error_members(value.ty).is_empty() {
            self.unused_error(value.ty, value.span);
        }
        Stmt::Expr(value)
    }

    /// The members of a type that are error types.
    fn error_members(&mut self, ty: Ty) -> Vec<Ty> {
        let members = ty.members().iter().copied();
        members.filter(|&m| m != Ty::Error && m != Ty::None && self.satisfies(m, "Error")).collect()
    }

    /// Starts `try` or `else`: keeps `value` in a hidden local, and splits its
    /// type into what counts as failure and what is left. `none` is a failure
    /// only in a union with no error type. Reports E0327 if nothing can fail
    /// or nothing is left.
    fn hold_failure(&mut self, value: Expr, word: &str) -> Option<(Local, Stmt, Ty, Ty)> {
        let ty = value.ty;
        if ty == Ty::Error {
            return None;
        }
        let errors = self.error_members(ty);
        let failed = if errors.is_empty() { Ty::None } else { self.union(errors) };
        let Some((failed, rest)) = self.split(ty, failed) else {
            let message = format!("`{word}` has nothing to act on in `{}`", ty.name());
            let note = "it needs a union of an error type or `none` and another type";
            self.report(Diagnostic::new("E0327", message, value.span).note(note));
            return None;
        };
        let holder = self.new_local(ty, false);
        self.frame.held.push(holder);
        Some((holder, Stmt::Set(holder, value), failed, rest))
    }

    fn holder_has(&self, holder: Local, want: Ty, span: Span) -> Expr {
        let kind = ExprKind::Holds(self.read(holder, span).into(), want.members().to_vec());
        Expr { kind, ty: Ty::Bool, span }
    }

    /// `try x`. See "Early exits" in docs/compiler.md.
    fn try_value(&mut self, value: &'a ast::Expr, span: Span) -> Expr {
        let value = self.expr(value);
        let Some((holder, set, failed, rest)) = self.hold_failure(value, "try") else {
            return poisoned(span);
        };
        let test = self.holder_has(holder, failed, span);
        let leave = if self.frame.returns.is_none() || self.frame.in_test {
            self.exit_failure(holder, failed, span)
        } else {
            self.return_failure(holder, failed, span)
        };
        let leave = Stmt::If { branches: vec![(test, leave)], otherwise: Vec::new() };
        let kind = ExprKind::Seq(vec![set, leave], view(self.read(holder, span), rest).into());
        Expr { kind, ty: rest, span }
    }

    fn return_failure(&mut self, holder: Local, failed: Ty, span: Span) -> Vec<Stmt> {
        let want = self.frame.returns.expect("in a task");
        if self.frame.open_result {
            let message = "a lambda with `try` needs a result type from its context";
            self.fail("E0324", message.into(), span);
            return Vec::new();
        }
        let error = self.widen(view(self.read(holder, span), failed), want);
        if error.ty == want || want == Ty::Error {
            return vec![Stmt::Return(error)];
        }
        // A union stays out of an `any`, but each of its members can go in.
        let members = failed.members();
        let fits = |checker: &mut Self, member| {
            let error = checker.widen(view(checker.read(holder, span), member), want);
            (error.ty == want).then_some(Stmt::Return(error))
        };
        let (&last, others) = members.split_last().expect("a type has a member");
        let mut branches = Vec::with_capacity(others.len());
        let mut left = fits(self, last);
        for &member in others {
            let test = self.holder_has(holder, member, span);
            match fits(self, member) {
                Some(leave) => branches.push((test, vec![leave])),
                None => left = None,
            }
        }
        let Some(left) = left else {
            let message = format!("this `try` can return `{}`", failed.name());
            let note = format!("the task returns `{}`, which must accept it", want.name());
            self.report(Diagnostic::new("E0303", message, span).note(note));
            return Vec::new();
        };
        vec![Stmt::If { branches, otherwise: vec![left] }]
    }

    /// At top level, `try` prints the error and ends the program.
    fn exit_failure(&mut self, holder: Local, failed: Ty, span: Span) -> Vec<Stmt> {
        let members = failed.members();
        let (&last, others) = members.split_last().expect("a type has a member");
        let mut branches = Vec::with_capacity(others.len());
        for &member in others {
            let test = self.holder_has(holder, member, span);
            branches.push((test, vec![self.exit_with(holder, member, span)]));
        }
        let otherwise = vec![self.exit_with(holder, last, span)];
        vec![Stmt::If { branches, otherwise }]
    }

    fn exit_with(&mut self, holder: Local, member: Ty, span: Span) -> Stmt {
        if member == Ty::None {
            return Stmt::Exit(Expr { kind: ExprKind::Text("none".into()), ty: Ty::Text, span });
        }
        let error = view(self.read(holder, span), member);
        Stmt::Exit(self.method_call(error, None, "error_message", &[], span))
    }

    /// `x else y`. The value takes the type that the context asks for, if a
    /// value of `x` fits it, so that `y` can be wider.
    fn fallback(
        &mut self,
        value: &'a ast::Expr,
        other: &'a ast::Expr,
        want: Option<Ty>,
        span: Span,
    ) -> Expr {
        let value = self.expr(value);
        let Some((holder, set, failed, rest)) = self.hold_failure(value, "else") else {
            self.expected(other, want);
            return poisoned(span);
        };
        let kept = view(self.read(holder, span), rest);
        let kept = match want.map(|want| self.widen(kept.clone(), want)) {
            Some(wide) if Some(wide.ty) == want => wide,
            _ => kept,
        };
        let ty = kept.ty;
        let other = self.expected(other, Some(ty));
        self.expect(&other, ty, "`else`, to match the value,");
        let test = self.holder_has(holder, failed, span);
        let choice = Expr { kind: ExprKind::If(test.into(), other.into(), kept.into()), ty, span };
        Expr { kind: ExprKind::Seq(vec![set], choice.into()), ty, span }
    }

    /// Lowers a `match` to an `if` chain over a hidden local, which keeps the
    /// value fixed if an arm assigns the variable it came from. The checks
    /// below prove that the last arm is all that can remain, so it needs no
    /// test, which is also how `ends` sees that every path is covered.
    ///
    /// ponytail: tests run one by one, and a text pattern allocates. A jump
    /// table and static text can follow if M4 benchmarks ask for them.
    ///
    /// `value` is `None` for a statement, or else holds the type that the
    /// context needs, if known. Returns the local that receives the value.
    fn matching(
        &mut self,
        subject: &'a ast::Expr,
        arms: &'a [ast::Arm],
        value: Option<Option<Ty>>,
    ) -> (Stmt, Local) {
        let subject = self.expr(subject);
        let ty = subject.ty;
        let holder = self.new_local(ty, false);
        let result = self.new_local(Ty::None, false);
        let mut want = value.flatten();
        let mut covered = Covered::default();
        let mut lowered = Vec::with_capacity(arms.len());
        for arm in arms {
            if self.missing(ty, &covered).is_empty() {
                let message = "this arm can never run: earlier arms cover everything";
                self.fail("E0317", message.into(), arm.pattern.span);
            }
            self.frame.scopes.push(HashMap::new());
            let (test, mut body) = self.pattern(&arm.pattern, holder, ty, &mut covered);
            match &arm.body {
                ast::ArmBody::Expr(arm_value) if value.is_some() => {
                    let lowered = self.expected(arm_value, want);
                    let wanted = *want.get_or_insert(lowered.ty);
                    self.expect(&lowered, wanted, "this arm");
                    body.push(Stmt::Set(result, lowered));
                }
                ast::ArmBody::Expr(arm_value) => {
                    body.extend(self.expr_stmt(arm_value, arm_value.span));
                }
                ast::ArmBody::Stmt(stmt) => {
                    if value.is_some() && matches!(stmt.kind, StmtKind::Print(_)) {
                        let message = "this `match` needs a value, and `print` gives none";
                        self.fail("E0303", message.into(), stmt.span);
                    }
                    body.extend(self.stmt(stmt));
                }
                ast::ArmBody::Block(stmts) => body.extend(self.stmts(stmts)),
            }
            self.close_scope(&mut body, None);
            lowered.push((test, body));
        }
        let missing = self.missing(ty, &covered);
        if !missing.is_empty() && ty != Ty::Error && !covered.failed {
            let (message, note) = (
                format!("this `match` does not cover every `{}`", ty.name()),
                format!("add an arm for {}", missing.join(", ")),
            );
            let span = arms.last().map_or(subject.span, |arm| arm.pattern.span);
            self.report(Diagnostic::new("E0316", message, span).note(note));
        }
        self.frame.locals[result] = want.unwrap_or(Ty::None);

        let otherwise = lowered.pop().map_or(Vec::new(), |(_, body)| body);
        let span = subject.span;
        let always = || Expr { kind: ExprKind::Bool(true), ty: Ty::Bool, span };
        let branches =
            lowered.into_iter().map(|(test, body)| (test.unwrap_or_else(always), body)).collect();
        let mut stmts = vec![Stmt::Set(holder, subject), Stmt::If { branches, otherwise }];
        if ty.is_counted() {
            stmts.push(Stmt::Clear(holder));
        }
        (Stmt::Block(stmts), result)
    }

    /// What no arm has covered, in the words of a note. Empty if nothing is left.
    fn missing(&self, ty: Ty, covered: &Covered) -> Vec<String> {
        if covered.all {
            return Vec::new();
        }
        if matches!(ty, Ty::Union(_)) {
            return (ty.members().iter())
                .filter(|&&member| !self.covers(ty, member, covered))
                .map(|member| format!("`{}`", member.name()))
                .collect();
        }
        match self.choice_of(ty) {
            Some(id) => (self.datas[id].variants.iter().enumerate())
                .filter(|(index, _)| !covered.variants.contains(index))
                .map(|(_, variant)| format!("`{}`", variant.name))
                .collect(),
            None if ty == Ty::Bool => ["true", "false"]
                .into_iter()
                .filter(|literal| !covered.literals.contains(*literal))
                .map(|literal| format!("`{literal}`"))
                .collect(),
            None => vec!["`_`".into()],
        }
    }

    /// The `choice` that a value of this type belongs to, if any.
    /// Advice for a variant or type name matched straight on a union: match
    /// the member first. Names the choice that has the variant, if one does.
    fn union_member_note(&self, ty: Ty, name: &ast::Name) -> String {
        let choice = ty.members().iter().find_map(|&member| {
            let id = self.choice_of(member)?;
            let data = &self.datas[id];
            data.variants.iter().any(|variant| variant.name == name.text).then_some(data)
        });
        match choice {
            Some(data) => {
                // A choice from a module has a full path, which a pattern does not take.
                let short = data.name.text.rsplit('.').next().unwrap_or_default();
                format!(
                    "`{}` is a variant of `{short}`: match `e: {short}` first, then match `e` in that arm",
                    name.text
                )
            }
            None => format!("to test what the union holds, write `x: {}`", name.text),
        }
    }

    fn choice_of(&self, ty: Ty) -> Option<usize> {
        match ty {
            Ty::Data(_, id, _) if self.datas[id].choice => Some(id),
            _ => None,
        }
    }

    /// Finds a variant of a choice, or reports that the name is wrong. A
    /// misspelled variant must not become a name that catches everything.
    fn variant_index(&mut self, id: usize, name: &ast::Name) -> Option<usize> {
        let data = &self.datas[id];
        let found = data.variants.iter().position(|variant| variant.name == name.text);
        if found.is_none() {
            let message = format!("`{}` has no variant `{}`", data.name.text, name.text);
            let names: Vec<&str> = data.variants.iter().map(|variant| variant.name).collect();
            let note = format!("its variants are {}, and `_` matches the rest", names.join(", "));
            self.report(Diagnostic::new("E0318", message, name.span).note(note));
        }
        found
    }

    /// Lowers a pattern. Returns its test, or `None` if anything matches, and
    /// the statements that bind its names.
    fn pattern(
        &mut self,
        pattern: &'a ast::Pattern,
        holder: Local,
        ty: Ty,
        covered: &mut Covered,
    ) -> (Option<Expr>, Vec<Stmt>) {
        let span = pattern.span;
        match &pattern.kind {
            ast::PatternKind::Wildcard => {
                covered.all = true;
                (None, Vec::new())
            }
            ast::PatternKind::None => {
                self.member_pattern(Ty::None, None, holder, ty, covered, span)
            }
            ast::PatternKind::Typed { name, ty: written } => {
                let target = self.ty(written);
                self.member_pattern(target, Some(name), holder, ty, covered, span)
            }
            ast::PatternKind::Literal(value) => (self.literal(value, holder, ty, covered), vec![]),
            ast::PatternKind::Name(name) if self.choice_of(ty).is_some() => {
                self.variant_pattern(name, None, holder, ty, covered)
            }
            ast::PatternKind::Name(name)
                if matches!(ty, Ty::Union(_))
                    && self.constructor_names.contains(name.text.as_str()) =>
            {
                covered.failed = true;
                let message = format!("`{}` is not a `choice`", ty.name());
                let note = self.union_member_note(ty, name);
                self.report(Diagnostic::new("E0318", message, name.span).note(note));
                (None, Vec::new())
            }
            ast::PatternKind::Name(name) => {
                let left: Vec<Ty> = (ty.members().iter().copied())
                    .filter(|&member| !self.covers(ty, member, covered))
                    .collect();
                covered.all = true;
                self.check_binding(name);
                let bound = self.union(left);
                let local = self.bind(name, bound, false);
                (None, vec![Stmt::Set(local, view(self.read(holder, span), bound))])
            }
            ast::PatternKind::Variant { name, bindings } => {
                self.variant_pattern(name, Some(bindings), holder, ty, covered)
            }
        }
    }

    /// True if an arm has taken this member of a union.
    fn covers(&self, ty: Ty, member: Ty, covered: &Covered) -> bool {
        matches!(ty, Ty::Union(_)) && covered.variants.contains(&self.type_id(member))
    }

    /// A pattern on the type a union holds: `none`, or `name: type`.
    fn member_pattern(
        &mut self,
        target: Ty,
        name: Option<&'a ast::Name>,
        holder: Local,
        ty: Ty,
        covered: &mut Covered,
        span: Span,
    ) -> (Option<Expr>, Vec<Stmt>) {
        let fits = matches!(ty, Ty::Union(_))
            && target.members().iter().all(|member| ty.members().contains(member));
        covered.failed |= !fits;
        if !fits && ty != Ty::Error && target != Ty::Error {
            let (have, want) = (ty.name(), target.name());
            self.fail(
                "E0318",
                match ty {
                    Ty::Union(_) => format!("`{have}` never holds `{want}`"),
                    _ => format!("`{have}` is not a union, so `{want}` is nothing to test"),
                },
                span,
            );
        }
        if let Some(name) = name {
            self.check_binding(name);
        }
        let local = name.map(|name| self.bind(name, if fits { target } else { Ty::Error }, false));
        if !fits {
            return (None, Vec::new());
        }
        let ids: Vec<usize> = target.members().iter().map(|&member| self.type_id(member)).collect();
        let fresh = ids.iter().filter(|id| !covered.variants.contains(id)).count();
        covered.variants.extend(ids.iter().copied());
        if fresh == 0 && !self.missing(ty, covered).is_empty() {
            let message = format!("this arm can never run: earlier arms have `{}`", target.name());
            self.fail("E0317", message, span);
        }
        let bound = local.map(|local| Stmt::Set(local, view(self.read(holder, span), target)));
        let kind = ExprKind::Holds(self.read(holder, span).into(), target.members().to_vec());
        (Some(Expr { kind, ty: Ty::Bool, span }), bound.into_iter().collect())
    }

    /// A variant or type name in a binding would read as a test, and catch
    /// everything. See decision 52.
    fn check_binding(&mut self, name: &ast::Name) {
        if self.constructor_names.contains(name.text.as_str()) {
            let message = format!("`{}` cannot be bound here", name.text);
            let note = "it names a type or variant, and patterns do not nest, so use another name";
            self.report(Diagnostic::new("E0318", message, name.span).note(note));
        }
    }

    fn literal(
        &mut self,
        value: &'a ast::Expr,
        holder: Local,
        ty: Ty,
        covered: &mut Covered,
    ) -> Option<Expr> {
        let span = value.span;
        if !is_literal(value) {
            self.fail("E0318", "a text pattern cannot hold `{}`".into(), span);
            return None;
        }
        let value = self.lower(value, Some(ty));
        self.expect(&value, ty, "this pattern");
        if value.ty != ty {
            covered.failed = true;
            return None;
        }
        let key = match &value.kind {
            ExprKind::Int(number) => number.to_string(),
            ExprKind::Float(number) => number.to_bits().to_string(),
            ExprKind::Bool(flag) => flag.to_string(),
            ExprKind::Text(text) => text.clone(),
            _ => unreachable!("`is_literal` allows only these"),
        };
        if !covered.literals.insert(key) && !self.missing(ty, covered).is_empty() {
            self.fail("E0317", "this arm can never run: an earlier arm has it".into(), span);
        }
        Some(self.binary_values(BinaryOp::Eq, self.read(holder, span), value, span))
    }

    /// `bindings` is `None` if the name stands alone. Bindings take the fields
    /// in written order, and `_` skips one.
    fn variant_pattern(
        &mut self,
        name: &'a ast::Name,
        bindings: Option<&'a Vec<ast::Name>>,
        holder: Local,
        ty: Ty,
        covered: &mut Covered,
    ) -> (Option<Expr>, Vec<Stmt>) {
        let span = name.span;
        let mut bound = Vec::new();
        let parentheses = bindings.is_some();
        let bindings = bindings.map_or(&[][..], Vec::as_slice);
        let variant = match self.choice_of(ty) {
            Some(id) => self.variant_index(id, name).map(|variant| (id, variant)),
            None => {
                if ty != Ty::Error {
                    covered.failed = true;
                    let message = format!("`{}` is not a `choice`", ty.name());
                    let diag = Diagnostic::new("E0318", message, span);
                    self.report(if matches!(ty, Ty::Union(_)) {
                        let note = self.union_member_note(ty, name);
                        diag.note(note)
                    } else {
                        diag
                    });
                }
                None
            }
        };
        let Some((id, variant)) = variant else {
            for binding in bindings.iter().filter(|binding| binding.text != "_") {
                self.bind(binding, Ty::Error, false);
            }
            return (None, bound);
        };
        if !covered.variants.insert(variant) && !self.missing(ty, covered).is_empty() {
            let message = format!("this arm can never run: an earlier arm has `{}`", name.text);
            self.fail("E0317", message, span);
        }
        let fields = self.datas[id].variants[variant].fields.len();
        if fields == 0 && parentheses {
            let message = format!("`{}` has no fields", name.text);
            let note = format!("write `{}`, with no `()`", name.text);
            self.report(Diagnostic::new("E0318", message, span).note(note));
        } else if bindings.len() != fields {
            let plural = if fields == 1 { "" } else { "s" };
            let message = format!("`{}` has {fields} field{plural}", name.text);
            let note = format!("bind all {fields} in order, and use `_` to skip one");
            self.report(Diagnostic::new("E0318", message, span).note(note));
        }
        for (index, binding) in bindings.iter().enumerate() {
            let field = self.datas[id].variants[variant].fields.get(index);
            let field_ty = field.map_or(Ty::Error, |field| field.ty);
            let field_ty = self.subst_data(field_ty, ty);
            if binding.text == "_" {
                continue;
            }
            self.check_binding(binding);
            let local = self.bind(binding, field_ty, false);
            let value = self.read(holder, span).into();
            let kind = ExprKind::Member { value, variant, field: index };
            bound.push(Stmt::Set(local, Expr { kind, ty: field_ty, span }));
        }
        let int = |kind| Box::new(Expr { kind, ty: Ty::Int, span });
        let tag = int(ExprKind::Tag(self.read(holder, span).into()));
        let kind = ExprKind::Binary(BinaryOp::Eq, tag, int(ExprKind::Int(variant as i64)));
        (Some(Expr { kind, ty: Ty::Bool, span }), bound)
    }

    fn assign(
        &mut self,
        target: &'a ast::Expr,
        op: ast::AssignOp,
        value: &'a ast::Expr,
        span: Span,
    ) -> Option<Stmt> {
        self.check_write(target);
        let name = match &target.kind {
            Ast::Name(name) => name.as_str(),
            Ast::SelfValue => "self",
            Ast::Index { base, indices } => {
                return self.assign_index(base, indices, op, value, span);
            }
            Ast::Field { base, name } => return self.assign_field(base, name, op, value, span),
            _ => unreachable!("the parser allows only a name, field, or index"),
        };
        let Some(local) = self.variable(name, target.span) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let ty = self.frame.locals[local];
        let value = match compound(op) {
            Some(op) => {
                let value = self.binary(op, target, value, None, span);
                self.widen(value, ty)
            }
            None => {
                if let Some((held, result)) = self.statement_value(value, Some(ty)) {
                    let read = self.held_value(result, ty, "this assignment", span);
                    self.forget(local);
                    return Some(self.with_held(vec![held, Stmt::Set(local, read)], result));
                }
                self.expected(value, Some(ty))
            }
        };
        self.expect(&value, ty, "this assignment");
        self.forget(local);
        Some(Stmt::Set(local, value))
    }

    /// The local of a name that can change. Reports E0305 if it cannot, and
    /// returns `None` unless the name is a local at all.
    fn variable(&mut self, name: &str, span: Span) -> Option<Local> {
        if self.lookup(name).is_none() && self.task_ids.contains_key(name) {
            self.fail("E0305", format!("`{name}` cannot change"), span);
            return None;
        }
        let Some(local) = self.lookup(name) else {
            if self.name(name, span).ty != Ty::Error {
                self.fail("E0305", format!("`{name}` cannot change"), span);
            }
            return None;
        };
        if self.frame.locals[local] == Ty::Error {
            return None;
        }
        if !self.frame.mutable[local] {
            let diag = Diagnostic::new("E0305", format!("`{name}` cannot change"), span);
            self.report(diag.note(if local < self.frame.params {
                "a parameter is read-only: declare it `change` to change the caller's value, or `take` to own a copy"
            } else {
                "only a name declared with `var` can change"
            }));
        }
        Some(local)
    }

    /// Reports E0326 if a plain `for` visits what this write changes.
    fn check_write(&mut self, target: &ast::Expr) {
        let Some((root, path)) = target.path() else { return };
        let Some(local) = self.lookup(root) else { return };
        let clash =
            |busy: &Busy| !busy.exclusive && busy.local == local && ast::overlap(&busy.path, &path);
        if self.frame.busy.iter().any(clash) {
            let message = format!("`{root}` cannot change while a `for` visits it");
            let diag = Diagnostic::new("E0326", message, target.span);
            self.report(diag.note(format!("loop over a copy: `let copy = {root}`")));
        }
    }

    fn read(&self, local: Local, span: Span) -> Expr {
        Expr { kind: ExprKind::Local(local), ty: self.frame.locals[local], span }
    }

    /// Finds what a write changes. Each index runs once, before the value
    /// that is written, so that the order is the order on the page.
    fn place(&mut self, target: &'a ast::Expr) -> Option<Place> {
        match &target.kind {
            Ast::Name(_) | Ast::SelfValue => {
                let name = match &target.kind {
                    Ast::Name(name) => name,
                    _ => "self",
                };
                let local = self.variable(name, target.span)?;
                let place = Place::at(local);
                let (ty, span) = (self.current(local), target.span);
                if ty == self.frame.locals[local] || matches!(ty, Ty::Union(_)) {
                    return Some(place);
                }
                // A narrowed union: what it holds is a field of its record.
                let read = view(self.read(local, span), ty);
                let key = Expr { kind: ExprKind::Int(0), ty: Ty::Int, span };
                Some(self.step(place, key, read))
            }
            Ast::Index { base, indices } => {
                let mut place = self.place(base)?;
                let (key, item) = self.key(self.frame.locals[place.local], indices, target.span)?;
                let key = self.hoist(key, &mut place.setup, &mut place.finish);
                let read = self.element(place.local, key, item, target.span);
                let key = self.read(key, target.span);
                Some(self.step(place, key, read))
            }
            Ast::Field { base, name } => {
                let mut place = self.place(base)?;
                let (slot, ty) = self.field(self.frame.locals[place.local], name)?;
                let span = target.span;
                let read = self.slot_of(place.local, slot, ty, span);
                let key = Expr { kind: ExprKind::Int(slot as i64), ty: Ty::Int, span };
                place = self.step(place, key, read);
                Some(place)
            }
            _ => {
                let diag = Diagnostic::new("E0305", "this value cannot change", target.span);
                self.report(diag.note("only a name declared with `var` can change"));
                None
            }
        }
    }

    /// Goes one level deeper: `read` is the element at `key` of `place`.
    fn step(&mut self, mut place: Place, key: Expr, read: Expr) -> Place {
        let (span, item) = (read.span, read.ty);
        let step = match key.kind {
            ExprKind::Int(slot) => Step::Slot(slot),
            ExprKind::Local(local) => Step::Key(local),
            _ => unreachable!("a key is hoisted, and a slot is a number"),
        };
        place.path.push(step);
        let parent = place.local;
        let opened =
            |&&(local, slot, _): &&(Local, i64, Local)| local == parent && Step::Slot(slot) == step;
        if self.frame.opening
            && let Some(&(_, _, child)) = self.frame.opened.iter().find(opened)
        {
            place.local = child;
            return place;
        }
        let value = self.new_local(item, true);
        if let (true, Step::Slot(slot)) = (self.frame.opening, step) {
            self.frame.opened.push((parent, slot, value));
        }
        let stored = self.read(value, span);
        if matches!(self.frame.locals[place.local], Ty::List(_) | Ty::Data(..)) {
            // The slot's own count moves to the local and back, so no count
            // changes. Until `take`, the local borrows from the slot.
            place.setup.push(Stmt::Bind(value, read));
            let local = place.local;
            place.take.push(Stmt::Update {
                local,
                op: Update::Take,
                args: vec![key.clone()],
                span,
            });
            let (op, args) = (Update::Put, vec![key, stored]);
            place.finish.insert(0, Stmt::Update { local, op, args, span });
        } else {
            place.setup.push(Stmt::Set(value, read));
            let empty = Expr { kind: ExprKind::None, ty: Ty::None, span };
            place.take.push(self.write(place.local, key.clone(), empty, span));
            let write = self.write(place.local, key, stored, span);
            place.finish.insert(0, write);
            place.finish.push(Stmt::Clear(value));
        }
        place.local = value;
        place
    }

    /// Moves a value to a hidden local, so that it can be read twice or read
    /// before `take`. The local lets go of it when the write ends.
    fn hoist(&mut self, value: Expr, setup: &mut Vec<Stmt>, finish: &mut Vec<Stmt>) -> Local {
        let local = self.new_local(value.ty, false);
        if value.ty.is_counted() {
            finish.push(Stmt::Clear(local));
        }
        setup.push(Stmt::Set(local, value));
        local
    }

    /// Reads an element from the container in `local`.
    fn element(&self, local: Local, key: Local, ty: Ty, span: Span) -> Expr {
        let (container, key) = (self.read(local, span).into(), self.read(key, span).into());
        let kind = match self.frame.locals[local] {
            Ty::List(_) => ExprKind::ListGet(container, key),
            _ => ExprKind::MapGet(container, key),
        };
        Expr { kind, ty, span }
    }

    fn write(&self, local: Local, key: Expr, value: Expr, span: Span) -> Stmt {
        let op = match self.frame.locals[local] {
            Ty::List(_) => Update::ListSet,
            Ty::Data(..) => Update::FieldSet,
            Ty::Union(_) => Update::UnionSet,
            _ => Update::MapSet,
        };
        Stmt::Update { local, op, args: vec![key, value], span }
    }

    fn assign_index(
        &mut self,
        base: &'a ast::Expr,
        indices: &'a [ast::Expr],
        op: ast::AssignOp,
        value: &'a ast::Expr,
        span: Span,
    ) -> Option<Stmt> {
        let Some(mut place) = self.place(base) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let local = place.local;
        let Some((key, item)) = self.key(self.frame.locals[local], indices, span) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let key = self.hoist(key, &mut place.setup, &mut place.finish);
        let value = match compound(op) {
            Some(op) => {
                let old = self.element(local, key, item, span);
                let value = self.lower(value, self.number_want(Some(old.ty)));
                self.binary_values(op, old, value, span)
            }
            None => self.expected(value, Some(item)),
        };
        self.expect(&value, item, "this assignment");
        let value = self.ready(value, &mut place);
        let write = self.write(local, self.read(key, span), value, span);
        Some(place.stmts(write))
    }

    fn assign_field(
        &mut self,
        base: &'a ast::Expr,
        name: &'a ast::Name,
        op: ast::AssignOp,
        value: &'a ast::Expr,
        span: Span,
    ) -> Option<Stmt> {
        let Some(mut place) = self.place(base) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let local = place.local;
        let Some((slot, ty)) = self.field(self.frame.locals[local], name) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let value = match compound(op) {
            Some(op) => {
                let old = self.slot_of(local, slot, ty, span);
                let value = self.lower(value, self.number_want(Some(old.ty)));
                self.binary_values(op, old, value, span)
            }
            None => self.expected(value, Some(ty)),
        };
        self.expect(&value, ty, "this assignment");
        let value = self.ready(value, &mut place);
        let key = Expr { kind: ExprKind::Int(slot as i64), ty: Ty::Int, span };
        let write = self.write(local, key, value, span);
        Some(place.stmts(write))
    }

    fn slot_of(&self, local: Local, field: usize, ty: Ty, span: Span) -> Expr {
        let value = self.read(local, span).into();
        Expr { kind: ExprKind::Member { value, variant: 0, field }, ty, span }
    }

    /// A field type of `data`, with the type arguments of `data` in place.
    fn subst_data(&mut self, field: Ty, data: Ty) -> Ty {
        let Ty::Data(_, _, args) = data else { return field };
        self.table.subst(field, args)
    }

    /// Finds a field of a `type`. Returns its position and type.
    fn field(&mut self, ty: Ty, name: &ast::Name) -> Option<(usize, Ty)> {
        let id = match ty {
            Ty::Error => return None,
            Ty::Data(_, id, _) if !self.datas[id].choice => id,
            Ty::Data(..) => {
                let message = format!("`{}` is a `choice`, so it has no fields", ty.name());
                let diag = Diagnostic::new("E0303", message, name.span);
                self.report(diag.note("use `match` to reach the fields of a variant"));
                return None;
            }
            Ty::Union(_) => {
                let message = format!("`{}` has no field `{}`", ty.name(), name.text);
                let diag = Diagnostic::new("E0303", message, name.span);
                self.report(diag.note(TEST_UNION));
                return None;
            }
            _ => usize::MAX,
        };
        let fields = self.datas.get(id).map_or(&[][..], |data| &data.variants[0].fields);
        let found = fields.iter().position(|f| f.name == name.text);
        let found = found.map(|index| (index, fields[index].ty));
        if found.is_none() {
            let message = format!("`{}` has no field `{}`", ty.name(), name.text);
            self.fail("E0303", message, name.span);
        }
        found.map(|(index, field)| (index, self.subst_data(field, ty)))
    }

    /// Lowers a value for a parameter or field of type `pattern`, whose type
    /// parameters may come from this value. Types flow left to right, as in a
    /// list literal, so an earlier value gives a type to a later one. See
    /// decision 67. `settled` checks the result, once all types are known.
    fn inferred(&mut self, value: &'a ast::Expr, pattern: Ty, known: &mut [Option<Ty>]) -> Expr {
        let want = self.table.partial(pattern, known);
        // A lambda can give a `U` that only its result shows, once `T` is known.
        if let (None, Ast::Lambda { params, body }, Ty::Task(types, returns)) =
            (want, &value.kind, pattern)
        {
            let given: Option<Vec<Ty>> =
                types.iter().map(|&ty| self.table.partial(ty, known)).collect();
            if let Some(given) = given.filter(|given| given.len() == params.len()) {
                let result = self.table.partial(*returns, known);
                let lowered = self.lambda_with(params, body, &given, result, value.span);
                self.table.unify(pattern, lowered.ty, known);
                return lowered;
            }
        }
        let lowered = self.expected(value, want);
        if want.is_none() {
            self.table.unify(pattern, lowered.ty, known);
        }
        lowered
    }

    /// Gives a value its final type, and checks it. A `none` for a `T?` could
    /// not widen before a later value told what `T` is.
    fn settled(&mut self, value: Expr, ty: Ty, what: &str) -> Expr {
        let value = self.widen(value, ty);
        self.expect(&value, ty, what);
        value
    }

    /// Finishes type arguments. Reports E0319 for one with no answer, unless
    /// an error was already reported since `reported`.
    fn solved(
        &mut self,
        name: &str,
        params: &[&str],
        known: &[Option<Ty>],
        reported: usize,
        span: Span,
    ) -> Option<&'static [Ty]> {
        if let Some(index) = known.iter().position(Option::is_none) {
            if self.diags.len() == reported {
                let message = format!("cannot tell what `{}` is here", params[index]);
                let note = format!("name the types, as in `{name}[int](...)`");
                self.report(Diagnostic::new("E0319", message, span).note(note));
            }
            return None;
        }
        Some(known.iter().flatten().copied().collect::<Vec<_>>().leak())
    }

    /// Reads written type arguments, for something that takes `want` of them.
    fn explicit(
        &mut self,
        indices: &'a [ast::Expr],
        want: usize,
        name: &str,
        span: Span,
    ) -> Option<Vec<Ty>> {
        let args: Vec<Ty> = indices.iter().map(|index| self.expr_type(index)).collect();
        (self.arity(name, want, args.len(), span) && !args.contains(&Ty::Error)).then_some(args)
    }

    /// Builds a `type` or a variant. Arguments are always named, and run in
    /// the order written, so any order keeps the order on the page.
    ///
    /// Type arguments come from `explicit`, then `want`, then the values.
    fn construct(
        &mut self,
        (id, variant): (usize, usize),
        explicit: Option<&[Ty]>,
        want: Option<Ty>,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Expr {
        let reported = self.diags.len();
        let name = self.datas[id].variants[variant].name;
        let count = self.datas[id].variants[variant].fields.len();
        let wanted = want.iter().flat_map(|ty| ty.members()).find_map(|ty| match ty {
            Ty::Data(_, found, args) if *found == id => Some(*args),
            _ => None,
        });
        let mut known: Vec<Option<Ty>> = match explicit.or(wanted) {
            Some(types) => types.iter().copied().map(Some).collect(),
            None => vec![None; self.datas[id].type_params.len()],
        };
        let mut given = vec![false; count];
        let mut fields = Vec::with_capacity(count);
        for arg in args {
            let info = &self.datas[id].variants[variant];
            let found = arg
                .name
                .as_ref()
                .and_then(|label| info.fields.iter().position(|field| field.name == label.text));
            let pattern = found.map_or(Ty::Error, |index| info.fields[index].ty);
            let value = self.inferred(&arg.value, pattern, &mut known);
            let span = arg.value.span;
            let Some(label) = &arg.name else {
                let diag =
                    Diagnostic::new("E0307", format!("`{name}` needs named arguments"), span);
                self.report(diag.note("write `field: value`"));
                continue;
            };
            let Some(index) = found else {
                self.fail("E0307", format!("`{name}` has no field `{}`", label.text), label.span);
                continue;
            };
            if given[index] {
                let message = format!("`{}` is given twice", label.text);
                self.fail("E0307", message, label.span);
                continue;
            }
            given[index] = true;
            fields.push((index, value));
        }
        for index in (0..count).filter(|&index| !given[index]) {
            let field = &self.datas[id].variants[variant].fields[index];
            if field.default.is_none() {
                let message = format!("`{name}` is missing field `{}`", field.name);
                self.fail("E0307", message, span);
            }
        }
        let params: Vec<&str> = self.datas[id].type_params.iter().map(|&(name, _)| name).collect();
        let Some(args) = self.solved(name, &params, &known, reported, span) else {
            return poisoned(span);
        };
        self.check_data_bounds(id, args, span);
        for (index, value) in &mut fields {
            let ty = self.datas[id].variants[variant].fields[*index].ty;
            let ty = self.table.subst(ty, args);
            let lowered = std::mem::replace(value, poisoned(span));
            *value = self.settled(lowered, ty, "this field");
        }
        for index in (0..count).filter(|&index| !given[index]) {
            let field = &self.datas[id].variants[variant].fields[index];
            if let Some(value) = field.default.clone() {
                let ty = field.ty;
                let ty = self.table.subst(ty, args);
                fields.push((index, Expr { ty, ..value }));
            }
        }
        let Ty::Data(type_name, ..) = self.datas[id].ty else {
            unreachable!("a data type has its own name")
        };
        let ty = Ty::Data(type_name, id, args);
        Expr { kind: ExprKind::New { tag: variant, counted: 0, fields }, ty, span }
    }

    /// The `type` or variant that this name builds here, if any.
    fn constructor(&self, name: &str) -> Option<(usize, usize)> {
        let free = !self.visible(name) && !self.top.contains_key(name);
        self.constructors.get(name).copied().filter(|_| free)
    }

    fn task_named(&self, name: &str) -> Option<usize> {
        let free = !self.visible(name) && !self.top.contains_key(name);
        self.task_ids.get(name).copied().filter(|_| free)
    }

    /// `Name(...)` with fields, or `Name[T](...)`.
    fn construct_call(
        &mut self,
        target: (usize, usize),
        name: &str,
        explicit: Option<&[Ty]>,
        want: Option<Ty>,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Expr {
        let (id, variant) = target;
        if self.datas[id].variants[variant].fields.is_empty() {
            let diag = Diagnostic::new("E0307", format!("`{name}` has no fields"), span);
            return self.report(diag.note(format!("write `{name}`, with no `()`")));
        }
        self.construct(target, explicit, want, args, span)
    }

    /// Reads a value now if the write takes a slot that it may read.
    fn ready(&mut self, value: Expr, place: &mut Place) -> Expr {
        if place.take.is_empty() {
            return value;
        }
        let span = value.span;
        let value = self.hoist(value, &mut place.setup, &mut place.finish);
        self.read(value, span)
    }

    /// True if the language builds this task in for the type of `value`.
    /// Errors stay for the real lowering to report.
    fn is_builtin(&mut self, value: &'a ast::Expr, name: &str) -> bool {
        let reported = self.diags.len();
        let found = builtin_method(self.expr(value).ty, name);
        self.diags.truncate(reported);
        found
    }

    fn changing_call(
        &mut self,
        base: &'a ast::Expr,
        name: &'a ast::Name,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Option<Stmt> {
        if name.text == "pop" {
            let (stmt, result) = self.pop(base, args, span);
            return Some(self.with_held(vec![stmt], result));
        }
        self.check_write(base);
        let mut place = self.place(base)?;
        let local = place.local;
        let ty = self.frame.locals[local];
        let (op, wants): (Update, &[Ty]) = match (ty, name.text.as_str()) {
            (Ty::List(item), "push") => (Update::ListPush, std::slice::from_ref(item)),
            (Ty::List(_), "swap") => (Update::ListSwap, &[Ty::Int, Ty::Int]),
            (Ty::Set(item), "add") => (Update::MapSet, std::slice::from_ref(item)),
            (Ty::Set(key) | Ty::Map(key, _), "remove") => {
                (Update::MapRemove, std::slice::from_ref(key))
            }
            (ty, _) => {
                let message = format!("`{}` does not work on `{}`", name.text, ty.name());
                self.fail("E0303", message, span);
                return None;
            }
        };
        if args.len() != wants.len() || args.iter().any(|arg| arg.change || arg.name.is_some()) {
            let (count, plural) = (wants.len(), if wants.len() == 1 { "" } else { "s" });
            let message = format!("`{}` takes {count} positional argument{plural}", name.text);
            self.fail("E0307", message, span);
            return None;
        }
        let mut values = Vec::with_capacity(2);
        for (arg, &want) in args.iter().zip(wants) {
            let value = self.expected(&arg.value, Some(want));
            self.expect(&value, want, "this argument");
            values.push(self.ready(value, &mut place));
        }
        if name.text == "add" {
            values.push(Expr { kind: ExprKind::None, ty: Ty::None, span });
        }
        Some(place.stmts(Stmt::Update { local, op, args: values, span }))
    }

    /// Reads the last value, and drops it, or gives `none` for an empty list.
    /// Gives a statement and the local that holds the value.
    fn pop(&mut self, base: &'a ast::Expr, args: &'a [ast::Arg], span: Span) -> (Stmt, Local) {
        let nothing = Stmt::Block(Vec::new());
        self.check_write(base);
        let Some(place) = self.place(base) else {
            return (nothing, self.new_local(Ty::Error, false));
        };
        let local = place.local;
        let Ty::List(item) = self.frame.locals[local] else {
            let ty = self.frame.locals[local].name();
            self.fail("E0303", format!("`pop` does not work on `{ty}`"), span);
            return (nothing, self.new_local(Ty::Error, false));
        };
        if !args.is_empty() {
            self.fail("E0307", "`pop` takes no arguments".into(), span);
        }
        let ty = self.union([*item, Ty::None]);
        let result = self.new_local(ty, false);
        let int = |value| Expr { kind: ExprKind::Int(value), ty: Ty::Int, span };
        let list = Box::new(self.read(local, span));
        let len = Expr { kind: ExprKind::Len(list.clone()), ty: Ty::Int, span };
        let any = self.binary_values(BinaryOp::Gt, len.clone(), int(0), span);
        let last = self.binary_values(BinaryOp::Sub, len, int(1), span);
        let value = Expr { kind: ExprKind::ListGet(list, last.into()), ty: *item, span };
        let found = vec![
            Stmt::Set(result, self.widen(value, ty)),
            Stmt::Update { local, op: Update::ListPop, args: Vec::new(), span },
        ];
        let none = Expr { kind: ExprKind::None, ty: Ty::None, span };
        let otherwise = vec![Stmt::Set(result, self.widen(none, ty))];
        (place.stmts(Stmt::If { branches: vec![(any, found)], otherwise }), result)
    }

    fn range(&mut self, start: &'a ast::Expr, end: &'a ast::Expr) -> (Expr, Expr) {
        let start = self.expr(start);
        let end = self.expr(end);
        self.expect(&start, Ty::Int, "a range");
        self.expect(&end, Ty::Int, "a range");
        (start, end)
    }

    /// Checks a loop body, in a new scope that holds the loop name.
    fn loop_body(
        &mut self,
        name: &'a ast::Name,
        ty: Ty,
        mutable: bool,
        body: &'a [ast::Stmt],
    ) -> (Local, Vec<Stmt>) {
        self.forget_assigned(body);
        self.frame.scopes.push(HashMap::new());
        let local = self.bind(name, ty, mutable);
        let held = self.frame.held.len();
        self.frame.loop_scopes.push((self.frame.scopes.len() - 1, Some(local), held));
        let mut body = self.stmts(body);
        self.frame.loop_scopes.pop();
        self.close_scope(&mut body, Some(local));
        (local, body)
    }

    fn for_loop(
        &mut self,
        change: bool,
        name: &'a ast::Name,
        iterable: &'a ast::Expr,
        body: &'a [ast::Stmt],
    ) -> Stmt {
        let Ast::Binary(op @ (BinaryOp::Range | BinaryOp::RangeInclusive), start, end) =
            &iterable.kind
        else {
            let reported = self.diags.len();
            let value = self.expr(iterable);
            let span = value.span;
            let value = match (change, value.ty) {
                (_, Ty::Error) => value,
                (true, Ty::Text) => self.fail("E0303", "text cannot change".into(), span),
                (true, Ty::List(_)) => {
                    // It finds its place from the source, with the same errors.
                    self.diags.truncate(reported);
                    return self.for_change(iterable, name, body);
                }
                (true, Ty::Map(..) | Ty::Set(_)) => {
                    self.fail("E0303", "a key cannot change".into(), span)
                }
                (false, Ty::Text | Ty::List(_) | Ty::Map(..) | Ty::Set(_)) => value,
                (_, ty) => {
                    self.fail("E0303", format!("`for` cannot loop over `{}`", ty.name()), span)
                }
            };
            // Text cannot change, so a `for` over it has nothing to protect.
            let busy = iterable.path().filter(|_| value.ty != Ty::Text);
            let busy = busy.and_then(|(root, path)| Some((self.lookup(root)?, path)));
            let (Some((local, path)), true) = (busy, !change) else {
                return self.for_each(value, name, body);
            };
            self.frame.busy.push(Busy { local, path, exclusive: false });
            let stmt = self.for_each(value, name, body);
            self.frame.busy.pop();
            return stmt;
        };
        let (start, end) = self.range(start, end);
        if change {
            self.fail("E0303", "a range has nothing to change".into(), iterable.span);
        }
        let (local, body) = self.loop_body(name, Ty::Int, false, body);
        Stmt::For { local, start, end, inclusive: *op == BinaryOp::RangeInclusive, body }
    }

    /// `for change x in items`. The list stays out of its place for the whole
    /// loop, and each element is out of the list while the body runs, so both
    /// hold one count and every write is in place. So `x` is a plain local, and
    /// any way out of the body puts it back first, and any `return` puts the
    /// list back too.
    fn for_change(
        &mut self,
        iterable: &'a ast::Expr,
        name: &'a ast::Name,
        body: &'a [ast::Stmt],
    ) -> Stmt {
        let span = iterable.span;
        self.check_write(iterable);
        let Some(mut place) = self.place(iterable) else {
            return Stmt::Block(Vec::new());
        };
        let list = place.local;
        let Ty::List(&item) = self.frame.locals[list] else {
            self.fail("E0303", "`for change` needs a list".into(), span);
            return Stmt::Block(Vec::new());
        };
        // Every step of the path is out of its place while the body runs, so
        // only a field beside the first step can be read. A narrowed name has
        // given up its whole value.
        let root = place.root;
        let mut path = iterable.path().map_or(Vec::new(), |(_, path)| path);
        let narrowed = self.current(root) != self.frame.locals[root];
        path.truncate(usize::from(!narrowed));
        self.frame.busy.push(Busy { local: root, path, exclusive: true });
        let (current, mut inner) = self.loop_body(name, item, true, body);
        self.frame.busy.pop();

        let position = self.new_local(Ty::Int, true);
        let at = self.new_local(Ty::Int, true);
        let expr = |kind, ty| Expr { kind, ty, span };
        let int = |value| expr(ExprKind::Int(value), Ty::Int);
        let local = |id, ty| expr(ExprKind::Local(id), ty);
        let list_ty = self.frame.locals[list];
        let len = expr(ExprKind::Len(local(list, list_ty).into()), Ty::Int);
        let inside = self.binary_values(BinaryOp::Lt, local(position, Ty::Int), len, span);
        let next = self.binary_values(BinaryOp::Add, local(position, Ty::Int), int(1), span);
        let element = ExprKind::ListGet(local(list, list_ty).into(), local(at, Ty::Int).into());
        let mut start = vec![Stmt::Set(at, local(position, Ty::Int)), Stmt::Set(position, next)];
        let back = if item.is_counted() {
            // As in `step`: the slot's own count moves to the loop name and back.
            let key = local(at, Ty::Int);
            start.push(Stmt::Bind(current, expr(element, item)));
            let (op, args) = (Update::Take, vec![key.clone()]);
            start.push(Stmt::Update { local: list, op, args, span });
            let (op, args) = (Update::Put, vec![key, local(current, item)]);
            vec![Stmt::Update { local: list, op, args, span }]
        } else {
            start.push(Stmt::Set(current, expr(element, item)));
            vec![self.write(list, local(at, Ty::Int), local(current, item), span)]
        };
        let mut leave = back.clone();
        leave.extend(place.finish.iter().cloned());
        let (locals, mutable) = (&mut self.frame.locals, &mut self.frame.mutable);
        PutBack { locals, mutable, back: &back, leave: &leave, own_loop: true }.stmts(&mut inner);
        start.append(&mut inner);
        start.extend(back);

        let mut stmts = std::mem::take(&mut place.setup);
        stmts.append(&mut place.take);
        stmts.push(Stmt::Set(position, int(0)));
        stmts.push(Stmt::While { condition: inside, body: start });
        stmts.append(&mut place.finish);
        Stmt::Block(stmts)
    }

    /// Lowers to a `while` over a hidden position. A hidden local keeps the
    /// value alive and fixed, since the body may assign the variable it came
    /// from. The position moves before the body, so `continue` needs no
    /// separate step. A text position is a byte offset, a map's is an entry.
    fn for_each(&mut self, value: Expr, name: &'a ast::Name, body: &'a [ast::Stmt]) -> Stmt {
        let (span, ty) = (value.span, value.ty);
        let item = match ty {
            Ty::List(item) | Ty::Set(item) | Ty::Map(item, _) => *item,
            _ => ty,
        };
        let owner = self.new_local(ty, false);
        let position = self.new_local(Ty::Int, true);
        let (current, body) = self.loop_body(name, item, false, body);

        let expr = |kind, ty| Expr { kind, ty, span };
        let local = |id, ty| Box::new(expr(ExprKind::Local(id), ty));
        let (held, at) = (local(owner, ty), local(position, Ty::Int));
        let zero = || Box::new(expr(ExprKind::Int(0), Ty::Int));
        let one = Box::new(expr(ExprKind::Int(1), Ty::Int));
        let next = |at| ExprKind::MapNext(held.clone(), at);
        let after = ExprKind::Binary(BinaryOp::Add, at.clone(), one);
        let (first, inside, read, advance) = match ty {
            Ty::Text => {
                let byte_len = |id| Box::new(expr(ExprKind::TextByteLen(local(id, ty)), Ty::Int));
                (
                    ExprKind::Int(0),
                    ExprKind::Binary(BinaryOp::Lt, at.clone(), byte_len(owner)),
                    ExprKind::TextCharAt(held.clone(), at.clone()),
                    ExprKind::Binary(BinaryOp::Add, at, byte_len(current)),
                )
            }
            Ty::List(_) => {
                let len = expr(ExprKind::Len(held.clone()), Ty::Int);
                (
                    ExprKind::Int(0),
                    ExprKind::Binary(BinaryOp::Lt, at.clone(), len.into()),
                    ExprKind::ListGet(held.clone(), at),
                    after,
                )
            }
            _ => (
                next(zero()),
                ExprKind::Binary(BinaryOp::Ge, at.clone(), zero()),
                ExprKind::MapKey(held.clone(), at),
                next(expr(after, Ty::Int).into()),
            ),
        };
        let mut inner =
            vec![Stmt::Set(current, expr(read, item)), Stmt::Set(position, expr(advance, Ty::Int))];
        inner.extend(body);
        Stmt::Block(vec![
            Stmt::Set(owner, value),
            Stmt::Set(position, expr(first, Ty::Int)),
            Stmt::While { condition: expr(inside, Ty::Bool), body: inner },
            Stmt::Clear(current),
            Stmt::Clear(owner),
        ])
    }

    fn name(&mut self, name: &str, span: Span) -> Expr {
        if let Some(local) = self.lookup(name) {
            self.mark_read(local);
            return view(self.read(local, span), self.current(local));
        }
        if let Some(value) = self.captured(name, span) {
            return value;
        }
        if self.frame.returns.is_some() {
            match self.top.get(name) {
                Some(&Top::Literal(value, ty)) => {
                    // Its own statement reports any error.
                    let reported = self.diags.len();
                    let want = ty.map(|ty| self.ty(ty));
                    let value = self.expected(value, want);
                    self.diags.truncate(reported);
                    return Expr { span, ..value };
                }
                Some(Top::Var) => {
                    let message = format!("a task cannot use top-level `var` `{name}`");
                    let diag = Diagnostic::new("E0306", message, span);
                    return self.report(diag.note("pass it as a parameter"));
                }
                Some(Top::Let) => {
                    return self.unsupported("a task reading a non-literal top-level `let`", span);
                }
                None => {}
            }
        }
        if let Some(&id) = self.task_ids.get(name) {
            return self.task_value(id, name, None, span);
        }
        if let Some(&(id, variant)) = self.constructors.get(name) {
            if self.datas[id].variants[variant].fields.is_empty() {
                return self.construct((id, variant), None, None, &[], span);
            }
            let diag = Diagnostic::new("E0307", format!("`{name}` needs its fields"), span);
            return self.report(diag.note(format!("write `{name}(field: value)`")));
        }
        let diag = Diagnostic::new("E0301", format!("name `{name}` is not defined"), span);
        let dotted = self.methods.contains_key(name);
        let note = format!("`{name}` has `self`, so it has no value form: write a lambda");
        let diag = if dotted { diag.note(note) } else { self.similar(diag, name, span) };
        self.report(diag)
    }

    /// A literal has `want` if that is an integer type, or else `int`.
    fn int(&mut self, lexeme: &str, want: Option<Ty>, span: Span) -> Expr {
        let ty = want.filter(|ty| ty.is_integer()).unwrap_or(Ty::Int);
        let negative = lexeme.starts_with('-');
        let digits = lexeme.trim_start_matches('-').replace('_', "");
        let parsed = match digits.get(..2) {
            Some("0x") => u64::from_str_radix(&digits[2..], 16),
            Some("0b") => u64::from_str_radix(&digits[2..], 2),
            _ => digits.parse(),
        };
        let value = parsed.map(|value: u64| {
            let value = i128::from(value);
            if negative { -value } else { value }
        });
        let (low, high) = ty.int_range();
        match value {
            Ok(value) if (low..=high).contains(&value) => {
                Expr { kind: ExprKind::Int(value as i64), ty, span }
            }
            _ => {
                let message = format!("`{lexeme}` does not fit in `{}`", ty.name());
                self.fail("E0312", message, span)
            }
        }
    }

    /// A literal is a `float` unless `want` is `f32`.
    fn float(&mut self, lexeme: &str, want: Option<Ty>, span: Span) -> Expr {
        let ty = if want == Some(Ty::F32) { Ty::F32 } else { Ty::Float };
        let digits = lexeme.replace('_', "");
        let mantissa = digits.split(['e', 'E']).next().unwrap_or_default();
        let nonzero = mantissa.contains(|c: char| matches!(c, '1'..='9'));
        // A direct parse rounds once, so the `f32` is the one nearest the text.
        let parsed = match ty {
            Ty::F32 => digits.parse::<f32>().map(f64::from),
            _ => digits.parse::<f64>(),
        };
        match parsed {
            Ok(value) if value.is_finite() && (value != 0.0 || !nonzero) => {
                Expr { kind: ExprKind::Float(value), ty, span }
            }
            _ => {
                let message = format!("`{lexeme}` does not fit in `{}`", ty.name());
                self.fail("E0312", message, span)
            }
        }
    }

    /// Lowers `expr`. `want` is the type that the context needs, if known,
    /// which is how an empty list or map gets its type, and how a value
    /// becomes a union.
    fn expected(&mut self, expr: &'a ast::Expr, want: Option<Ty>) -> Expr {
        let value = self.lower(expr, want);
        match want {
            Some(want) => self.widen(value, want),
            None => value,
        }
    }

    /// The number type that `want` asks of a literal, with a `T?` counting as `T`.
    fn number_want(&self, want: Option<Ty>) -> Option<Ty> {
        self.literal_want(want, |ty| ty.is_number()).filter(|ty| ty.is_number())
    }

    fn lower(&mut self, expr: &'a ast::Expr, want: Option<Ty>) -> Expr {
        let span = expr.span;
        let number = self.number_want(want);
        match &expr.kind {
            Ast::Int(lexeme) => self.int(lexeme, number, span),
            Ast::Float(lexeme) => self.float(lexeme, number, span),
            Ast::Unary(op @ (UnaryOp::Neg | UnaryOp::BitNot), value) => {
                self.unary(*op, value, number, span)
            }
            Ast::Binary(BinaryOp::Fallback, left, right) => self.fallback(left, right, want, span),
            Ast::Binary(op, left, right) if is_math(*op) => {
                self.binary(*op, left, right, number, span)
            }
            Ast::List(items) => self.list(items, want, span),
            Ast::Map(entries) => self.map(entries, want, span),
            Ast::If { condition, then, otherwise } => {
                let test = self.condition(condition, "`if`");
                let start = self.frame.narrowed.len();
                let (found, rest) = (self.facts(condition, true), self.facts(condition, false));
                self.frame.narrowed.extend(found);
                let then = self.expected(then, want);
                self.frame.narrowed.truncate(start);
                self.frame.narrowed.extend(rest);
                let otherwise = self.expected(otherwise, want.or(Some(then.ty)));
                self.frame.narrowed.truncate(start);
                self.expect(&otherwise, then.ty, "`else`, to match `then`,");
                let ty = then.ty;
                let kind = ExprKind::If(test.into(), then.into(), otherwise.into());
                Expr { kind, ty, span }
            }
            Ast::Call { callee, args } => self.call(callee, args, want, span),
            Ast::Lambda { params, body } => self.lambda(params, body, want, span),
            Ast::Name(name) => match self.constructor(name) {
                Some((id, variant)) if self.datas[id].variants[variant].fields.is_empty() => {
                    self.construct((id, variant), None, want, &[], span)
                }
                _ => self.expr(expr),
            },
            _ => self.expr(expr),
        }
    }

    /// True if `set` here means the built-in, and no name of the program hides it.
    fn is_set_call(&self, name: &str) -> bool {
        self.is_built_in_call(name, "set")
    }

    /// True if `name` is the built-in `built_in`, which no name of the
    /// program hides.
    fn is_built_in_call(&self, name: &str, built_in: &str) -> bool {
        name == built_in
            && !self.visible(name)
            && !self.top.contains_key(name)
            && !self.task_ids.contains_key(name)
    }

    fn set_call(&mut self, args: &'a [ast::Arg], want: Option<Ty>, span: Span) -> Expr {
        let want = self.literal_want(want, |ty| matches!(ty, Ty::Set(_)));
        let [ast::Arg { name: None, change: false, value }] = args else {
            return self.fail("E0307", "`set` takes one list".into(), span);
        };
        let item = match want {
            Some(Ty::Set(item)) => Some(item),
            _ => None,
        };
        let inner_want = if want == Some(Ty::Error) { want } else { item.map(Ty::List) };
        let list = self.expected(value, inner_want);
        match list.ty {
            Ty::Error => poisoned(span),
            Ty::List(listed) => {
                // The written type already reported a bad key.
                if item.is_none() {
                    self.check_key(*listed, list.span);
                }
                Expr { kind: ExprKind::SetOf(list.into(), None), ty: Ty::Set(listed), span }
            }
            ty => {
                let message = format!("`set` needs a list, found `{}`", ty.name());
                self.fail("E0303", message, list.span)
            }
        }
    }

    fn untyped_empty(&mut self, what: &str, want: Option<Ty>, example: &str, span: Span) -> Expr {
        match want {
            Some(Ty::Error) => return poisoned(span),
            Some(Ty::Set(_)) => {
                let message = format!("an empty {what} does not fit `{}`", want_name(want));
                let diag = Diagnostic::new("E0303", message, span);
                return self.report(diag.note("an empty set is `set([])`"));
            }
            Some(_) => {
                let message = format!("an empty {what} does not fit `{}`", want_name(want));
                return self.fail("E0303", message, span);
            }
            None => {}
        }
        let message = format!("an empty {what} needs a type");
        let diag = Diagnostic::new("E0314", message, span);
        self.report(diag.note(format!("write its type, as in `{example}`")))
    }

    fn list(&mut self, items: &'a [ast::Expr], want: Option<Ty>, span: Span) -> Expr {
        let want = self.literal_want(want, |ty| matches!(ty, Ty::List(_)));
        let mut item = match want {
            Some(Ty::List(item)) => Some(*item),
            _ => None,
        };
        let mut values = Vec::with_capacity(items.len());
        for value in items {
            let value = self.expected(value, item);
            let ty = *item.get_or_insert(value.ty);
            self.expect(&value, ty, "this list");
            values.push(value);
        }
        match item {
            Some(Ty::Error) => poisoned(span),
            Some(item) => Expr { kind: ExprKind::ListOf(values), ty: Ty::List(inner(item)), span },
            None => self.untyped_empty("list", want, "let x: list[int] = []", span),
        }
    }

    fn map(&mut self, entries: &'a [(ast::Expr, ast::Expr)], want: Option<Ty>, span: Span) -> Expr {
        let want = self.literal_want(want, |ty| matches!(ty, Ty::Map(..)));
        let (mut key_ty, mut value_ty) = match want {
            Some(Ty::Map(key, value)) => (Some(*key), Some(*value)),
            _ => (None, None),
        };
        let inferred = key_ty.is_none();
        let mut lowered = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let key = self.expected(key, key_ty);
            let value = self.expected(value, value_ty);
            let want_key = *key_ty.get_or_insert(key.ty);
            let want_value = *value_ty.get_or_insert(value.ty);
            self.expect(&key, want_key, "this map");
            self.expect(&value, want_value, "this map");
            lowered.push((key, value));
        }
        match (key_ty, value_ty) {
            (Some(Ty::Error), _) | (_, Some(Ty::Error)) => poisoned(span),
            (Some(key), Some(value)) => {
                if inferred {
                    self.check_key(key, span);
                }
                let ty = Ty::Map(inner(key), inner(value));
                Expr { kind: ExprKind::MapOf(lowered, None), ty, span }
            }
            _ => self.untyped_empty("map", want, "let x: map[text, int] = {}", span),
        }
    }

    fn expr(&mut self, expr: &'a ast::Expr) -> Expr {
        let span = expr.span;
        let (kind, ty) = match &expr.kind {
            Ast::Int(lexeme) => return self.int(lexeme, None, span),
            Ast::Float(lexeme) => return self.float(lexeme, None, span),
            Ast::Bool(value) => (ExprKind::Bool(*value), Ty::Bool),
            Ast::None => (ExprKind::None, Ty::None),
            Ast::Name(name) => return self.name(name, span),
            Ast::Unary(op, value) => return self.unary(*op, value, None, span),
            Ast::Binary(BinaryOp::Fallback, left, right) => {
                return self.fallback(left, right, None, span);
            }
            Ast::Binary(op, left, right) => return self.binary(*op, left, right, None, span),
            Ast::If { .. } | Ast::List(_) | Ast::Map(_) => return self.expected(expr, None),
            Ast::Call { callee, args } => return self.call(callee, args, None, span),
            Ast::Text(parts) => return self.text(parts, span),
            Ast::Is { value, negated, target } => {
                return self.is_test(value, *negated, target, span);
            }
            Ast::Lambda { params, body } => return self.lambda(params, body, None, span),
            Ast::Index { base, indices } => return self.index(base, indices, span),
            Ast::Field { base, name } => {
                let value = self.expr(base);
                let Some((field, ty)) = self.field(value.ty, name) else {
                    return poisoned(span);
                };
                (ExprKind::Member { value: value.into(), variant: 0, field }, ty)
            }
            Ast::Match { .. } => unreachable!("only `let`, `return`, and statements hold one"),
            Ast::Optional(_) | Ast::Type(_) => {
                let diag = Diagnostic::new("E0322", "a type cannot be a value", span);
                return self.report(diag.note("a type only goes in brackets, as in `f[int?](x)`"));
            }
            Ast::SelfValue => return self.name("self", span),
        };
        Expr { kind, ty, span }
    }

    fn index(&mut self, base: &'a ast::Expr, indices: &'a [ast::Expr], span: Span) -> Expr {
        if let Ast::Name(name) = &base.kind
            && let Some((id, variant)) = self.constructor(name)
            && self.datas[id].variants[variant].fields.is_empty()
        {
            let want = self.datas[id].type_params.len();
            let Some(args) = self.explicit(indices, want, name, span) else {
                return poisoned(span);
            };
            return self.construct((id, variant), Some(&args), None, &[], span);
        }
        if let Ast::Name(name) = &base.kind
            && let Some(id) = self.task_named(name)
        {
            let want = self.signatures[id].type_params.len();
            let Some(args) = self.explicit(indices, want, name, span) else {
                return poisoned(span);
            };
            return self.task_value(id, name, Some(&args), span);
        }
        let value = self.expr(base);
        let slice = match indices {
            [
                ast::Expr {
                    kind: Ast::Binary(op @ (BinaryOp::Range | BinaryOp::RangeInclusive), start, end),
                    ..
                },
            ] => Some((*op, start, end)),
            _ => None,
        };
        let is_list = matches!(value.ty, Ty::List(_));
        if value.ty != Ty::Text && !(is_list && slice.is_some()) {
            let Some((key, ty)) = self.key(value.ty, indices, span) else {
                return poisoned(span);
            };
            let kind = match value.ty {
                Ty::List(_) => ExprKind::ListGet(value.into(), key.into()),
                _ => ExprKind::MapGet(value.into(), key.into()),
            };
            return Expr { kind, ty, span };
        }
        let Some((op, start, end)) = slice else {
            let diag = Diagnostic::new("E0303", "text needs a range index", span);
            return self.report(diag.note("for one character, use `x[i..i + 1]`"));
        };
        let (start, mut end) = self.range(start, end);
        if op == BinaryOp::RangeInclusive {
            let span = end.span;
            let one = Expr { kind: ExprKind::Int(1), ty: Ty::Int, span };
            let kind = ExprKind::Binary(BinaryOp::Add, end.into(), one.into());
            end = Expr { kind, ty: Ty::Int, span };
        }
        let ty = value.ty;
        let kind = if is_list {
            ExprKind::ListSlice(value.into(), start.into(), end.into())
        } else {
            ExprKind::TextSlice(value.into(), start.into(), end.into())
        };
        Expr { kind, ty, span }
    }

    /// Checks an index of a list or map of type `ty`, or of a value that can
    /// have no index. Returns the index and the element type.
    fn key(&mut self, ty: Ty, indices: &'a [ast::Expr], span: Span) -> Option<(Expr, Ty)> {
        match (ty, indices) {
            (Ty::Error, _) => None,
            (
                Ty::List(_),
                [
                    ast::Expr {
                        kind: Ast::Binary(BinaryOp::Range | BinaryOp::RangeInclusive, ..),
                        ..
                    },
                ],
            ) => {
                let diag = Diagnostic::new("E0305", "a list slice cannot change", span);
                self.report(diag.note("a slice is a new list: write to the elements of the list"));
                None
            }
            (Ty::List(item), [index]) => {
                let index = self.expected(index, Some(Ty::Int));
                self.expect(&index, Ty::Int, "a list index");
                Some((index, *item))
            }
            (Ty::Map(key, value), [index]) => {
                let index = self.expected(index, Some(*key));
                self.expect(&index, *key, "a map key");
                Some((index, *value))
            }
            (Ty::List(_) | Ty::Map(..), _) => {
                self.fail("E0303", "this needs one index".into(), span);
                None
            }
            (Ty::Set(_), _) => {
                let diag = Diagnostic::new("E0303", "a set has no index", span);
                self.report(diag.note("to test for a value, use `x in items`"));
                None
            }
            (Ty::Text, _) => {
                self.fail("E0303", "text cannot change".into(), span);
                None
            }
            (Ty::Union(_), _) => {
                let message = format!("`{}` has no index", ty.name());
                self.report(Diagnostic::new("E0303", message, span).note(TEST_UNION));
                None
            }
            _ => {
                self.unsupported("indexing", span);
                None
            }
        }
    }

    fn is_test(
        &mut self,
        value: &'a ast::Expr,
        negated: bool,
        target: &'a ast::IsTarget,
        span: Span,
    ) -> Expr {
        let value = self.expr(value);
        let test = match self.is_target(target) {
            None => self.is_empty(value, span),
            Some(want) if want == Ty::Error || value.ty == Ty::Error => poisoned(span),
            Some(want) => {
                if self.split(value.ty, want).is_none() {
                    let foreign = matches!(value.ty, Ty::Union(_))
                        && want.members().iter().any(|member| !value.ty.members().contains(member));
                    let (want, ty) = (want.name(), value.ty.name());
                    let message = if foreign {
                        format!("`{ty}` never holds `{want}`")
                    } else {
                        format!("`is {want}` gives one answer for every `{ty}`")
                    };
                    let note = "`is` tests what a union holds, and `==` tests equality";
                    return self.report(Diagnostic::new("E0303", message, span).note(note));
                }
                let kind = ExprKind::Holds(value.into(), want.members().to_vec());
                Expr { kind, ty: Ty::Bool, span }
            }
        };
        if negated && test.ty != Ty::Error {
            return Expr { kind: ExprKind::Not(test.into()), ty: Ty::Bool, span };
        }
        test
    }

    fn is_empty(&mut self, value: Expr, span: Span) -> Expr {
        match value.ty {
            Ty::Error => poisoned(span),
            Ty::Text => Expr { kind: ExprKind::TextIsEmpty(value.into()), ty: Ty::Bool, span },
            ty if ty.is_collection() => {
                let int = |kind| Box::new(Expr { kind, ty: Ty::Int, span });
                let len = int(ExprKind::Len(value.into()));
                let kind = ExprKind::Binary(BinaryOp::Eq, len, int(ExprKind::Int(0)));
                Expr { kind, ty: Ty::Bool, span }
            }
            _ => self.method_call(value, None, "is_empty", &[], span),
        }
    }

    /// `x.get(i)` on a list, or `x.get(key)` on a map. A map's value may hold
    /// `none` itself, and `V?` then is `V`, as decision 39 says.
    fn get(&mut self, value: Expr, args: &'a [ast::Arg], span: Span) -> Expr {
        let (key_ty, item) = match value.ty {
            Ty::List(item) => (Ty::Int, *item),
            Ty::Map(key, item) => (*key, *item),
            _ => unreachable!("`builtin_method` allows only these"),
        };
        let [ast::Arg { name: None, change: false, value: key }] = args else {
            return self.fail("E0307", "`get` takes 1 positional argument".into(), span);
        };
        let key = self.expected(key, Some(key_ty));
        self.expect(&key, key_ty, "this argument");
        if key.ty != key_ty {
            return poisoned(span);
        }
        let ty = self.union([item, Ty::None]);
        let mut setup = Vec::new();
        let (value, key) = (self.repeated(value, &mut setup), self.repeated(key, &mut setup));
        let test = if matches!(value.ty, Ty::List(_)) {
            let zero = Expr { kind: ExprKind::Int(0), ty: Ty::Int, span };
            let len = Expr { kind: ExprKind::Len(value.clone().into()), ty: Ty::Int, span };
            let above = self.binary_values(BinaryOp::Ge, key.clone(), zero, span);
            let below = self.binary_values(BinaryOp::Lt, key.clone(), len, span);
            self.binary_values(BinaryOp::And, above, below, span)
        } else {
            let kind = ExprKind::MapHas(key.clone().into(), value.clone().into());
            Expr { kind, ty: Ty::Bool, span }
        };
        let kind = match value.ty {
            Ty::List(_) => ExprKind::ListGet(value.into(), key.into()),
            _ => ExprKind::MapGet(value.into(), key.into()),
        };
        let found = self.widen(Expr { kind, ty: item, span }, ty);
        let missing = self.widen(Expr { kind: ExprKind::None, ty: Ty::None, span }, ty);
        let kind = ExprKind::If(test.into(), found.into(), missing.into());
        let read = Expr { kind, ty, span };
        if setup.is_empty() {
            return read;
        }
        Expr { kind: ExprKind::Seq(setup, read.into()), ty, span }
    }

    /// `x.split(separator)` on a text, which traps on an empty separator.
    fn text_split(&mut self, value: Expr, args: &'a [ast::Arg], span: Span) -> Expr {
        let [ast::Arg { name: None, change: false, value: separator }] = args else {
            self.poison_args(args, span);
            return self.fail("E0307", "`split` takes 1 positional argument".into(), span);
        };
        let separator = self.expected(separator, Some(Ty::Text));
        self.expect(&separator, Ty::Text, "this argument");
        if separator.ty != Ty::Text {
            return poisoned(span);
        }
        let mut setup = Vec::new();
        let value = self.repeated(value, &mut setup);
        let separator = self.repeated(separator, &mut setup);
        let empty = ExprKind::TextIsEmpty(separator.clone().into());
        setup.push(Stmt::If {
            branches: vec![(
                Expr { kind: empty, ty: Ty::Bool, span },
                vec![Stmt::Trap("split by empty text", span)],
            )],
            otherwise: Vec::new(),
        });
        let kind = ExprKind::Foreign("lucid_text_split", vec![value, separator]);
        let ty = Ty::List(&Ty::Text);
        Expr { kind: ExprKind::Seq(setup, Expr { kind, ty, span }.into()), ty, span }
    }

    /// `x.to_int()` on a text: the `int` it spells, or `none`.
    fn text_to_int(&mut self, value: Expr, span: Span) -> Expr {
        let ty = self.union([Ty::Int, Ty::None]);
        let mut setup = Vec::new();
        let value = self.repeated(value, &mut setup);
        let kind = ExprKind::Foreign("lucid_text_is_int", vec![value.clone()]);
        let test = Expr { kind, ty: Ty::Bool, span };
        let kind = ExprKind::Foreign("lucid_text_int", vec![value]);
        let found = self.widen(Expr { kind, ty: Ty::Int, span }, ty);
        let missing = self.widen(Expr { kind: ExprKind::None, ty: Ty::None, span }, ty);
        let read = Expr { kind: ExprKind::If(test.into(), found.into(), missing.into()), ty, span };
        if setup.is_empty() {
            return read;
        }
        Expr { kind: ExprKind::Seq(setup, read.into()), ty, span }
    }

    /// Makes a value safe to read twice. A plain read stays as it is: a hidden
    /// local keeps a count until it is overwritten, so the next write to that
    /// list would copy.
    fn repeated(&mut self, value: Expr, setup: &mut Vec<Stmt>) -> Expr {
        if repeatable(&value) {
            return value;
        }
        let (span, local) = (value.span, self.new_local(value.ty, false));
        setup.push(Stmt::Set(local, value));
        self.read(local, span)
    }

    /// Reports a value that `print` and interpolation cannot show.
    fn showable(&mut self, value: Expr) -> Expr {
        if !self.require(value.ty, "Show", value.span) {
            return poisoned(value.span);
        }
        value
    }

    fn text(&mut self, parts: &'a [TextPart], span: Span) -> Expr {
        let mut values: Vec<Expr> = parts
            .iter()
            .map(|part| match part {
                TextPart::Literal(text) => {
                    Expr { kind: ExprKind::Text(text.clone()), ty: Ty::Text, span }
                }
                TextPart::Expr(value) => {
                    let value = self.expr(value);
                    let value = self.showable(value);
                    match value.ty {
                        Ty::Text | Ty::Error => value,
                        _ => {
                            let span = value.span;
                            Expr { kind: ExprKind::ToText(value.into()), ty: Ty::Text, span }
                        }
                    }
                }
            })
            .collect();
        match values.len() {
            0 => Expr { kind: ExprKind::Text(String::new()), ty: Ty::Text, span },
            1 => values.remove(0),
            _ => Expr { kind: ExprKind::Join(values), ty: Ty::Text, span },
        }
    }

    fn unary(&mut self, op: UnaryOp, value: &'a ast::Expr, hint: Option<Ty>, span: Span) -> Expr {
        match op {
            UnaryOp::Neg => {
                let value = self.lower(value, hint);
                match value.ty {
                    Ty::Error => poisoned(span),
                    ty if ty.is_float() => Expr { kind: ExprKind::Neg(value.into()), ty, span },
                    ty if ty.is_integer() && ty.int_range().0 < 0 => {
                        let zero = Expr { kind: ExprKind::Int(0), ty, span };
                        let kind = ExprKind::Binary(BinaryOp::Sub, zero.into(), value.into());
                        Expr { kind, ty, span }
                    }
                    ty if ty.is_integer() => {
                        let message = format!("`-` needs a signed number, found `{}`", ty.name());
                        self.fail("E0303", message, value.span)
                    }
                    ty => {
                        let message = format!("`-` needs a number, found `{}`", ty.name());
                        self.fail("E0303", message, value.span)
                    }
                }
            }
            // All bits set, in the width and extension of the type.
            UnaryOp::BitNot => {
                let value = self.lower(value, hint);
                match value.ty {
                    Ty::Error => poisoned(span),
                    ty if ty.is_integer() => {
                        let ones =
                            if ty.int_range().0 < 0 { -1 } else { ty.int_range().1 as u64 as i64 };
                        let ones = Expr { kind: ExprKind::Int(ones), ty, span };
                        let kind = ExprKind::Binary(BinaryOp::BitXor, value.into(), ones.into());
                        Expr { kind, ty, span }
                    }
                    ty => {
                        let message = format!("`~` needs an integer, found `{}`", ty.name());
                        self.fail("E0303", message, value.span)
                    }
                }
            }
            UnaryOp::Not => {
                let value = self.condition(value, "`not`");
                Expr { kind: ExprKind::Not(value.into()), ty: Ty::Bool, span }
            }
            UnaryOp::Try => self.try_value(value, span),
            UnaryOp::Spawn => self.spawn(value, span),
            UnaryOp::Shared | UnaryOp::Weak => self.unsupported("`shared` and `weak`", span),
        }
    }

    /// A literal takes the number type of the other side, or else `hint`.
    fn binary(
        &mut self,
        op: BinaryOp,
        left: &'a ast::Expr,
        right: &'a ast::Expr,
        hint: Option<Ty>,
        span: Span,
    ) -> Expr {
        match op {
            BinaryOp::Fallback => unreachable!("`lower` and `expr` take this first"),
            BinaryOp::In | BinaryOp::NotIn => return self.contains(op, left, right, span),
            BinaryOp::Range | BinaryOp::RangeInclusive => {
                return self.unsupported("a range outside `for`", span);
            }
            BinaryOp::And | BinaryOp::Or => {
                let first = self.expr(left);
                let start = self.frame.narrowed.len();
                let facts = self.facts(left, op == BinaryOp::And);
                self.frame.narrowed.extend(facts);
                let right = self.expr(right);
                self.frame.narrowed.truncate(start);
                return self.binary_values(op, first, right, span);
            }
            _ => {}
        }
        let (left, right) = if literal_math(left) && !literal_math(right) {
            let right = self.lower(right, hint);
            let want = self.number_want(Some(right.ty)).or(hint);
            (self.lower(left, want), right)
        } else {
            let first = self.lower(left, hint);
            // A collection literal takes its element type from the other side.
            let want = if matches!(right.kind, Ast::List(_) | Ast::Map(_)) {
                Some(first.ty).filter(|ty| ty.is_collection())
            } else {
                self.number_want(Some(first.ty)).or(hint)
            };
            let right = self.lower(right, want);
            (first, right)
        };
        self.binary_values(op, left, right, span)
    }

    fn contains(
        &mut self,
        op: BinaryOp,
        left: &'a ast::Expr,
        right: &'a ast::Expr,
        span: Span,
    ) -> Expr {
        // A literal takes its type from the other side, so that side goes first.
        let first = (!literal_math(left)).then(|| self.expr(left));
        let want = first.as_ref().map(|value| value.ty).filter(|ty| ty.is_number());
        let collection = want.and_then(|item| match &right.kind {
            Ast::List(_) => Some(Ty::List(inner(item))),
            Ast::Call { callee, .. }
                if matches!(&callee.kind, Ast::Name(name) if self.is_set_call(name)) =>
            {
                Some(Ty::Set(inner(item)))
            }
            _ => None,
        });
        let right = self.lower(right, collection);
        let (item, list) = match right.ty {
            Ty::Error => {
                if first.is_none() {
                    self.expr(left);
                }
                return poisoned(span);
            }
            Ty::List(item) => (*item, true),
            Ty::Set(item) | Ty::Map(item, _) => (*item, false),
            ty => {
                if first.is_none() {
                    self.expr(left);
                }
                let message = format!("`in` needs a list, map, or set, found `{}`", ty.name());
                return self.fail("E0303", message, right.span);
            }
        };
        let left = match first {
            Some(left) => left,
            None => self.lower(left, self.number_want(Some(item))),
        };
        if list && !self.require(item, "Equal", right.span) {
            return poisoned(span);
        }
        let left = self.widen(left, item);
        self.expect(&left, item, "`in`");
        let kind = if list {
            ExprKind::ListHas(left.into(), right.into())
        } else {
            ExprKind::MapHas(left.into(), right.into())
        };
        let found = Expr { kind, ty: Ty::Bool, span };
        if op == BinaryOp::In {
            return found;
        }
        Expr { kind: ExprKind::Not(found.into()), ty: Ty::Bool, span }
    }

    fn binary_values(&mut self, op: BinaryOp, left: Expr, right: Expr, span: Span) -> Expr {
        if left.ty == Ty::Error || right.ty == Ty::Error {
            return poisoned(span);
        }
        let name = symbol(op);
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            self.expect(&left, Ty::Bool, &format!("`{name}`"));
            self.expect(&right, Ty::Bool, &format!("`{name}`"));
            let (then, otherwise) = if op == BinaryOp::And {
                (right, Expr { kind: ExprKind::Bool(false), ty: Ty::Bool, span })
            } else {
                (Expr { kind: ExprKind::Bool(true), ty: Ty::Bool, span }, right)
            };
            let kind = ExprKind::If(left.into(), then.into(), otherwise.into());
            return Expr { kind, ty: Ty::Bool, span };
        }
        if left.ty != right.ty {
            let (a, b) = (left.ty.name(), right.ty.name());
            let message = format!("`{name}` needs one type on both sides, found `{a}` and `{b}`");
            let diag = Diagnostic::new("E0303", message, span);
            let union =
                [left.ty, right.ty].iter().any(|ty| *ty == Ty::None || matches!(ty, Ty::Union(_)));
            let numbers = left.ty.is_number() && right.ty.is_number();
            let plain = matches!((left.ty, right.ty), (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int));
            return self.report(if union && !self.frames.is_empty() {
                diag.note("test a union with `is`. A captured name does not narrow, so test a `let` copy of it")
            } else if union {
                diag.note("narrow a union with `is` first, as in `x is none`, or give a fallback with `else`")
            } else if left.ty == Ty::Text || right.ty == Ty::Text {
                diag.note(JOIN_TEXT)
            } else if numbers && !plain {
                diag.note(format!("conversions are explicit: use `.to_{a}()`"))
            } else if numbers {
                diag.note("conversions are explicit: use `.to_float()` or `.to_int()`")
            } else {
                diag
            });
        }
        let ty = left.ty;
        let equality = matches!(op, BinaryOp::Eq | BinaryOp::Ne);
        let order = matches!(op, BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge);
        let int_only = is_math(op)
            && !matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div);
        let valid = match ty {
            ty if ty.is_integer() => true,
            ty if ty.is_float() => !int_only,
            Ty::Bool => equality,
            Ty::Text => equality || order,
            ty if (ty.is_compound()
                || ty.is_handle()
                || matches!(ty, Ty::Param(..) | Ty::Task(..)))
                && (equality || order) =>
            {
                let trait_name = if equality { "Equal" } else { "Order" };
                if !self.require(ty, trait_name, span) {
                    return poisoned(span);
                }
                true
            }
            ty if (ty.is_compound() || matches!(ty, Ty::Param(..))) && !int_only => {
                let trait_name = match op {
                    BinaryOp::Add => "Add",
                    BinaryOp::Sub => "Sub",
                    BinaryOp::Mul => "Mul",
                    _ => "Div",
                };
                if !self.require(ty, trait_name, span) {
                    return poisoned(span);
                }
                true
            }
            _ => false,
        };
        if !valid {
            let needs = if int_only { "`int`" } else { "a number" };
            let message = format!("`{name}` needs {needs}, found `{}`", ty.name());
            let diag = Diagnostic::new("E0303", message, left.span);
            return self.report(if ty == Ty::Text && op == BinaryOp::Add {
                diag.note(JOIN_TEXT)
            } else {
                diag
            });
        }
        let result = match op {
            _ if equality || order => Ty::Bool,
            BinaryOp::Div if ty.is_integer() => Ty::Float,
            _ => ty,
        };
        let float = |value: Expr| {
            let span = value.span;
            Expr { kind: ExprKind::Convert(value.into()), ty: Ty::Float, span }
        };
        let (left, right) = if op == BinaryOp::Div && ty.is_integer() {
            (float(left), float(right))
        } else {
            (left, right)
        };
        Expr { kind: ExprKind::Binary(op, left.into(), right.into()), ty: result, span }
    }

    fn call(
        &mut self,
        callee: &'a ast::Expr,
        args: &'a [ast::Arg],
        want: Option<Ty>,
        span: Span,
    ) -> Expr {
        match &callee.kind {
            Ast::Field { base, name } => {
                let reported = self.diags.len();
                let value = self.expr(base);
                self.dot_call(value, (base, reported), name, args, span)
            }
            Ast::Name(name) if self.is_set_call(name) => self.set_call(args, want, span),
            Ast::Name(name) if self.is_built_in_call(name, "channel") => {
                self.new_channel(None, args, want, span)
            }
            Ast::Name(name) if self.is_built_in_call(name, "lock") => {
                self.new_lock(args, want, span)
            }
            Ast::Index { base, indices } if matches!(&base.kind, Ast::Name(name) if self.is_built_in_call(name, "channel")) => {
                self.new_channel(Some(indices), args, want, span)
            }
            Ast::Index { base, indices } => {
                if let Ast::Name(name) = &base.kind {
                    if let Some(target) = self.constructor(name) {
                        let count = self.datas[target.0].type_params.len();
                        let Some(types) = self.explicit(indices, count, name, span) else {
                            return self.poison_args(args, span);
                        };
                        return self.construct_call(target, name, Some(&types), None, args, span);
                    }
                    if let Some(id) = self.task_named(name) {
                        let count = self.signatures[id].type_params.len();
                        let Some(types) = self.explicit(indices, count, name, span) else {
                            return self.poison_args(args, span);
                        };
                        return self.task_call(id, name, Some(&types), None, args, span);
                    }
                }
                let value = self.expr(callee);
                self.call_value(value, args, span)
            }
            Ast::Name(name) => {
                if let Some(target) = self.constructor(name) {
                    return self.construct_call(target, name, None, want, args, span);
                }
                if let Some(id) = self.task_named(name) {
                    return self.task_call(id, name, None, None, args, span);
                }
                if self.visible(name) || self.top.contains_key(name.as_str()) {
                    let value = self.name(name, callee.span);
                    if matches!(value.ty, Ty::Task(..) | Ty::Error) {
                        return self.call_value(value, args, span);
                    }
                    self.poison_args(args, span);
                    return self.fail("E0308", format!("`{name}` is not a task"), callee.span);
                }
                self.poison_args(args, span);
                let diag =
                    Diagnostic::new("E0301", format!("name `{name}` is not defined"), callee.span);
                let dotted = self.methods.contains_key(name.as_str());
                let note = format!("`{name}` has `self`, so call it with a dot: `x.{name}()`");
                let diag =
                    if dotted { diag.note(note) } else { self.similar(diag, name, callee.span) };
                self.report(diag)
            }
            _ => {
                let value = self.expr(callee);
                self.call_value(value, args, span)
            }
        }
    }

    /// Named arguments follow positional ones, in parameter order, so that the
    /// order of evaluation is always the order on the page.
    fn task_call(
        &mut self,
        id: usize,
        name: &str,
        explicit: Option<&[Ty]>,
        receiver: Option<Receiver<'a>>,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Expr {
        if self.foreign.contains(&id) {
            self.need_unsafe(&format!("`{name}`, an `extern` task,"), span);
        }
        let reported = self.diags.len();
        let opened = self.frame.opened.len();
        let (receiver, seeded, written) = match receiver {
            Some(Receiver { value, known, changed }) => (Some(value), Some(known), changed),
            None => (None, None, None),
        };
        let mut known: Vec<Option<Ty>> = match explicit {
            Some(types) => types.iter().copied().map(Some).collect(),
            None => seeded.unwrap_or(vec![None; self.signatures[id].type_params.len()]),
        };
        let count = self.signatures[id].params.len();
        let required = self.signatures[id].params.iter().filter(|p| p.default.is_none()).count();
        let mut values: Vec<Option<Expr>> = vec![None; count];
        let mut next = usize::from(receiver.is_some());
        values[..next].clone_from_slice(&[receiver][..next]);
        let mut places: Vec<Option<Place>> = (0..count).map(|_| None).collect();
        let (mut changed, mut peers) = (Vec::new(), Vec::new());
        if let Some((base, place)) = written {
            peers.push(base.span);
            changed.push(base);
            places[0] = Some(place);
        }
        let mut named = false;
        for arg in args {
            let params = &self.signatures[id].params;
            let want = match &arg.name {
                None => params.get(next),
                Some(label) => params.iter().find(|p| p.name == label.text),
            };
            let pattern = want.map_or(Ty::Error, |p| p.ty);
            let mode = want.map(|p| p.mode);
            let span = arg.value.span;
            let wrong_form = match (mode, arg.change) {
                (None, _) | (Some(ast::ParamMode::Change), true) | (_, false) => None,
                _ => Some(format!("this parameter of `{name}` is not `change`")),
            };
            if let Some(message) = wrong_form {
                self.fail("E0307", message, span);
            }
            if mode == Some(ast::ParamMode::Change) && !arg.change {
                let message = format!("`{name}` changes this argument, so mark it");
                let diag = Diagnostic::new("E0307", message, span);
                self.report(diag.note(if arg.name.is_some() {
                    "pass it by position, as `change x`"
                } else {
                    "write `change` before it"
                }));
            }
            let before = self.frame.changed.len();
            let mut place = None;
            let value = if mode == Some(ast::ParamMode::Change) && arg.change {
                peers.push(span);
                changed.push(&arg.value);
                place = self.change_place(&arg.value, pattern);
                match &place {
                    Some(place) => {
                        let ty = self.frame.locals[place.local];
                        self.table.unify(pattern, ty, &mut known);
                        Expr { kind: ExprKind::Change(place.local), ty, span }
                    }
                    None => poisoned(span),
                }
            } else {
                self.inferred(&arg.value, pattern, &mut known)
            };
            if !arg.change
                && !value.ty.is_counted()
                && value.ty != Ty::Error
                && self.frame.changed.len() == before
            {
                peers.push(span);
            }
            let slot = match &arg.name {
                None if named => Err("a positional argument cannot follow a named one".into()),
                None if next >= required => {
                    Err(format!("`{name}` has too many positional arguments"))
                }
                None => Ok(next),
                Some(label) => {
                    named = true;
                    let params = &self.signatures[id].params;
                    match params.iter().position(|p| p.name == label.text) {
                        None => Err(format!("`{name}` has no parameter `{}`", label.text)),
                        Some(slot) if params[slot].default.is_none() => {
                            Err(format!("`{}` has no default, so pass it by position", label.text))
                        }
                        Some(slot) if slot < next => {
                            Err(format!("`{}` is out of parameter order", label.text))
                        }
                        Some(slot) => Ok(slot),
                    }
                }
            };
            match slot {
                Ok(slot) => {
                    values[slot] = Some(value);
                    places[slot] = place;
                    next = slot + 1;
                }
                Err(message) => {
                    self.fail("E0307", message, arg.value.span);
                }
            }
        }
        self.frame.opened.truncate(opened);
        for arg in changed {
            let Some((root, path)) = arg.path() else { continue };
            let span = arg.span;
            self.frame.changed.push(Changed { root, path, span, peers: peers.clone() });
        }
        for (slot, value) in values.iter().enumerate() {
            let param = &self.signatures[id].params[slot];
            if value.is_none() && param.default.is_none() {
                let message = format!("`{name}` is missing argument `{}`", param.name);
                self.fail("E0307", message, span);
            }
        }
        let params: Vec<&str> =
            self.signatures[id].type_params.iter().map(|&(name, _)| name).collect();
        let Some(types) = self.solved(name, &params, &known, reported, span) else {
            return poisoned(span);
        };
        let bounds = self.signatures[id].bounds.clone();
        for (&ty, traits) in types.iter().zip(bounds) {
            for name in traits {
                self.require(ty, name, span);
            }
        }
        let (mut setup, mut take, mut finish) = (Vec::new(), Vec::new(), Vec::new());
        let paths = places.iter().flatten().any(|place| !place.setup.is_empty());
        let mut changing: Vec<Place> = Vec::new();
        let mut lowered = Vec::with_capacity(count);
        for (slot, value) in values.into_iter().enumerate() {
            let param = &self.signatures[id].params[slot];
            let default = param.default.clone();
            let ty = param.ty;
            let ty = self.table.subst(ty, types);
            if let (Some(value), Some(place)) = (&value, places[slot].take()) {
                if ty != Ty::Error && value.ty != Ty::Error && ty != value.ty {
                    let (want, found) = (ty.name(), value.ty.name());
                    let message = format!("a `change` argument must be `{want}`, found `{found}`");
                    let diag = Diagnostic::new("E0303", message, value.span);
                    self.report(
                        diag.note("the task may store any such value, so types must match"),
                    );
                }
                lowered.push(value.clone());
                setup.extend(place.setup.iter().cloned());
                take.extend(place.take.iter().cloned());
                finish.splice(0..0, place.finish.iter().cloned());
                changing.push(place);
            } else if let Some(value) = value {
                let value = self.settled(value, ty, "this argument");
                let plain = matches!(value.kind, ExprKind::Local(_) | ExprKind::Int(_));
                lowered.push(if paths && !plain {
                    let span = value.span;
                    let held = self.hoist(value, &mut setup, &mut finish);
                    self.read(held, span)
                } else {
                    value
                });
            } else if let Some(value) = default {
                lowered.push(Expr { ty, ..value });
            }
        }
        self.check_overlap(&changing, &mut setup, span);
        let returns = self.signatures[id].returns;
        let ty = self.table.subst(returns, types);
        let kind = if self.signatures[id].in_trait {
            ExprKind::Method(self.signatures[id].name.text.clone().leak(), lowered)
        } else {
            ExprKind::Call(id, types, lowered)
        };
        let call = Expr { kind, ty, span };
        if setup.is_empty() && take.is_empty() && finish.is_empty() {
            return call;
        }
        let (last, value) = if ty == Ty::None {
            (Stmt::Expr(call), Expr { kind: ExprKind::None, ty, span })
        } else {
            let result = self.new_local(ty, false);
            (Stmt::Set(result, call), self.read(result, span))
        };
        setup.append(&mut take);
        setup.push(last);
        setup.append(&mut finish);
        Expr { kind: ExprKind::Seq(setup, value.into()), ty, span }
    }

    /// Finds the place of a `change` argument. A name whose declared type is
    /// the one wanted goes whole, and may come back with any member.
    fn change_place(&mut self, target: &'a ast::Expr, pattern: Ty) -> Option<Place> {
        let name = match &target.kind {
            Ast::Name(name) => Some(name.as_str()),
            Ast::SelfValue => Some("self"),
            _ => None,
        };
        if let Some(local) = name.and_then(|name| self.lookup(name)) {
            self.mark_read(local);
            // A union narrowed to a smaller union also goes whole.
            if self.frame.locals[local] == pattern || matches!(self.current(local), Ty::Union(_)) {
                self.forget(local);
            }
        }
        self.check_write(target);
        let opening = std::mem::replace(&mut self.frame.opening, true);
        let place = self.place(target);
        self.frame.opening = opening;
        place
    }

    /// Two `change` arguments must not meet. Fields can be told apart here. Two
    /// indexes may be equal, so they are tested when the call runs.
    fn check_overlap(&mut self, places: &[Place], setup: &mut Vec<Stmt>, span: Span) {
        for (index, first) in places.iter().enumerate() {
            for second in places[index + 1..].iter().filter(|p| p.root == first.root) {
                let differs = first.path.iter().zip(&second.path).find(|(a, b)| a != b);
                let (Step::Key(a), Step::Key(b)) = (match differs {
                    Some((Step::Slot(_), Step::Slot(_))) => continue,
                    Some((&a, &b)) => (a, b),
                    None => {
                        let message = "two `change` arguments of this call overlap";
                        let note = "each one needs a value of its own";
                        self.report(Diagnostic::new("E0326", message, span).note(note));
                        continue;
                    }
                }) else {
                    continue;
                };
                let (a, b) = (self.read(a, span), self.read(b, span));
                let same = self.binary_values(BinaryOp::Eq, a, b, span);
                let trap = Stmt::Trap("two `change` arguments overlap", span);
                let otherwise = Vec::new();
                setup.push(Stmt::If { branches: vec![(same, vec![trap])], otherwise });
            }
        }
    }

    /// The task of this name in the traits of a type parameter or an `any`.
    fn trait_task(&self, ty: Ty, name: &str) -> Option<usize> {
        let traits = match ty {
            Ty::Param(_, index) => self.bounds.get(index).cloned().unwrap_or_default(),
            Ty::Any(held) => vec![held],
            _ => Vec::new(),
        };
        let tasks = traits.iter().filter_map(|t| self.traits.get(t)).flatten();
        tasks.copied().find(|&id| self.signatures[id].name.text == name)
    }

    /// The position and type of a field of a `type` that holds a task value.
    fn task_field(&mut self, ty: Ty, name: &str) -> Option<(usize, Ty)> {
        let Ty::Data(_, id, _) = ty else { return None };
        if self.datas[id].choice {
            return None;
        }
        let fields = &self.datas[id].variants[0].fields;
        let index = fields.iter().position(|field| field.name == name)?;
        let field = fields[index].ty;
        let field = self.subst_data(field, ty);
        matches!(field, Ty::Task(..)).then_some((index, field))
    }

    /// `x.f(a)` for a task with a `self` parameter. On a type parameter or an
    /// `any` it looks in their traits, and `mono.rs` finds the real task.
    ///
    /// `written` is the source of the receiver, with the diagnostics count from
    /// before it was lowered. `x is empty` has none, as it has no dot.
    fn method_call(
        &mut self,
        value: Expr,
        written: Option<(&'a ast::Expr, usize)>,
        name: &str,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Expr {
        let ty = value.ty;
        let found = self.trait_task(ty, name).map(|id| (id, Vec::new()));
        if let Some((id, known)) = found.or_else(|| self.find_method(name, ty)) {
            self.check_method_visible(id, span);
            if matches!(ty, Ty::Any(_)) && self.signatures[id].in_trait && self.changes(id) {
                self.poison_args(args, span);
                let message =
                    format!("`{name}` has a `change` parameter, so an `any` cannot call it");
                return self.fail("E0307", message, span);
            }
            if self.signatures[id].params[0].mode != ast::ParamMode::Change {
                let receiver = Receiver { value, known, changed: None };
                return self.task_call(id, name, None, Some(receiver), args, span);
            }
            let Some((base, reported)) = written else {
                let message = format!("`{name}` changes its receiver, so it needs a dot call");
                return self.fail("E0307", message, span);
            };
            // It was read once to find its type, which reports the same errors.
            self.diags.truncate(reported);
            let opened = self.frame.opened.len();
            let Some(place) = self.change_place(base, Ty::Error) else {
                return self.poison_args(args, span);
            };
            let ty = self.frame.locals[place.local];
            let value = Expr { kind: ExprKind::Change(place.local), ty, span: base.span };
            let receiver = Receiver { value, known, changed: Some((base, place)) };
            let result = self.task_call(id, name, None, Some(receiver), args, span);
            self.frame.opened.truncate(opened);
            return result;
        }
        if let Some((field, ty)) = self.task_field(ty, name) {
            let span = value.span;
            let kind = ExprKind::Member { value: value.into(), variant: 0, field };
            return self.call_value(Expr { kind, ty, span }, args, span);
        }
        let poison = self.poison_args(args, span);
        let free = self.task_named(name).is_some();
        let diag =
            Diagnostic::new("E0303", format!("`{name}` does not work on `{}`", ty.name()), span);
        self.report(if matches!(ty, Ty::Union(_)) {
            diag.note(TEST_UNION)
        } else if free {
            diag.note(format!("`{name}` has no `self`, so call it as `{name}(x)`"))
        } else if matches!(ty, Ty::Param(..)) {
            diag.note("a bound with this task lets a type parameter use it")
        } else {
            diag
        });
        poison
    }

    /// A private task with `self` can only be called in its own module. It
    /// still counts for traits, so a type means one thing everywhere.
    fn check_method_visible(&mut self, id: usize, span: Span) {
        let signature = &self.signatures[id];
        let home = file_index(self.files, signature.name.span.start);
        if !signature.in_trait && !signature.public && home != file_index(self.files, span.start) {
            let message = format!("`{}` is private to its module", signature.name.text);
            let note = "mark it `public` in its module to use it here";
            self.report(Diagnostic::new("E0333", message, span).note(note));
        }
    }

    /// A dot call: a built-in task on a built-in type, or else a method.
    fn dot_call(
        &mut self,
        value: Expr,
        written: (&'a ast::Expr, usize),
        name: &'a ast::Name,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Expr {
        let ty = value.ty;
        if ty == Ty::Error {
            return self.poison_args(args, span);
        }
        let text = name.text.as_str();
        if !builtin_method(ty, text) {
            return self.method_call(value, Some(written), text, args, span);
        }
        if text == "pop" {
            let message = "`pop` changes its list, so it must be a whole statement, or all of \
                           the value of `let`, `var`, `return`, or an assignment to a name";
            return self.fail("E0303", message.into(), span);
        }
        if text == "receive" {
            let message = "`receive` waits for a value, so it must be a whole statement, or all \
                           of the value of `let`, `var`, `return`, or an assignment to a name";
            return self.fail("E0303", message.into(), span);
        }
        if CHANGING_CALLS.contains(&text) || THREAD_CALLS.contains(&text) {
            let message = format!("`{text}` gives no value, so it must be a statement");
            return self.fail("E0303", message, span);
        }
        if text == "get" {
            return self.get(value, args, span);
        }
        if text == "split" {
            return self.text_split(value, args, span);
        }
        if let Some(op) = wrapping_op(ty, text) {
            let [ast::Arg { name: None, change: false, value: other }] = args else {
                return self.fail("E0307", format!("`{text}` takes 1 positional argument"), span);
            };
            let other = self.expected(other, Some(ty));
            self.expect(&other, ty, "this argument");
            return Expr { kind: ExprKind::Wrapping(op, value.into(), other.into()), ty, span };
        }
        if !args.is_empty() {
            self.poison_args(args, span);
            return self.fail("E0307", format!("`{text}` takes no arguments"), span);
        }
        if let Some(ty) = conversion(ty, text) {
            return Expr { kind: ExprKind::Convert(value.into()), ty, span };
        }
        let int = |kind| Expr { kind, ty: Ty::Int, span };
        let boxed: Box<Expr> = value.clone().into();
        match (ty, text) {
            (_, "sqrt") => Expr { kind: ExprKind::Sqrt(boxed), ty, span },
            (Ty::Text, "len") => int(ExprKind::TextLen(boxed)),
            (Ty::Text, "bytes") => {
                Expr { kind: ExprKind::TextBytes(boxed), ty: Ty::List(&Ty::Byte), span }
            }
            (Ty::Text, "upper") => {
                Expr { kind: ExprKind::Foreign("lucid_text_upper", vec![value]), ty, span }
            }
            (Ty::Text, "lower") => {
                Expr { kind: ExprKind::Foreign("lucid_text_lower", vec![value]), ty, span }
            }
            (Ty::Text, "to_int") => self.text_to_int(value, span),
            (Ty::Spawned(item), "wait") => self.first_of("lucid_wait", value, *item, span),
            (Ty::Lock(item), "read") => self.first_of("lucid_lock_read", value, *item, span),
            (_, "len") => int(ExprKind::Len(boxed)),
            (_, "pointer") => {
                self.need_unsafe("`.pointer()`", span);
                let name = match &value.kind {
                    ExprKind::View(inner) => &inner.kind,
                    kind => kind,
                };
                if !matches!(name, ExprKind::Local(_)) {
                    // A temporary would be gone by the time the address is used.
                    self.fail("E0331", "`.pointer()` needs a name".into(), span);
                }
                Expr { kind: ExprKind::ListPointer(boxed), ty: Ty::Pointer, span }
            }
            _ => self.is_empty(value, span),
        }
    }

    /// `spawn f(x)`: a task value with no parameters, which `lucid_spawn` runs
    /// on a new thread. The arguments run here, as its captures, and its code
    /// calls `f` with them. It gives a list that holds the result, so the
    /// runtime calls one shape of code, whatever the result type.
    fn spawn(&mut self, call: &'a ast::Expr, span: Span) -> Expr {
        let wrong = "`spawn` takes a call of a task, as in `spawn f(x)`";
        if !matches!(call.kind, Ast::Call { .. }) {
            self.expected(call, None);
            return self.fail("E0341", wrong.into(), span);
        }
        let call = self.expected(call, None);
        if call.ty == Ty::Error {
            return poisoned(span);
        }
        let returns = call.ty;
        // A task's id and type arguments, or else a task value, first.
        let (values, callee) = match call.kind {
            ExprKind::Call(id, types, args) => {
                if args.iter().any(|arg| matches!(arg.kind, ExprKind::Change(_))) {
                    let message = "a `change` argument cannot go to another thread";
                    let note = "pass the value, and give the new one back as the result";
                    return self.report(Diagnostic::new("E0341", message, span).note(note));
                }
                (args, Some((id, types)))
            }
            ExprKind::CallValue(callee, args) => {
                (std::iter::once(*callee).chain(args).collect(), None)
            }
            _ => return self.fail("E0341", wrong.into(), span),
        };
        let captures: Vec<Ty> = values.iter().map(|value| value.ty).collect();
        let read = |(index, &ty): (usize, &Ty)| Expr { kind: ExprKind::Capture(index), ty, span };
        let mut reads: Vec<Expr> = captures.iter().enumerate().map(read).collect();
        let kind = match callee {
            Some((id, types)) => ExprKind::Call(id, types, reads),
            None => {
                let callee = reads.remove(0);
                ExprKind::CallValue(callee.into(), reads)
            }
        };
        let call = Expr { kind, ty: returns, span };
        let list = Ty::List(inner(returns));
        let result = Expr { kind: ExprKind::ListOf(vec![call]), ty: list, span };
        let ty = Ty::Task(&[], inner(list));
        let (type_params, args) = self.own_type_args();
        let (change, take, borrowed) = (Vec::new(), Vec::new(), Vec::new());
        self.lifted.push(Task {
            name: "",
            params: 1,
            type_params,
            returns: list,
            locals: vec![ty],
            body: vec![Stmt::Return(result)],
            span,
            captures,
            change,
            take,
            borrowed,
        });
        let task = self.first_lifted + self.lifted.len() - 1;
        let fields = values.into_iter().enumerate().collect();
        let closure = Expr { kind: ExprKind::Closure { task, args, counted: 0, fields }, ty, span };
        let scope = match self.frame.spawns {
            Some((local, _)) => local,
            None => {
                let local = self.new_local(Ty::Spawned(&Ty::None), false);
                self.frame.spawns = Some((local, span));
                local
            }
        };
        let scope = Expr { kind: ExprKind::Local(scope), ty: Ty::Spawned(&Ty::None), span };
        let kind = ExprKind::Foreign("lucid_spawn", vec![scope, closure]);
        Expr { kind, ty: Ty::Spawned(inner(returns)), span }
    }

    /// Starts the hidden scope of a task that spawns. Its last count joins
    /// every thread it holds, which the exit block gives up when the task
    /// ends, so nothing outlives the task that ran it, by decision 171.
    fn open_spawns(&mut self, body: &mut Vec<Stmt>) {
        if let Some((local, span)) = self.frame.spawns.take() {
            let ty = self.frame.locals[local];
            let start = Expr { kind: ExprKind::Foreign("lucid_spawn_scope", Vec::new()), ty, span };
            body.insert(0, Stmt::Set(local, start));
        }
    }

    /// The one item of the list that the runtime task `symbol` gives for
    /// `value`: the result of a thread for `.wait()`, or a copy of what a lock
    /// holds for `.read()`.
    fn first_of(&mut self, symbol: &'static str, value: Expr, item: Ty, span: Span) -> Expr {
        let list = Ty::List(inner(item));
        let results = Expr { kind: ExprKind::Foreign(symbol, vec![value]), ty: list, span };
        let results = self.checked(results);
        let first = Expr { kind: ExprKind::Int(0), ty: Ty::Int, span };
        Expr { kind: ExprKind::ListGet(results.into(), first.into()), ty: item, span }
    }

    /// Traps here if the runtime task for threads that just ran failed: for a
    /// deadlock, a lock this thread holds, a closed channel, or a capacity
    /// below 1. The runtime finds these, and the trap shows this place.
    fn failure_check(&self, span: Span) -> Stmt {
        let failed = ExprKind::Foreign("lucid_thread_failed", Vec::new());
        let failed = Expr { kind: failed, ty: Ty::Bool, span };
        let why = ExprKind::Foreign("lucid_thread_failure", Vec::new());
        let why = Expr { kind: why, ty: Ty::Text, span };
        Stmt::If { branches: vec![(failed, vec![Stmt::Fail(why, span)])], otherwise: Vec::new() }
    }

    /// `call`, a runtime task for threads, then `failure_check`.
    fn checked(&mut self, call: Expr) -> Expr {
        let (ty, span) = (call.ty, call.span);
        let local = self.new_local(ty, false);
        let setup = vec![Stmt::Set(local, call), self.failure_check(span)];
        Expr { kind: ExprKind::Seq(setup, self.read(local, span).into()), ty, span }
    }

    /// The one positional argument of a built-in call, or E0307.
    fn one_arg(&mut self, what: &str, args: &'a [ast::Arg], span: Span) -> Option<&'a ast::Expr> {
        if let [ast::Arg { name: None, change: false, value }] = args {
            return Some(value);
        }
        self.poison_args(args, span);
        self.fail("E0307", format!("`{what}` takes one positional argument"), span);
        None
    }

    /// A list of one item, which is how the runtime holds a value of any type
    /// in a channel or a lock.
    fn boxed_one(&mut self, value: Expr, item: Ty) -> Expr {
        let span = value.span;
        let value = self.widen(value, item);
        Expr { kind: ExprKind::ListOf(vec![value]), ty: Ty::List(inner(item)), span }
    }

    /// `channel[T](capacity)`, or `channel(capacity)` where the context gives
    /// the type.
    fn new_channel(
        &mut self,
        indices: Option<&'a [ast::Expr]>,
        args: &'a [ast::Arg],
        want: Option<Ty>,
        span: Span,
    ) -> Expr {
        let want = self.literal_want(want, |ty| matches!(ty, Ty::Channel(_)));
        let item = match (indices, want) {
            (Some(indices), _) => match self.explicit(indices, 1, "channel", span) {
                Some(types) => Some(types[0]),
                None => return self.poison_args(args, span),
            },
            (None, Some(Ty::Channel(item))) => Some(*item),
            (None, Some(Ty::Error)) => return self.poison_args(args, span),
            (None, _) => None,
        };
        let Some(capacity) = self.one_arg("channel", args, span) else { return poisoned(span) };
        let capacity = self.expected(capacity, Some(Ty::Int));
        self.expect(&capacity, Ty::Int, "the capacity of a channel");
        let Some(item) = item else {
            let message = "a channel needs the type of what it holds";
            let diag = Diagnostic::new("E0324", message, span);
            return self.report(diag.note("write it, as in `channel[int](16)`"));
        };
        let kind = ExprKind::Foreign("lucid_channel", vec![capacity]);
        self.checked(Expr { kind, ty: Ty::Channel(inner(item)), span })
    }

    /// `lock(value)`, which holds the type of `value`, or the one that the
    /// context gives.
    fn new_lock(&mut self, args: &'a [ast::Arg], want: Option<Ty>, span: Span) -> Expr {
        let want = self.literal_want(want, |ty| matches!(ty, Ty::Lock(_)));
        let item = match want {
            Some(Ty::Lock(item)) => Some(*item),
            Some(Ty::Error) => Some(Ty::Error),
            _ => None,
        };
        let Some(value) = self.one_arg("lock", args, span) else { return poisoned(span) };
        let value = self.expected(value, item);
        let item = item.unwrap_or(value.ty);
        self.expect(&value, item, "the value of a lock");
        if item == Ty::Error {
            return poisoned(span);
        }
        let list = self.boxed_one(value, item);
        Expr { kind: ExprKind::Foreign("lucid_lock", vec![list]), ty: Ty::Lock(inner(item)), span }
    }

    /// `send`, `close`, `receive`, and `update`, as statements.
    fn thread_call(
        &mut self,
        base: &'a ast::Expr,
        name: &'a ast::Name,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Stmt {
        let handle = self.expr(base);
        let none = |kind| Stmt::Expr(Expr { kind, ty: Ty::None, span });
        match (handle.ty, name.text.as_str()) {
            (Ty::Channel(&item), "send") => {
                let Some(value) = self.one_arg("send", args, span) else {
                    return Stmt::Expr(handle);
                };
                let value = self.expected(value, Some(item));
                self.expect(&value, item, "this argument");
                let list = self.boxed_one(value, item);
                let send = none(ExprKind::Foreign("lucid_send", vec![handle, list]));
                Stmt::Block(vec![send, self.failure_check(span)])
            }
            (Ty::Channel(_), "close") => {
                if !args.is_empty() {
                    self.poison_args(args, span);
                    self.fail("E0307", "`close` takes no arguments".into(), span);
                }
                let close = none(ExprKind::Foreign("lucid_close", vec![handle]));
                Stmt::Block(vec![close, self.failure_check(span)])
            }
            (Ty::Channel(_), "receive") => {
                let (stmt, result) = self.receive(handle, args, span);
                self.with_held(vec![stmt], result)
            }
            (Ty::Lock(item), "update") => self.update(handle, item, args, span),
            _ => unreachable!("`is_builtin` found a channel or a lock call"),
        }
    }

    /// `channel.receive()`: the next value, or `none` once the channel is
    /// closed and empty. The runtime gives a list of one item, or an empty
    /// one. Gives a statement and the local that holds the value.
    fn receive(&mut self, channel: Expr, args: &'a [ast::Arg], span: Span) -> (Stmt, Local) {
        let Ty::Channel(&item) = channel.ty else {
            let ty = channel.ty.name();
            self.fail("E0303", format!("`receive` does not work on `{ty}`"), span);
            return (Stmt::Block(Vec::new()), self.new_local(Ty::Error, false));
        };
        if !args.is_empty() {
            self.poison_args(args, span);
            self.fail("E0307", "`receive` takes no arguments".into(), span);
        }
        let ty = self.union([item, Ty::None]);
        let result = self.new_local(ty, false);
        let list = Ty::List(inner(item));
        let got = self.new_local(list, false);
        let int = |value| Expr { kind: ExprKind::Int(value), ty: Ty::Int, span };
        let read = Box::new(self.read(got, span));
        let len = Expr { kind: ExprKind::Len(read.clone()), ty: Ty::Int, span };
        let any = self.binary_values(BinaryOp::Gt, len, int(0), span);
        let value = Expr { kind: ExprKind::ListGet(read, int(0).into()), ty: item, span };
        let found = vec![Stmt::Set(result, self.widen(value, ty))];
        let none = Expr { kind: ExprKind::None, ty: Ty::None, span };
        let otherwise = vec![Stmt::Set(result, self.widen(none, ty))];
        let receive =
            Expr { kind: ExprKind::Foreign("lucid_receive", vec![channel]), ty: list, span };
        let stmts = vec![
            Stmt::Set(got, receive),
            self.failure_check(span),
            Stmt::If { branches: vec![(any, found)], otherwise },
            Stmt::Clear(got),
        ];
        (Stmt::Block(stmts), result)
    }

    /// `lock.update(f)`: takes the value out of the lock, so that it has one
    /// count, passes it to `f`, which owns it by decision 183, and keeps what
    /// `f` gives. So a write in `f` is in place, as decision 173 needs. The
    /// lock and `f` run first, in the order on the page.
    fn update(&mut self, lock: Expr, item: &'static Ty, args: &'a [ast::Arg], span: Span) -> Stmt {
        let Some(f) = self.one_arg("update", args, span) else { return Stmt::Expr(lock) };
        let task = Ty::Task(std::slice::from_ref(item), item);
        let f = self.expected(f, Some(task));
        self.expect(&f, task, "this argument");
        let list = Ty::List(item);
        let (held, f_local, value, new) = (
            self.new_local(lock.ty, false),
            self.new_local(task, false),
            self.new_local(*item, false),
            self.new_local(*item, false),
        );
        let local = |local, ty| Expr { kind: ExprKind::Local(local), ty, span };
        let taken = self.new_local(list, false);
        let take = ExprKind::Foreign("lucid_lock_take", vec![local(held, lock.ty)]);
        let first = Expr { kind: ExprKind::Int(0), ty: Ty::Int, span };
        let get = ExprKind::ListGet(local(taken, list).into(), first.into());
        let call = ExprKind::CallValue(local(f_local, task).into(), vec![local(value, *item)]);
        let kept = self.boxed_one(local(new, *item), *item);
        let put = ExprKind::Foreign("lucid_lock_put", vec![local(held, lock.ty), kept]);
        Stmt::Block(vec![
            Stmt::Set(held, lock),
            Stmt::Set(f_local, f),
            Stmt::Set(taken, Expr { kind: take, ty: list, span }),
            self.failure_check(span),
            Stmt::Set(value, Expr { kind: get, ty: *item, span }),
            Stmt::Clear(taken),
            Stmt::Set(new, Expr { kind: call, ty: *item, span }),
            Stmt::Expr(Expr { kind: put, ty: Ty::None, span }),
            Stmt::Clear(value),
            Stmt::Clear(new),
            Stmt::Clear(f_local),
            Stmt::Clear(held),
        ])
    }

    /// Lowers arguments of a call that already failed, to report their errors.
    fn poison_args(&mut self, args: &'a [ast::Arg], span: Span) -> Expr {
        for arg in args {
            self.expected(&arg.value, Some(Ty::Error));
        }
        poisoned(span)
    }
}
