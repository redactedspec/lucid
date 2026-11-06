//! The monomorphizer: one concrete task per used set of type arguments.
//!
//! The checker checks a generic body once, with its parameters as opaque types.
//! Unions flatten, so `T?` is `int?` for `T = int?`, and what counts as a union
//! or a counted value is only known here. So the checker records intent, as
//! `Wrap`, `View`, `Member`, and `Holds`, and this pass picks the code for each.
//! No type error can appear here: substitution keeps equal types equal.

use crate::derive::Derived;
use crate::diag::{Diagnostic, Span};
use crate::mir::{Expr, ExprKind, KeyTasks, Program, Shape, Stmt, Task, Ty, Types, Update, Visit};
use std::collections::HashMap;

/// Generic instances and derived tasks past this stop as E0903. See also
/// `Ty::too_large`: a size limit alone can fire late when one task makes
/// several larger copies of itself.
const MAX_INSTANCES: usize = 20_000;

pub fn run(program: &mut Program) -> Vec<Diagnostic> {
    let source = std::mem::take(&mut program.tasks);
    let mut mono = Mono {
        source: &source,
        methods: &program.methods,
        datas: &program.datas,
        types: &mut program.types,
        instances: HashMap::new(),
        derived: HashMap::new(),
        done: Vec::new(),
        generated: 0,
        queue: Vec::new(),
        diags: Vec::new(),
        args: &[],
        locals: Vec::new(),
    };
    // Every task without parameters compiles, used or not.
    for (id, task) in source.iter().enumerate() {
        if task.type_params == 0 {
            mono.instance(id, &[], task.span);
        }
    }
    mono.task(&mut program.main, &[]);
    while let Some((slot, job)) = mono.queue.pop() {
        if !mono.diags.is_empty() {
            break;
        }
        let task = match job {
            Job::Task(id, args) => {
                let mut task = source[id].clone();
                mono.task(&mut task, args);
                task
            }
            Job::Derived(kind, ty, span) => mono.derive(kind, ty, span),
        };
        mono.done[slot] = Some(task);
    }
    let (done, diags) = (mono.done, mono.diags);
    if diags.is_empty() {
        program.tasks = done.into_iter().map(|task| task.expect("the queue is empty")).collect();
    }
    diags
}

/// A task to build.
enum Job {
    /// A source task, and its type arguments.
    Task(usize, &'static [Ty]),
    Derived(Derived, Ty, Span),
}

pub struct Mono<'a> {
    source: &'a [Task],
    methods: &'a HashMap<String, Vec<usize>>,
    pub datas: &'a [Shape],
    pub types: &'a mut Types,
    /// A generic task with its type arguments, and its place in `done`.
    instances: HashMap<(usize, &'static [Ty]), usize>,
    /// A derived task for a type, and its place in `done`.
    derived: HashMap<(Derived, Ty), usize>,
    done: Vec<Option<Task>>,
    /// How many generic instances and derived tasks exist.
    generated: usize,
    /// Tasks still to build, with their places in `done`.
    queue: Vec<(usize, Job)>,
    diags: Vec<Diagnostic>,
    /// The type arguments of the task being rewritten.
    args: &'static [Ty],
    /// The locals of that task, with real types.
    locals: Vec<Ty>,
}

impl Mono<'_> {
    fn instance(&mut self, id: usize, args: &'static [Ty], span: Span) -> usize {
        let Some(_) = self.check_size(args, span) else { return 0 };
        if let Some(&slot) = self.instances.get(&(id, args)) {
            return slot;
        }
        let Some(slot) = self.reserve(args, span) else { return 0 };
        self.instances.insert((id, args), slot);
        self.queue.push((slot, Job::Task(id, args)));
        slot
    }

    /// The task with a `self` parameter that takes this type, if any. The
    /// checker keeps receivers from overlapping, so one fits at most.
    pub fn method(&mut self, name: &str, ty: Ty, span: Span) -> Option<usize> {
        self.method_and_result(name, ty, span).map(|(slot, _)| slot)
    }

    /// Like `method`, with the type that the task returns.
    pub fn method_and_result(&mut self, name: &str, ty: Ty, span: Span) -> Option<(usize, Ty)> {
        let ids = self.methods.get(name)?;
        for &id in ids {
            let task = &self.source[id];
            if let Some(known) = self.types.fit(task.locals[0], ty, task.type_params) {
                let args: Vec<Ty> = known.into_iter().collect::<Option<_>>()?;
                let result = self.types.subst(task.returns, &args);
                return Some((self.instance(id, args.leak(), span), result));
            }
        }
        None
    }

