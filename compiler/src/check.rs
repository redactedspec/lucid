//! Binds names, checks types, and lowers the syntax tree to MIR in one pass.
//! A generic body is checked once, with its parameters as opaque types, and
//! `mono.rs` makes the concrete tasks. There are no modules yet, so nothing
//! else needs a separate stage. List, map, and set tasks are built in.

use crate::ast::{
    self, BinaryOp, ExprKind as Ast, StmtKind, TextPart, TypeBody, TypeKind, UnaryOp,
};
use crate::diag::{Diagnostic, Span};
use crate::mir::{
    Expr, ExprKind, Local, Program, Shape, ShapeVariant, Stmt, Task, Ty, Types, Update, inner,
};
use std::collections::{HashMap, HashSet};

/// Names of the spec's built-in types that this milestone cannot represent.
const LATER_TYPES: [&str; 10] =
    ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "pointer"];

/// Type names that a program cannot define for itself.
const BUILT_IN_TYPES: [&str; 8] = ["int", "float", "bool", "byte", "text", "list", "map", "set"];

/// The same limit as nesting in the parser, for a chain of aliases.
const MAX_ALIASES: usize = 128;

/// How many instances of generic types the E0315 check follows, and how many
/// questions one trait check asks. For E0315 a type past this limit counts as
/// finite, and `mono.rs` stops any that really grows.
const MAX_FINITE_CHECK: usize = 256;

/// Traits that the compiler knows, and that a bound can name.
const BUILT_IN_TRAITS: [&str; 8] = ["Equal", "Order", "Show", "Hash", "Add", "Sub", "Mul", "Div"];

/// Traits in the spec that are not built yet.
const LATER_TRAITS: [&str; 3] = ["Copy", "Json", "Error"];

/// Dot calls that change their receiver, so they only work as statements, or
/// for `pop`, as the whole value of one.
const CHANGING_CALLS: [&str; 5] = ["push", "swap", "add", "remove", "pop"];

const JOIN_TEXT: &str = "join text with interpolation: \"{a}{b}\"";
const TEST_UNION: &str = "test it first, as in `if x is not none`. Only a name narrows, so name a field or call with `let`";

/// Checks a file. A program using an unbuilt feature gets only E0901, because
/// names and types that feature would supply make any other error unreliable.
pub fn check(file: &ast::File) -> (Program, Vec<Diagnostic>) {
    let mut checker = Checker::default();
    checker.declare_types(file);
    checker.declare(file);
    let tasks = file
        .stmts
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Task(def) => def.body.as_ref().map(|body| (def, body, stmt.span)),
            _ => None,
        })
        .enumerate()
        .map(|(id, (def, body, span))| checker.task(id, def, body, span))
        .collect();
    let main = checker.main(file);

    let datas = checker.datas.iter().map(Data::shape).collect();
    let methods = checker.methods.iter().map(|(&name, ids)| (name.into(), ids.clone())).collect();
    let mut diags = checker.diags;
    if diags.iter().any(|d| d.code == "E0901") {
        diags.retain(|d| d.code == "E0901");
    }
    (Program { tasks, main, types: checker.table, datas, methods }, diags)
}

/// Type parameters in scope, with the type that each one stands for.
type TypeEnv<'a> = Vec<(&'a str, Ty)>;

/// The traits that each type parameter in scope has, by position.
type Bounds<'a> = Vec<Vec<&'a str>>;

struct Signature<'a> {
    name: &'a ast::Name,
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
        };
        Shape { variants: self.variants.iter().map(shape).collect() }
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
    diags: Vec<Diagnostic>,
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
    /// The aliases being resolved, innermost last.
    resolving: Vec<&'a str>,
    top: HashMap<&'a str, Top<'a>>,
    scopes: Vec<HashMap<&'a str, Local>>,
    locals: Vec<Ty>,
    mutable: Vec<bool>,
    /// The return type, or `None` at top level.
    returns: Option<Ty>,
    loops: usize,
    table: Types,
    /// What tests have proved about locals, latest last. See `forget`.
    narrowed: Vec<(Local, Ty)>,
}

/// The parameters that a declaration brings into scope, by position.
fn own_params(params: &[ast::TypeParam]) -> TypeEnv<'_> {
    (params.iter().enumerate())
        .map(|(index, p)| (p.name.text.as_str(), Ty::Param(p.name.text.clone().leak(), index)))
        .collect()
}

/// True for a task that the language builds in for this type.
fn builtin_method(ty: Ty, name: &str) -> bool {
    match ty {
        Ty::Int => name == "to_float",
        Ty::Float | Ty::Byte => name == "to_int",
        Ty::Text => matches!(name, "len" | "is_empty" | "bytes"),
        Ty::List(_) => matches!(name, "len" | "is_empty" | "get" | "push" | "pop" | "swap"),
        Ty::Map(..) => matches!(name, "len" | "is_empty" | "get" | "remove"),
        Ty::Set(_) => matches!(name, "len" | "is_empty" | "add" | "remove"),
        _ => false,
    }
}

fn poisoned(span: Span) -> Expr {
    Expr { kind: ExprKind::None, ty: Ty::Error, span }
}

fn is_literal(expr: &ast::Expr) -> bool {
    match &expr.kind {
        Ast::Int(_) | Ast::Float(_) | Ast::Bool(_) | Ast::None => true,
        Ast::Text(parts) => parts.iter().all(|part| matches!(part, TextPart::Literal(_))),
        _ => false,
    }
}