    /// The derived task of this kind for a type.
    pub fn derived(&mut self, kind: Derived, ty: Ty, span: Span) -> usize {
        let Some(_) = self.check_size(&[ty], span) else { return 0 };
        if let Some(&slot) = self.derived.get(&(kind, ty)) {
            return slot;
        }
        let Some(slot) = self.reserve(&[ty], span) else { return 0 };
        self.derived.insert((kind, ty), slot);
        self.queue.push((slot, Job::Derived(kind, ty, span)));
        slot
    }

    /// The tasks that a map with this key keeps.
    fn key_tasks(&mut self, key: Ty, span: Span) -> KeyTasks {
        if key.is_key() {
            return None;
        }
        let equal = match self.method("equal", key, span) {
            Some(slot) => slot,
            None => self.derived(Derived::Equal, key, span),
        };
        Some((self.derived(Derived::Hash, key, span), equal))
    }

    /// Reports E0903 for a type too large to hash. Slot 0 then stands in, and
    /// nothing runs, since the error stops the program.
    fn check_size(&mut self, types: &[Ty], span: Span) -> Option<()> {
        if types.iter().any(|ty| ty.too_large()) {
            self.too_many(span);
            return None;
        }
        Some(())
    }

    fn too_many(&mut self, span: Span) {
        let note = "a generic task or type keeps asking for a larger type of itself";
        let diag = Diagnostic::new("E0903", "too many generic instances", span);
        self.diags.push(diag.note(note));
    }

    /// A place in `done`. A task with no type arguments always gets one.
    fn reserve(&mut self, types: &[Ty], span: Span) -> Option<usize> {
        if !types.is_empty() {
            if self.generated == MAX_INSTANCES {
                self.too_many(span);
                return None;
            }
            self.generated += 1;
        }
        self.done.push(None);
        Some(self.done.len() - 1)
    }

    fn task(&mut self, task: &mut Task, args: &'static [Ty]) {
        self.args = args;
        task.type_params = 0;
        task.returns = self.types.subst(task.returns, args);
        self.locals = task.locals.iter().map(|&ty| self.types.subst(ty, args)).collect();
        self.stmts(&mut task.body);
        task.locals = std::mem::take(&mut self.locals);
    }

    /// The slot of a field, and how many counted slots its variant has.
    /// Counted fields take the first slots, so that a record needs only their
    /// number to know what it holds.
    pub fn layout(&mut self, data: Ty, variant: usize, field: usize) -> (usize, usize) {
        let Ty::Data(_, id, args) = data else { unreachable!("only a `type` or `choice`") };
        let fields = &self.datas[id].variants[variant].fields;
        let counted: Vec<bool> =
            fields.iter().map(|&(_, ty)| self.types.subst(ty, args).is_counted()).collect();
        let before = |want: bool, end: usize| counted[..end].iter().filter(|&&c| c == want).count();
        let total = before(true, counted.len());
        let slot = match counted.get(field) {
            Some(true) => before(true, field),
            Some(false) => total + before(false, field),
            None => 0,
        };
        (slot, total)
    }

    /// A hidden local of the task being rewritten.
    pub fn new_local(&mut self, ty: Ty) -> usize {
        self.locals.push(ty);
        self.locals.len() - 1
    }

    /// A value as the wider type `to`.
    fn wrap(&self, value: Expr, to: Ty, span: Span) -> Expr {
        if !matches!(to, Ty::Union(_)) || matches!(value.ty, Ty::Union(_)) {
            return Expr { ty: to, ..value };
        }
        let (tag, counted) = (self.types.id(value.ty), usize::from(value.ty.is_counted()));
        Expr { kind: ExprKind::New { tag, counted, fields: vec![(0, value)] }, ty: to, span }
    }

    /// A union as the narrower type `to`.
    fn view(&self, value: Expr, to: Ty, span: Span) -> Expr {
        if value.ty == to || matches!(to, Ty::Union(_)) || !matches!(value.ty, Ty::Union(_)) {
            return Expr { ty: to, ..value };
        }
        Expr { kind: ExprKind::Field(value.into(), 0), ty: to, span }
    }

    /// A test of what a union holds. With a parameter gone, the value may no
    /// longer be a union, and the answer is then known. The value still runs.
    fn holds(&mut self, value: Expr, members: &[Ty], span: Span) -> ExprKind {
        let members: Vec<Ty> = members.iter().map(|&m| self.types.subst(m, self.args)).collect();
        if matches!(value.ty, Ty::Union(_)) {
            let wanted = members.iter().flat_map(|m| m.members());
            let tags = wanted.map(|&m| self.types.id(m)).collect();
            return ExprKind::TagIn(value.into(), tags);
        }
        let answer = members.iter().any(|member| member.members().contains(&value.ty));
        let answer = Expr { kind: ExprKind::Bool(answer), ty: Ty::Bool, span };
        ExprKind::Seq(vec![Stmt::Expr(value)], answer.into())
    }
}

impl Visit for Mono<'_> {
    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            self.stmt_parts(stmt);
            match stmt {
                Stmt::Print(value) if value.ty.is_compound() => {
                    let span = value.span;
                    let empty = Expr { kind: ExprKind::None, ty: Ty::None, span };
                    let taken = std::mem::replace(value, empty);
                    *value = self.shown_alone(taken, span);
                }
                Stmt::Clear(local) if !self.locals[*local].is_counted() => {
                    *stmt = Stmt::Block(Vec::new());
                }
                Stmt::Update { local, op: Update::FieldSet, args, .. } => {
                    let ExprKind::Int(field) = args[0].kind else {
                        unreachable!("a field write names its field")
                    };
                    let (slot, _) = self.layout(self.locals[*local], 0, field as usize);
                    args[0].kind = ExprKind::Int(slot as i64);
                }
                Stmt::Update { local, op: op @ Update::UnionSet, args, .. } => {
                    if matches!(self.locals[*local], Ty::Union(_)) {
                        *op = Update::FieldSet;
                    } else {
                        // A narrowed union that lost its other members: the
                        // local is the value. See `Place` in check.rs.
                        let value = args.pop().expect("a write has a value");
                        *stmt = if matches!(value.kind, ExprKind::None) {
                            Stmt::Block(Vec::new())
                        } else {
                            Stmt::Set(*local, value)
                        };
                    }
                }
                _ => {}
            }
        }
    }

    fn expr(&mut self, value: &mut Expr) {
        self.expr_parts(value);
        let span = value.span;
        value.ty = self.types.subst(value.ty, self.args);
        let ty = value.ty;
        let kind = std::mem::replace(&mut value.kind, ExprKind::None);
        value.kind = match kind {
            ExprKind::Call(id, args, values) => {
                let args = self.types.subst_all(args, self.args);
                ExprKind::Call(self.instance(id, args, span), &[], values)
            }
            ExprKind::Wrap(inner) => {
                *value = self.wrap(*inner, ty, span);
                return;
            }
            ExprKind::View(inner) => {
                *value = self.view(*inner, ty, span);
                return;
            }
            ExprKind::Holds(inner, members) => self.holds(*inner, &members, span),
            ExprKind::ToText(inner) => {
                *value = self.shown_alone(*inner, span);
                return;
            }
            ExprKind::Binary(op, left, right) if left.ty.is_compound() => {
                *value = self.compare(op, *left, *right, ty, span);
                return;
            }
            ExprKind::Method(name, values) => {
                let slot = self.method(name, values[0].ty, span);
                ExprKind::Call(slot.expect("a bound was checked at the call"), &[], values)
            }
            ExprKind::MapOf(entries, _) => {
                let Ty::Map(key, _) = ty else { unreachable!("it has a map type") };
                ExprKind::MapOf(entries, self.key_tasks(*key, span))
            }
            ExprKind::SetOf(list, _) => {
                let Ty::Set(key) = ty else { unreachable!("it has a set type") };
                ExprKind::SetOf(list, self.key_tasks(*key, span))
            }
            ExprKind::ListHas(item, list) if !item.ty.is_key() => {
                *value = self.list_has(*item, *list, span);
                return;
            }
            ExprKind::Member { value: inner, variant, field } => {
                let (slot, _) = self.layout(inner.ty, variant, field);
                ExprKind::Field(inner, slot)
            }
            ExprKind::New { tag, fields, .. } => {
                let (_, counted) = self.layout(ty, tag, usize::MAX);
                let fields = fields
                    .into_iter()
                    .map(|(field, value)| (self.layout(ty, tag, field).0, value))
                    .collect();
                ExprKind::New { tag, counted, fields }
            }
            other => other,
        };
    }
}