/// A place to write through, such as `x[i]` in `x[i][j] = 1`. `local` holds the
/// value, and `setup` fills it. `take` empties its slot in the container, so
/// that the local holds the only count and the write needs no copy. `finish`
/// stores it back. Nothing may read the container between `take` and `finish`.
struct Place {
    local: Local,
    setup: Vec<Stmt>,
    take: Vec<Stmt>,
    finish: Vec<Stmt>,
}

impl Place {
    /// The whole write. `write` holds the value that the write changes.
    fn stmts(mut self, write: Stmt) -> Stmt {
        self.setup.append(&mut self.take);
        self.setup.push(write);
        self.setup.append(&mut self.finish);
        Stmt::Block(self.setup)
    }
}

/// True if control never reaches the end of `stmts`. With `jumps`, leaving
/// through `break` or `continue` counts.
fn ends(stmts: &[Stmt], jumps: bool) -> bool {
    stmts.iter().any(|stmt| match stmt {
        Stmt::Return(_) | Stmt::Placeholder(_) => true,
        Stmt::Break | Stmt::Continue => jumps,
        Stmt::Block(stmts) => ends(stmts, jumps),
        Stmt::If { branches, otherwise } => {
            branches.iter().all(|(_, then)| ends(then, jumps)) && ends(otherwise, jumps)
        }
        _ => false,
    })
}

/// A union value as a narrower type.
fn view(value: Expr, ty: Ty) -> Expr {
    if ty == value.ty || ty == Ty::Error {
        return Expr { ty, ..value };
    }
    let span = value.span;
    Expr { kind: ExprKind::View(value.into()), ty, span }
}

/// True if reading `value` twice costs nothing and changes nothing.
fn repeatable(value: &Expr) -> bool {
    match &value.kind {
        ExprKind::Local(_) | ExprKind::Int(_) | ExprKind::Bool(_) => true,
        ExprKind::Member { value: inner, .. } | ExprKind::View(inner) => repeatable(inner),
        ExprKind::ListGet(base, key) | ExprKind::MapGet(base, key) => {
            repeatable(base) && repeatable(key)
        }
        _ => false,
    }
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
        let boxes = members.iter().find(|&&member| self.fits_any(value.ty, member));
        match boxes {
            Some(&member) => wrap(wrap(value, member), want),
            None => value,
        }
    }

    /// True if a value of type `ty` can go in `target`, which must be an `any`.
    fn fits_any(&mut self, ty: Ty, target: Ty) -> bool {
        let Ty::Any(name) = target else { return false };
        ty != target && ty.members().iter().all(|&member| self.satisfies(member, name))
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
        let found = self.narrowed.iter().rev().find(|(narrowed, _)| *narrowed == local);
        found.map_or(self.locals[local], |&(_, ty)| ty)
    }

    /// Ends what tests proved about a local that was assigned. Entries stay on
    /// the stack, so that a block can still cut it back to its own length. So
    /// an assignment on any path ends the narrowing on all that follow.
    fn forget(&mut self, local: Local) {
        let declared = self.locals[local];
        for entry in self.narrowed.iter_mut().filter(|(narrowed, _)| *narrowed == local) {
            entry.1 = declared;
        }
    }

    /// Runs before a loop: its body runs again after its own assignments.
    ///
    /// When `change` parameters arrive, passing a local must also `forget` it.
    fn forget_assigned(&mut self, stmts: &'a [ast::Stmt]) {
        for stmt in stmts {
            match &stmt.kind {
                StmtKind::Assign { target: ast::Expr { kind: Ast::Name(name), .. }, .. } => {
                    if let Some(local) = self.lookup(name) {
                        self.forget(local);
                    }
                }
                StmtKind::If { branches, otherwise } => {
                    for (_, then) in branches {
                        self.forget_assigned(then);
                    }
                    self.forget_assigned(otherwise.as_deref().unwrap_or_default());
                }
                StmtKind::While { body, .. } | StmtKind::For { body, .. } => {
                    self.forget_assigned(body);
                }
                StmtKind::Expr(ast::Expr { kind: Ast::Match { arms, .. }, .. }) => {
                    for arm in arms {
                        if let ast::ArmBody::Block(body) = &arm.body {
                            self.forget_assigned(body);
                        }
                    }
                }
                _ => {}
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
                let start = self.narrowed.len();
                self.narrowed.extend(facts.iter().copied());
                let more = self.facts(right, truth);
                self.narrowed.truncate(start);
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
            (Ty::Float, Ty::Int) => diag.note("conversions are explicit: use `.to_float()`"),
            (Ty::Int, Ty::Float) => diag.note("conversions are explicit: use `.to_int()`"),
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
                _ => self.unsupported("this type", ty.span).ty,
            },
            TypeKind::Any(held) => self.any_type(held, ty.span),
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
        if matches!(name, "list" | "map" | "set") {
            return self.collection_type(name, &args, span);
        }
        let plain = match name {
            "int" => Some(Ty::Int),
            "float" => Some(Ty::Float),
            "bool" => Some(Ty::Bool),
            "byte" => Some(Ty::Byte),
            "text" => Some(Ty::Text),
            _ => None,
        };
        if let Some(plain) = plain {
            return if self.arity(name, 0, args.len(), span) { plain } else { Ty::Error };
        }
        if LATER_TYPES.contains(&name) {
            return self.unsupported(&format!("type `{name}`"), span).ty;
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
        let reserved = BUILT_IN_TYPES.contains(&text)
            || LATER_TYPES.contains(&text)
            || BUILT_IN_TRAITS.contains(&text)
            || LATER_TRAITS.contains(&text)
            || self.traits.contains_key(text);
        if reserved {
            self.fail("E0304", format!("`{text}` is already defined"), name.span);
            return false;
        }
        self.define(name)
    }

    fn data_header(&mut self, public: bool, span: Span) {
        if public {
            self.unsupported("`public`", span);
        }
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
        if let Ty::Param(_, index) = ty {
            return self.bounds.get(index).is_some_and(|traits| traits.contains(&name));
        }
        if let Ty::Any(held) = ty
            && (held == name || matches!(name, "Show" | "Equal"))
        {
            return true;
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
        let parts = if name == "Hash" {
            // A `none` inside a value hashes as nothing. Alone it is no key.
            parts.into_iter().filter(|&part| part != Ty::None).collect()
        } else {
            parts
        };
        let holds = match name {
            "Show" | "Equal" => true,
            "Hash" => ty.is_key() || (ty.is_compound() && !matches!(ty, Ty::Any(_))),
            "Order" => {
                matches!(ty, Ty::Int | Ty::Float | Ty::Byte | Ty::Text | Ty::List(_))
                    || matches!(ty, Ty::Data(..))
            }
            "Div" => return ty == Ty::Float,
            _ => return matches!(ty, Ty::Int | Ty::Float),
        };
        if !holds {
            return false;
        }
        parts.into_iter().all(|part| self.satisfied(part, name, seen))
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
    /// gives no `Hash`, since a derived hash would not match it. See decision 77.
    fn written_hash(&mut self, ty: Ty, seen: &mut Vec<(Ty, String)>) -> Option<bool> {
        let Some(found) = self.written_task("hash", ty, seen) else {
            return self.find_method("equal", ty).map(|_| false);
        };
        let (id, known) = self.find_method("hash", ty)?;
        let args: Option<Vec<Ty>> = known.into_iter().collect();
        let (Some(args), true) = (args, found) else { return Some(false) };
        let result = self.table.subst(self.signatures[id].returns, &args);
        Some(self.satisfied(result, "Hash", seen))
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
        wanted.len() == have.len() && wanted[1..] == have[1..] && want_returns == have_returns
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
                self.declare_trait(def, stmt.span);
            }
        }
        for stmt in &file.stmts {
            match &stmt.kind {
                StmtKind::Type(def) => {
                    self.data_header(def.public, stmt.span);
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
                            self.datas[id].variants.push(Variant { name, fields: Vec::new() });
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
                    self.data_header(def.public, stmt.span);
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
                        }
                        self.datas[id].variants.push(Variant { name, fields: Vec::new() });
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
    fn declare_trait(&mut self, def: &'a ast::TraitDef, span: Span) {
        if def.public {
            self.unsupported("`public`", span);
        }
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
            if def.body.is_none() {
                self.unsupported("`unsafe extern task`", stmt.span);
                continue;
            }
            if def.public {
                self.unsupported("`public`", stmt.span);
            }
            let name = def.sig.name.text.as_str();
            let id = self.signatures.len();
            if def.sig.params.first().is_some_and(|param| param.name.text == "self") {
                self.methods.entry(name).or_default().push(id);
            } else if self.define(&def.sig.name) {
                self.task_ids.insert(name, id);
            }
            let signature = self.signature(&def.sig, false);
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
            if param.mode != ast::ParamMode::Read {
                self.unsupported("a `change` or `take` parameter", span);
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
            params.push(Param { name: &param.name.text, ty, default });
        }
        let returns = sig.returns.as_ref().map_or(Ty::None, |ty| self.ty(ty));
        self.type_params.clear();
        self.bounds.clear();
        Signature { name: &sig.name, type_params, bounds, in_trait, params, returns }
    }

    fn default_value(&mut self, value: &'a ast::Expr, ty: Ty) -> Expr {
        if !is_literal(value) {
            return self.fail("E0313", "a default must be a literal".into(), value.span);
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
        self.returns = Some(returns);
        self.type_params.clone_from(&self.signatures[id].type_params);
        self.bounds.clone_from(&self.signatures[id].bounds);
        let params: Vec<(&'a str, Ty)> =
            self.signatures[id].params.iter().map(|p| (p.name, p.ty)).collect();
        let mut scope = HashMap::new();
        for &(name, ty) in &params {
            scope.insert(name, self.new_local(ty, false));
        }
        self.scopes.push(scope);
        let body = self.stmts(body);
        self.scopes.pop();
        if returns != Ty::None && returns != Ty::Error && !ends(&body, false) {
            let name = &def.sig.name;
            let message = format!("`{}` must return `{}` on every path", name.text, returns.name());
            self.fail("E0309", message, name.span);
        }
        self.mutable.clear();
        let type_params = self.type_params.len();
        self.type_params.clear();
        self.bounds.clear();
        let locals = std::mem::take(&mut self.locals);
        Task { params: params.len(), type_params, returns, locals, body, span }
    }

    fn main(&mut self, file: &'a ast::File) -> Task {
        self.returns = None;
        self.scopes.push(HashMap::new());
        let body = self.stmts(&file.stmts);
        let span = Span { start: 0, end: 0 };
        let locals = std::mem::take(&mut self.locals);
        Task { params: 0, type_params: 0, returns: Ty::None, locals, body, span }
    }

    fn new_local(&mut self, ty: Ty, mutable: bool) -> Local {
        self.locals.push(ty);
        self.mutable.push(mutable);
        self.locals.len() - 1
    }

    /// Adds a name to the innermost scope. Shadowing there is an error.
    fn bind(&mut self, name: &'a ast::Name, ty: Ty, mutable: bool) -> Local {
        let text = name.text.as_str();
        let top_level = self.returns.is_none() && self.scopes.len() == 1;
        let scope = self.scopes.last().expect("a scope is open");
        let global = self.task_ids.contains_key(text) || self.constructors.contains_key(text);
        if scope.contains_key(text) || (top_level && global) {
            self.fail("E0304", format!("`{text}` is already defined"), name.span);
        }
        let local = self.new_local(ty, mutable);
        self.scopes.last_mut().expect("a scope is open").insert(text, local);
        local
    }

    fn lookup(&self, name: &str) -> Option<Local> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name).copied())
    }

    fn block(&mut self, stmts: &'a [ast::Stmt]) -> Vec<Stmt> {
        self.scopes.push(HashMap::new());
        let stmts = self.stmts(stmts);
        self.scopes.pop();
        stmts
    }

    fn stmts(&mut self, stmts: &'a [ast::Stmt]) -> Vec<Stmt> {
        let start = self.narrowed.len();
        let stmts = stmts.iter().filter_map(|stmt| self.stmt(stmt)).collect();
        self.narrowed.truncate(start);
        stmts
    }

    fn condition(&mut self, value: &'a ast::Expr, what: &str) -> Expr {
        let value = self.expr(value);
        self.expect(&value, Ty::Bool, what);
        value
    }

    fn stmt(&mut self, stmt: &'a ast::Stmt) -> Option<Stmt> {
        let span = stmt.span;
        Some(match &stmt.kind {
            StmtKind::Task(_) => {
                if self.returns.is_some() || self.scopes.len() > 1 {
                    self.unsupported("a nested task", span);
                }
                return None;
            }
            StmtKind::Let { mutable, name, ty, value } => {
                let want = ty.as_ref().map(|ty| self.ty(ty));
                if let Some((held, result)) = self.statement_value(value, want) {
                    let ty = want.unwrap_or(self.locals[result]);
                    let read = self.held_value(result, ty, "this annotation", value.span);
                    let set = Stmt::Set(self.bind(name, ty, *mutable), read);
                    return Some(self.with_held(vec![held, set], result));
                }
                let value = self.expected(value, want);
                let ty = want.unwrap_or(value.ty);
                self.expect(&value, ty, "this annotation");
                Stmt::Set(self.bind(name, ty, *mutable), value)
            }
            StmtKind::Assign { target, op, value } => return self.assign(target, *op, value, span),
            StmtKind::If { branches, otherwise } => self.if_stmt(branches, otherwise.as_deref()),
            StmtKind::While { condition, body } => {
                self.forget_assigned(body);
                let start = self.narrowed.len();
                let test = self.condition(condition, "`while`");
                let facts = self.facts(condition, true);
                self.narrowed.extend(facts);
                self.loops += 1;
                let body = self.block(body);
                self.loops -= 1;
                self.narrowed.truncate(start);
                Stmt::While { condition: test, body }
            }
            StmtKind::For { change, name, iterable, body } => {
                self.for_loop(*change, name, iterable, body)
            }
            StmtKind::Return(value) => {
                let none = Expr { kind: ExprKind::None, ty: Ty::None, span };
                let mut matched = None;
                let value = match value {
                    Some(value) => match self.statement_value(value, self.returns) {
                        Some((stmt, result)) => {
                            matched = Some(stmt);
                            let read = self.read(result, value.span);
                            match self.returns {
                                Some(want) => self.widen(read, want),
                                None => read,
                            }
                        }
                        None => self.expected(value, self.returns),
                    },
                    None => none,
                };
                match self.returns {
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
            StmtKind::Break | StmtKind::Continue if self.loops == 0 => {
                self.fail("E0311", "`break` and `continue` need a loop".into(), span);
                return None;
            }
            StmtKind::Break => Stmt::Break,
            StmtKind::Continue => Stmt::Continue,
            StmtKind::Placeholder => Stmt::Placeholder(span),
            StmtKind::Expr(value) => return self.expr_stmt(value, span),
            StmtKind::Use { .. } => return self.later("`use`", span),
            StmtKind::Test { .. } => return self.later("`test`", span),
            StmtKind::Type(_) | StmtKind::Choice(_) | StmtKind::Trait(_) => return None,
            StmtKind::Expect(_) => return self.later("`expect`", span),
            StmtKind::Unsafe(_) => return self.later("`unsafe`", span),
        })
    }

    /// Later branches, and any code after an `if` whose branches all leave,
    /// know that the earlier conditions were false.
    fn if_stmt(
        &mut self,
        branches: &'a [(ast::Expr, Vec<ast::Stmt>)],
        otherwise: Option<&'a [ast::Stmt]>,
    ) -> Stmt {
        let start = self.narrowed.len();
        let mut lowered = Vec::with_capacity(branches.len());
        for (condition, then) in branches {
            let test = self.condition(condition, "`if`");
            let (found, rest) = (self.facts(condition, true), self.facts(condition, false));
            let outer = self.narrowed.len();
            self.narrowed.extend(found);
            let then = self.block(then);
            self.narrowed.truncate(outer);
            self.narrowed.extend(rest);
            lowered.push((test, then));
        }
        let otherwise = otherwise.map_or(Vec::new(), |body| self.block(body));
        if !lowered.iter().all(|(_, then)| ends(then, true)) {
            self.narrowed.truncate(start);
        }
        Stmt::If { branches: lowered, otherwise }
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
        stmts.extend(self.locals[result].is_counted().then_some(Stmt::Clear(result)));
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
                _ => Stmt::Expr(self.expr(value)),
            },
            _ => Stmt::Expr(self.expr(value)),
        })
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
            self.scopes.push(HashMap::new());
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
            self.scopes.pop();
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
        self.locals[result] = want.unwrap_or(Ty::None);

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
        if self.constructors.contains_key(name.text.as_str()) {
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
        let value = self.expr(value);
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
                        diag.note("match what the union holds, as in `s: Shape`")
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
        let name = match &target.kind {
            Ast::Name(name) => name,
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
        let ty = self.locals[local];
        let value = match compound(op) {
            Some(op) => {
                let value = self.binary(op, target, value, span);
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
        if self.locals[local] == Ty::Error {
            return None;
        }
        if !self.mutable[local] {
            let diag = Diagnostic::new("E0305", format!("`{name}` cannot change"), span);
            self.report(diag.note("only a name declared with `var` can change"));
        }
        Some(local)
    }

    fn read(&self, local: Local, span: Span) -> Expr {
        Expr { kind: ExprKind::Local(local), ty: self.locals[local], span }
    }

    /// Finds what a write changes. Each index runs once, before the value
    /// that is written, so that the order is the order on the page.
    fn place(&mut self, target: &'a ast::Expr) -> Option<Place> {
        match &target.kind {
            Ast::Name(name) => {
                let local = self.variable(name, target.span)?;
                let place =
                    Place { local, setup: Vec::new(), take: Vec::new(), finish: Vec::new() };
                let (ty, span) = (self.current(local), target.span);
                if ty == self.locals[local] || matches!(ty, Ty::Union(_)) {
                    return Some(place);
                }
                // A narrowed union: what it holds is a field of its record.
                let read = view(self.read(local, span), ty);
                let key = Expr { kind: ExprKind::Int(0), ty: Ty::Int, span };
                Some(self.step(place, key, read))
            }
            Ast::Index { base, indices } => {
                let mut place = self.place(base)?;
                let (key, item) = self.key(self.locals[place.local], indices, target.span)?;
                let key = self.hoist(key, &mut place);
                let read = self.element(place.local, key, item, target.span);
                let key = self.read(key, target.span);
                Some(self.step(place, key, read))
            }
            Ast::Field { base, name } => {
                let mut place = self.place(base)?;
                let (slot, ty) = self.field(self.locals[place.local], name)?;
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
        let value = self.new_local(item, true);
        place.setup.push(Stmt::Set(value, read));
        let empty = Expr { kind: ExprKind::None, ty: Ty::None, span };
        place.take.push(self.write(place.local, key.clone(), empty, span));
        let stored = self.read(value, span);
        let write = self.write(place.local, key, stored, span);
        place.finish.insert(0, write);
        place.finish.push(Stmt::Clear(value));
        place.local = value;
        place
    }

    /// Moves a value to a hidden local, so that it can be read twice or read
    /// before `take`. The local lets go of it when the write ends.
    fn hoist(&mut self, value: Expr, place: &mut Place) -> Local {
        let local = self.new_local(value.ty, false);
        if value.ty.is_counted() {
            place.finish.push(Stmt::Clear(local));
        }
        place.setup.push(Stmt::Set(local, value));
        local
    }

    /// Reads an element from the container in `local`.
    fn element(&self, local: Local, key: Local, ty: Ty, span: Span) -> Expr {
        let (container, key) = (self.read(local, span).into(), self.read(key, span).into());
        let kind = match self.locals[local] {
            Ty::List(_) => ExprKind::ListGet(container, key),
            _ => ExprKind::MapGet(container, key),
        };
        Expr { kind, ty, span }
    }

    fn write(&self, local: Local, key: Expr, value: Expr, span: Span) -> Stmt {
        let op = match self.locals[local] {
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
        let Some((key, item)) = self.key(self.locals[local], indices, span) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let key = self.hoist(key, &mut place);
        let value = match compound(op) {
            Some(op) => {
                let old = self.element(local, key, item, span);
                let value = self.expr(value);
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
        let Some((slot, ty)) = self.field(self.locals[local], name) else {
            self.expected(value, Some(Ty::Error));
            return None;
        };
        let value = match compound(op) {
            Some(op) => {
                let old = self.slot_of(local, slot, ty, span);
                let value = self.expr(value);
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
        let free = self.lookup(name).is_none() && !self.top.contains_key(name);
        self.constructors.get(name).copied().filter(|_| free)
    }

    fn task_named(&self, name: &str) -> Option<usize> {
        let free = self.lookup(name).is_none() && !self.top.contains_key(name);
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
        let value = self.hoist(value, place);
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
        let mut place = self.place(base)?;
        let local = place.local;
        let ty = self.locals[local];
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
        let Some(place) = self.place(base) else {
            return (nothing, self.new_local(Ty::Error, false));
        };
        let local = place.local;
        let Ty::List(item) = self.locals[local] else {
            let ty = self.locals[local].name();
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
        body: &'a [ast::Stmt],
    ) -> (Local, Vec<Stmt>) {
        self.forget_assigned(body);
        self.scopes.push(HashMap::new());
        let local = self.bind(name, ty, false);
        self.loops += 1;
        let body = self.stmts(body);
        self.loops -= 1;
        self.scopes.pop();
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
            let value = self.expr(iterable);
            let span = value.span;
            let value = match (change, value.ty) {
                (_, Ty::Error) => value,
                (true, Ty::Text) => self.fail("E0303", "text cannot change".into(), span),
                (true, Ty::List(_)) => self.unsupported("`for change` on a list", span),
                (true, Ty::Map(..) | Ty::Set(_)) => {
                    self.fail("E0303", "a key cannot change".into(), span)
                }
                (false, Ty::Text | Ty::List(_) | Ty::Map(..) | Ty::Set(_)) => value,
                (_, ty) => {
                    self.fail("E0303", format!("`for` cannot loop over `{}`", ty.name()), span)
                }
            };
            return self.for_each(value, name, body);
        };
        let (start, end) = self.range(start, end);
        if change {
            self.fail("E0303", "a range has nothing to change".into(), iterable.span);
        }
        let (local, body) = self.loop_body(name, Ty::Int, body);
        Stmt::For { local, start, end, inclusive: *op == BinaryOp::RangeInclusive, body }
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
        let (current, body) = self.loop_body(name, item, body);

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
            Stmt::Clear(owner),
        ])
    }

    fn name(&mut self, name: &str, span: Span) -> Expr {
        if let Some(local) = self.lookup(name) {
            return view(self.read(local, span), self.current(local));
        }
        if self.returns.is_some() {
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
        if self.task_ids.contains_key(name) {
            return self.unsupported("a task used as a value", span);
        }
        if let Some(&(id, variant)) = self.constructors.get(name) {
            if self.datas[id].variants[variant].fields.is_empty() {
                return self.construct((id, variant), None, None, &[], span);
            }
            let diag = Diagnostic::new("E0307", format!("`{name}` needs its fields"), span);
            return self.report(diag.note(format!("write `{name}(field: value)`")));
        }
        self.fail("E0301", format!("name `{name}` is not defined"), span)
    }

    fn int(&mut self, lexeme: &str, span: Span) -> Expr {
        let negative = lexeme.starts_with('-');
        let digits = lexeme.trim_start_matches('-').replace('_', "");
        let parsed = match digits.get(..2) {
            Some("0x") => u64::from_str_radix(&digits[2..], 16),
            Some("0b") => u64::from_str_radix(&digits[2..], 2),
            _ => digits.parse(),
        };
        let limit = i64::MAX as u64 + u64::from(negative);
        match parsed {
            Ok(value) if value <= limit => {
                let value = if negative { value.wrapping_neg() } else { value } as i64;
                Expr { kind: ExprKind::Int(value), ty: Ty::Int, span }
            }
            _ => self.fail("E0312", format!("`{lexeme}` does not fit in `int`"), span),
        }
    }

    fn float(&mut self, lexeme: &str, span: Span) -> Expr {
        let digits = lexeme.replace('_', "");
        let mantissa = digits.split(['e', 'E']).next().unwrap_or_default();
        let nonzero = mantissa.contains(|c: char| matches!(c, '1'..='9'));
        match digits.parse::<f64>() {
            Ok(value) if value.is_finite() && (value != 0.0 || !nonzero) => {
                Expr { kind: ExprKind::Float(value), ty: Ty::Float, span }
            }
            _ => self.fail("E0312", format!("`{lexeme}` does not fit in `float`"), span),
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

    fn lower(&mut self, expr: &'a ast::Expr, want: Option<Ty>) -> Expr {
        let span = expr.span;
        match &expr.kind {
            Ast::List(items) => self.list(items, want, span),
            Ast::Map(entries) => self.map(entries, want, span),
            Ast::If { condition, then, otherwise } => {
                let test = self.condition(condition, "`if`");
                let start = self.narrowed.len();
                let (found, rest) = (self.facts(condition, true), self.facts(condition, false));
                self.narrowed.extend(found);
                let then = self.expected(then, want);
                self.narrowed.truncate(start);
                self.narrowed.extend(rest);
                let otherwise = self.expected(otherwise, want.or(Some(then.ty)));
                self.narrowed.truncate(start);
                self.expect(&otherwise, then.ty, "`else`, to match `then`,");
                let ty = then.ty;
                let kind = ExprKind::If(test.into(), then.into(), otherwise.into());
                Expr { kind, ty, span }
            }
            Ast::Call { callee, args } => self.call(callee, args, want, span),
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
        name == "set"
            && self.lookup(name).is_none()
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
            Ast::Int(lexeme) => return self.int(lexeme, span),
            Ast::Float(lexeme) => return self.float(lexeme, span),
            Ast::Bool(value) => (ExprKind::Bool(*value), Ty::Bool),
            Ast::None => (ExprKind::None, Ty::None),
            Ast::Name(name) => return self.name(name, span),
            Ast::Unary(op, value) => return self.unary(*op, value, span),
            Ast::Binary(op, left, right) => return self.binary(*op, left, right, span),
            Ast::If { .. } | Ast::List(_) | Ast::Map(_) => return self.expected(expr, None),
            Ast::Call { callee, args } => return self.call(callee, args, None, span),
            Ast::Text(parts) => return self.text(parts, span),
            Ast::Is { value, negated, target } => {
                return self.is_test(value, *negated, target, span);
            }
            Ast::Lambda { .. } => return self.unsupported("a lambda", span),
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
            _ => self.method_call(value, "is_empty", &[], span),
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

    fn unary(&mut self, op: UnaryOp, value: &'a ast::Expr, span: Span) -> Expr {
        match op {
            UnaryOp::Neg => {
                let value = self.expr(value);
                match value.ty {
                    Ty::Error => poisoned(span),
                    Ty::Int => {
                        let zero = Expr { kind: ExprKind::Int(0), ty: Ty::Int, span };
                        let kind = ExprKind::Binary(BinaryOp::Sub, zero.into(), value.into());
                        Expr { kind, ty: Ty::Int, span }
                    }
                    Ty::Float => Expr { kind: ExprKind::Neg(value.into()), ty: Ty::Float, span },
                    Ty::Byte => self.unsupported("arithmetic on `byte`", span),
                    ty => {
                        let message = format!("`-` needs a number, found `{}`", ty.name());
                        self.fail("E0303", message, value.span)
                    }
                }
            }
            UnaryOp::Not => {
                let value = self.condition(value, "`not`");
                Expr { kind: ExprKind::Not(value.into()), ty: Ty::Bool, span }
            }
            UnaryOp::Try => self.unsupported("`try`", span),
            UnaryOp::Spawn => self.unsupported("`spawn`", span),
            UnaryOp::Shared | UnaryOp::Weak => self.unsupported("`shared` and `weak`", span),
        }
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        left: &'a ast::Expr,
        right: &'a ast::Expr,
        span: Span,
    ) -> Expr {
        match op {
            BinaryOp::Fallback => return self.unsupported("`else` on a value", span),
            BinaryOp::In | BinaryOp::NotIn => return self.contains(op, left, right, span),
            BinaryOp::Range | BinaryOp::RangeInclusive => {
                return self.unsupported("a range outside `for`", span);
            }
            BinaryOp::And | BinaryOp::Or => {
                let first = self.expr(left);
                let start = self.narrowed.len();
                let facts = self.facts(left, op == BinaryOp::And);
                self.narrowed.extend(facts);
                let right = self.expr(right);
                self.narrowed.truncate(start);
                return self.binary_values(op, first, right, span);
            }
            _ => {}
        }
        let left = self.expr(left);
        let right = self.expr(right);
        self.binary_values(op, left, right, span)
    }

    fn contains(
        &mut self,
        op: BinaryOp,
        left: &'a ast::Expr,
        right: &'a ast::Expr,
        span: Span,
    ) -> Expr {
        let left = self.expr(left);
        let right = self.expr(right);
        let (item, list) = match right.ty {
            Ty::Error => return poisoned(span),
            Ty::List(item) => (*item, true),
            Ty::Set(item) | Ty::Map(item, _) => (*item, false),
            ty => {
                let message = format!("`in` needs a list, map, or set, found `{}`", ty.name());
                return self.fail("E0303", message, right.span);
            }
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
            let note = if left.ty == Ty::None
                || right.ty == Ty::None
                || matches!(left.ty, Ty::Union(_))
                || matches!(right.ty, Ty::Union(_))
            {
                "test a union with `is`, as in `x is none`"
            } else if left.ty == Ty::Text || right.ty == Ty::Text {
                JOIN_TEXT
            } else {
                "conversions are explicit: use `.to_float()` or `.to_int()`"
            };
            return self.report(Diagnostic::new("E0303", message, span).note(note));
        }
        let ty = left.ty;
        let equality = matches!(op, BinaryOp::Eq | BinaryOp::Ne);
        let order = matches!(op, BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge);
        let int_only = matches!(op, BinaryOp::FloorDiv | BinaryOp::Rem);
        let valid = match ty {
            Ty::Int => true,
            Ty::Float => !int_only,
            Ty::Bool => equality,
            Ty::Text => equality || order,
            Ty::Byte if equality || order => true,
            Ty::Byte => return self.unsupported("arithmetic on `byte`", span),
            ty if (ty.is_compound() || matches!(ty, Ty::Param(..))) && (equality || order) => {
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
            BinaryOp::Div if matches!(ty, Ty::Int | Ty::Float) => Ty::Float,
            _ => ty,
        };
        let float = |value: Expr| {
            let span = value.span;
            Expr { kind: ExprKind::ToFloat(value.into()), ty: Ty::Float, span }
        };
        let (left, right) = if op == BinaryOp::Div && ty == Ty::Int {
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
                let value = self.expr(base);
                self.dot_call(value, name, args, span)
            }
            Ast::Name(name) if self.is_set_call(name) => self.set_call(args, want, span),
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
                self.unsupported("this call", span)
            }
            Ast::Name(name) => {
                if let Some(target) = self.constructor(name) {
                    return self.construct_call(target, name, None, want, args, span);
                }
                if let Some(id) = self.task_named(name) {
                    return self.task_call(id, name, None, None, args, span);
                }
                for arg in args {
                    self.expected(&arg.value, Some(Ty::Error));
                }
                if self.lookup(name).is_some() || self.top.contains_key(name.as_str()) {
                    return self.fail("E0308", format!("`{name}` is not a task"), callee.span);
                }
                let diag =
                    Diagnostic::new("E0301", format!("name `{name}` is not defined"), callee.span);
                let dotted = self.methods.contains_key(name.as_str());
                let note = format!("`{name}` has `self`, so call it with a dot: `x.{name}()`");
                self.report(if dotted { diag.note(note) } else { diag })
            }
            _ => self.unsupported("this call", span),
        }
    }

    /// Named arguments follow positional ones, in parameter order, so that the
    /// order of evaluation is always the order on the page.
    fn task_call(
        &mut self,
        id: usize,
        name: &str,
        explicit: Option<&[Ty]>,
        receiver: Option<(Expr, Vec<Option<Ty>>)>,
        args: &'a [ast::Arg],
        span: Span,
    ) -> Expr {
        let reported = self.diags.len();
        let (receiver, seeded) = receiver.unzip();
        let mut known: Vec<Option<Ty>> = match explicit {
            Some(types) => types.iter().copied().map(Some).collect(),
            None => seeded.unwrap_or(vec![None; self.signatures[id].type_params.len()]),
        };
        let count = self.signatures[id].params.len();
        let required = self.signatures[id].params.iter().filter(|p| p.default.is_none()).count();
        let mut values: Vec<Option<Expr>> = vec![None; count];
        let mut next = usize::from(receiver.is_some());
        values[..next].clone_from_slice(&[receiver][..next]);
        let mut named = false;
        for arg in args {
            if arg.change {
                self.unsupported("a `change` argument", arg.value.span);
            }
            let params = &self.signatures[id].params;
            let want = match &arg.name {
                None => params.get(next),
                Some(label) => params.iter().find(|p| p.name == label.text),
            };
            let pattern = want.map_or(Ty::Error, |p| p.ty);
            let value = self.inferred(&arg.value, pattern, &mut known);
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
                    next = slot + 1;
                }
                Err(message) => {
                    self.fail("E0307", message, arg.value.span);
                }
            }
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
        let mut lowered = Vec::with_capacity(count);
        for (slot, value) in values.into_iter().enumerate() {
            let param = &self.signatures[id].params[slot];
            let default = param.default.clone();
            let ty = param.ty;
            let ty = self.table.subst(ty, types);
            if let Some(value) = value {
                lowered.push(self.settled(value, ty, "this argument"));
            } else if let Some(value) = default {
                lowered.push(Expr { ty, ..value });
            }
        }
        let returns = self.signatures[id].returns;
        let ty = self.table.subst(returns, types);
        if self.signatures[id].in_trait {
            let name = self.signatures[id].name.text.clone().leak();
            return Expr { kind: ExprKind::Method(name, lowered), ty, span };
        }
        Expr { kind: ExprKind::Call(id, types, lowered), ty, span }
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

    /// `x.f(a)` for a task with a `self` parameter. On a type parameter or an
    /// `any` it looks in their traits, and `mono.rs` finds the real task.
    fn method_call(&mut self, value: Expr, name: &str, args: &'a [ast::Arg], span: Span) -> Expr {
        let ty = value.ty;
        if let Some(id) = self.trait_task(ty, name) {
            return self.task_call(id, name, None, Some((value, Vec::new())), args, span);
        }
        if let Some((id, known)) = self.find_method(name, ty) {
            return self.task_call(id, name, None, Some((value, known)), args, span);
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

    /// A dot call: a built-in task on a built-in type, or else a method.
    fn dot_call(
        &mut self,
        value: Expr,
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
            return self.method_call(value, text, args, span);
        }
        if text == "pop" {
            let message = "`pop` changes its list, so it must be a whole statement, or all of \
                           the value of `let`, `var`, `return`, or an assignment to a name";
            return self.fail("E0303", message.into(), span);
        }
        if CHANGING_CALLS.contains(&text) {
            let message = format!("`{text}` gives no value, so it must be a statement");
            return self.fail("E0303", message, span);
        }
        if text == "get" {
            return self.get(value, args, span);
        }
        if !args.is_empty() {
            self.poison_args(args, span);
            return self.fail("E0307", format!("`{text}` takes no arguments"), span);
        }
        let int = |kind| Expr { kind, ty: Ty::Int, span };
        let boxed: Box<Expr> = value.clone().into();
        match (ty, text) {
            (Ty::Int, "to_float") => Expr { kind: ExprKind::ToFloat(boxed), ty: Ty::Float, span },
            (Ty::Float, "to_int") => int(ExprKind::ToInt(boxed)),
            (Ty::Text, "len") => int(ExprKind::TextLen(boxed)),
            (Ty::Text, "bytes") => {
                Expr { kind: ExprKind::TextBytes(boxed), ty: Ty::List(&Ty::Byte), span }
            }
            (Ty::Byte, "to_int") => Expr { ty: Ty::Int, ..value },
            (_, "len") => int(ExprKind::Len(boxed)),
            _ => self.is_empty(value, span),
        }
    }

    /// Lowers arguments of a call that already failed, to report their errors.
    fn poison_args(&mut self, args: &'a [ast::Arg], span: Span) -> Expr {
        for arg in args {
            self.expected(&arg.value, Some(Ty::Error));
        }
        poisoned(span)
    }
}
