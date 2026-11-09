//! Ownership: gives every counted value an owner, and makes each count explicit
//! in MIR. See "Counting" in mir.rs.
//!
//! An expression either borrows, or gives a count to what holds it. A value
//! that a borrow needs goes to a hidden `Temp`. A place that keeps a value gets
//! a count: a new value's own, a local's through `Move` if nothing reads that
//! local again, and otherwise a `Retain`.
//!
//! With `moves` off this is the naive stage of docs/memory-model.md: every new
//! value goes to a `Temp`, and everything that keeps a value retains it. Tests
//! run both ways, since a program must not depend on an optimization.

use crate::diag::{Lines, Source, Span, json_string};
use crate::mir::{Expr, ExprKind, Local, Program, Stmt, Task, Ty, Update, Visit};
use std::collections::{BTreeSet, HashMap};

pub fn run(program: &mut Program, moves: bool) {
    let takes: Vec<Vec<Local>> = program
        .tasks
        .iter_mut()
        .map(|task| {
            // A task value's code owns its arguments in both stages, since a
            // call through a value cannot know what it calls, by decision 183.
            // Only that code is named `lambda` or has no name and takes any.
            let value_code = task.name == "lambda" || task.name.is_empty();
            if !moves && !value_code {
                task.take.clear();
            }
            task.take.clone()
        })
        .collect();
    for task in program.tasks.iter_mut().chain([&mut program.main]) {
        let mut exits = Exits::default();
        exits.stmts(&mut task.body);
        if exits.found {
            let (locals, made) = (&mut task.locals, Vec::new());
            Settle { locals, takes: &takes, made }.stmts(&mut task.body);
        }
        if moves {
            borrow_locals(task);
        }
        let mut bound = Bound::default();
        bound.stmts(&mut task.body);
        task.borrowed = bound.0.into_iter().collect();
        let live = Bits::new(task.locals.len());
        let Task { params, locals, body, take, borrowed, .. } = task;
        let mut pass = Owners {
            moves,
            params: *params,
            borrowed,
            take,
            takes: &takes,
            locals,
            rewrite: false,
            live,
            exits: Vec::new(),
            heads: Vec::new(),
            next_loop: 0,
            changed: false,
            reads: HashMap::new(),
            moving: false,
            temps: Vec::new(),
        };
        pass.walk(body);
        while moves && pass.changed {
            pass.walk(body);
        }
        pass.rewrite = true;
        pass.walk(body);
    }
}

/// A place where a program pays for a value being held twice.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct Cost {
    start: usize,
    kind: &'static str,
    ty: String,
}

/// Lists each `Retain` of `program`, which `run` has lowered. A count on a value
/// that a later write changes is a copy. A derived task has its own code at the
/// span of its first use, so it stays out, since no source line shows it.
pub fn costs(program: &mut Program) -> BTreeSet<Cost> {
    let takes: Vec<Vec<Local>> = program.tasks.iter().map(|task| task.take.clone()).collect();
    let mut found = BTreeSet::new();
    for task in program.tasks.iter_mut().chain([&mut program.main]) {
        let mut updates = Updated(vec![false; task.locals.len()]);
        updates.stmts(&mut task.body);
        let mut sites = Sites {
            task: task.span,
            takes: &takes,
            written: &updates.0,
            params: task.params,
            found: &mut found,
        };
        sites.stmts(&mut task.body);
    }
    let copies: Vec<(usize, String)> = (found.iter())
        .filter(|cost| cost.kind == "copy")
        .map(|cost| (cost.start, cost.ty.clone()))
        .collect();
    found.retain(|cost| {
        cost.kind == "copy" || !copies.iter().any(|c| c.0 == cost.start && c.1 == cost.ty)
    });
    found
}

/// Renders `costs` the way `lucid check` renders errors.
pub fn render_costs(source: &Source, costs: &BTreeSet<Cost>, json: bool) -> String {
    let lines = Lines::new(source);
    // `std` ships with the compiler, so its sites are not the program's to fix.
    let own = costs.iter().filter(|cost| !lines.place(cost.start).0.starts_with("@std/"));
    let items = own.map(|cost| {
        let (file, line, column) = lines.place(cost.start);
        let meaning = match cost.kind {
            "copy" => "is held twice, so the next write copies it",
            _ => "gets one more count here",
        };
        let (kind, ty) = (cost.kind, crate::diag::plain(&cost.ty));
        if json {
            let (file, ty) = (json_string(file), json_string(&ty));
            format!(
                "{{\"kind\":\"{kind}\",\"type\":{ty},\"file\":{file},\"line\":{line},\"column\":{column}}}"
            )
        } else {
            format!("{file}:{line}:{column}: {kind}: this `{ty}` {meaning}\n")
        }
    });
    if json { format!("[{}]\n", items.collect::<Vec<_>>().join(",")) } else { items.collect() }
}

/// Marks each local that `Stmt::Update` or a `change` argument writes.
struct Updated(Vec<bool>);

impl Visit for Updated {
    fn expr(&mut self, value: &mut Expr) {
        if let ExprKind::Change(local) = value.kind {
            self.0[local] = true;
        }
        self.expr_parts(value);
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            if let Stmt::Update { local, .. } = stmt {
                self.0[*local] = true;
            }
            self.stmt_parts(stmt);
        }
    }
}

/// Collects the sites of one task.
struct Sites<'a> {
    task: Span,
    takes: &'a [Vec<Local>],
    written: &'a [bool],
    params: usize,
    found: &'a mut BTreeSet<Cost>,
}

impl Sites<'_> {
    fn add(&mut self, kind: &'static str, value: &Expr) {
        if value.span != self.task {
            let (start, ty) = (value.span.start, value.ty.name());
            self.found.insert(Cost { start, kind, ty });
        }
    }
}

impl Visit for Sites<'_> {
    fn expr(&mut self, value: &mut Expr) {
        if matches!(value.kind, ExprKind::Retain(_)) {
            self.add("count", value);
        }
        if let ExprKind::Call(task, _, args) = &value.kind {
            for (param, arg) in args.iter().enumerate() {
                if self.takes[*task].contains(&param) && matches!(arg.kind, ExprKind::Retain(_)) {
                    self.add("copy", arg);
                }
            }
        }
        // A task value takes every argument, by decision 183.
        if let ExprKind::CallValue(_, args) = &value.kind {
            for arg in args.iter().filter(|arg| matches!(arg.kind, ExprKind::Retain(_))) {
                self.add("copy", arg);
            }
        }
        self.expr_parts(value);
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            if let Stmt::Set(local, value) = stmt
                && *local >= self.params
                && self.written[*local]
                && matches!(value.kind, ExprKind::Retain(_))
            {
                self.add("copy", value);
            }
            self.stmt_parts(stmt);
        }
    }
}

/// Finds a `return` that a `try` left inside an expression.
#[derive(Default)]
struct Exits {
    in_value: bool,
    found: bool,
}

impl Exits {
    fn in_value(value: &mut Expr) -> bool {
        let mut exits = Exits::default();
        exits.expr(value);
        exits.found
    }
}

impl Visit for Exits {
    fn expr(&mut self, value: &mut Expr) {
        let outer = std::mem::replace(&mut self.in_value, true);
        self.expr_parts(value);
        self.in_value = outer;
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            self.found |= self.in_value && matches!(stmt, Stmt::Return(_));
            self.stmt_parts(stmt);
        }
    }
}

/// A value that a record, a list, or a `take` parameter keeps has its count
/// outside any local until the keeper is done. So if one of those values can
/// return from the task, it and the ones before it run first, in order, each
/// into a local of its own, which the task's exit releases. If the statement
/// ends normally, it clears them.
struct Settle<'a> {
    locals: &'a mut Vec<Ty>,
    takes: &'a [Vec<Local>],
    /// The counted locals made so far for the open statements.
    made: Vec<Local>,
}

impl Settle<'_> {
    fn settle(&mut self, mut values: Vec<&mut Expr>) -> Vec<Stmt> {
        let last = values.iter_mut().rposition(|value| Exits::in_value(value));
        let count = last.map_or(0, |last| last + 1);
        let mut setup = Vec::with_capacity(count);
        for value in values.into_iter().take(count) {
            // A slot number holds no count and runs nothing, and must stay a number.
            if matches!(value.kind, ExprKind::Int(_)) {
                continue;
            }
            let (ty, span) = (value.ty, value.span);
            self.locals.push(ty);
            let local = self.locals.len() - 1;
            if ty.is_counted() {
                self.made.push(local);
            }
            let read = Expr { kind: ExprKind::Local(local), ty, span };
            setup.push(Stmt::Set(local, std::mem::replace(value, read)));
        }
        setup
    }
}

impl Visit for Settle<'_> {
    fn expr(&mut self, value: &mut Expr) {
        self.expr_parts(value);
        let parts: Vec<&mut Expr> = match &mut value.kind {
            ExprKind::Call(task, _, args) if !self.takes[*task].is_empty() => {
                args.iter_mut().filter(|arg| !matches!(arg.kind, ExprKind::Change(_))).collect()
            }
            ExprKind::CallValue(callee, args) => {
                std::iter::once(&mut **callee).chain(args.iter_mut()).collect()
            }
            ExprKind::ListOf(items) | ExprKind::Join(items) => items.iter_mut().collect(),
            ExprKind::MapOf(entries, _) => {
                entries.iter_mut().flat_map(|(key, value)| [key, value]).collect()
            }
            ExprKind::New { fields, .. } | ExprKind::Closure { fields, .. } => {
                fields.iter_mut().map(|(_, value)| value).collect()
            }
            _ => return,
        };
        let setup = self.settle(parts);
        if !setup.is_empty() {
            let (ty, span) = (value.ty, value.span);
            let done = std::mem::replace(value, Expr { kind: ExprKind::None, ty, span });
            value.kind = ExprKind::Seq(setup, done.into());
        }
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            let start = self.made.len();
            self.stmt_parts(stmt);
            let mut setup = match stmt {
                Stmt::Update { args, .. } => self.settle(args.iter_mut().collect()),
                _ => Vec::new(),
            };
            if !setup.is_empty() || self.made.len() > start {
                setup.push(std::mem::replace(stmt, Stmt::Unreachable));
                setup.extend(self.made.drain(start..).map(Stmt::Clear));
                *stmt = Stmt::Block(setup);
            }
        }
    }
}

/// Lists the locals that `Stmt::Bind` sets, in order.
#[derive(Default)]
struct Bound(BTreeSet<Local>);

impl Visit for Bound {
    fn expr(&mut self, value: &mut Expr) {
        self.expr_parts(value);
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            if let Stmt::Bind(local, _) = stmt {
                self.0.insert(*local);
            }
            self.stmt_parts(stmt);
        }
    }
}

/// Finds locals that need no count, and gives them `Stmt::Bind`. Such a local
/// only ever holds a path into a read parameter, or into another such local,
/// and nothing changes it. The caller keeps a read parameter alive and fixed
/// for the whole call, so what it holds stays valid.
fn borrow_locals(task: &mut Task) {
    let may_borrow = |(local, ty): (usize, &Ty)| local >= task.params && ty.is_counted();
    let locals = task.locals.iter().enumerate();
    let mut finder = Borrows {
        can: locals.map(may_borrow).collect(),
        params: task.params,
        fixed: &task.change,
        owned: &task.take,
        changed: false,
    };
    finder.stmts(&mut task.body);
    while std::mem::take(&mut finder.changed) {
        finder.stmts(&mut task.body);
    }
    Bind { can: &finder.can }.stmts(&mut task.body);
}

/// Strikes out each local that cannot borrow.
struct Borrows<'a> {
    can: Vec<bool>,
    params: usize,
    fixed: &'a [Local],
    owned: &'a [Local],
    changed: bool,
}

impl Borrows<'_> {
    fn strike(&mut self, local: Local) {
        self.changed |= std::mem::replace(&mut self.can[local], false);
    }

    /// True if `value` is a path into something that stays valid.
    fn is_path(&self, value: &Expr) -> bool {
        match &value.kind {
            ExprKind::Local(local) if *local < self.params => {
                !self.fixed.contains(local) && !self.owned.contains(local)
            }
            ExprKind::Local(local) => self.can[*local],
            ExprKind::Field(inner, _) | ExprKind::ListGet(inner, _) => self.is_path(inner),
            _ => false,
        }
    }
}

impl Visit for Borrows<'_> {
    fn expr(&mut self, value: &mut Expr) {
        if let ExprKind::Change(local) = value.kind {
            self.strike(local);
        }
        self.expr_parts(value);
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::Set(local, value) if self.can[*local] && !self.is_path(value) => {
                    self.strike(*local);
                }
                Stmt::Update { local, .. } | Stmt::Bind(local, _) => self.strike(*local),
                _ => {}
            }
            self.stmt_parts(stmt);
        }
    }
}

/// Rewrites what `Borrows` left.
struct Bind<'a> {
    can: &'a [bool],
}

impl Visit for Bind<'_> {
    fn expr(&mut self, value: &mut Expr) {
        self.expr_parts(value);
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            self.stmt_parts(stmt);
            match stmt {
                Stmt::Set(local, value) if self.can[*local] => {
                    let (local, ty, span) = (*local, value.ty, value.span);
                    let empty = Expr { kind: ExprKind::None, ty, span };
                    *stmt = Stmt::Bind(local, std::mem::replace(value, empty));
                }
                Stmt::Clear(local) if self.can[*local] => *stmt = Stmt::Block(Vec::new()),
                _ => {}
            }
        }
    }
}

/// A set of locals.
#[derive(Clone, PartialEq)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(locals: usize) -> Self {
        Bits(vec![0; locals.div_ceil(64)])
    }

    fn has(&self, local: Local) -> bool {
        self.0.get(local / 64).is_some_and(|word| word >> (local % 64) & 1 == 1)
    }

    fn add(&mut self, local: Local) {
        self.0[local / 64] |= 1 << (local % 64);
    }

    fn remove(&mut self, local: Local) {
        self.0[local / 64] &= !(1 << (local % 64));
    }

    fn add_all(&mut self, other: &Bits) {
        self.0.iter_mut().zip(&other.0).for_each(|(word, more)| *word |= more);
    }

    fn clear(&mut self) {
        self.0.fill(0);
    }
}

/// True for an expression that makes a new value, which comes with a count.
fn is_new(value: &Expr) -> bool {
    matches!(
        value.kind,
        ExprKind::Text(_)
            | ExprKind::ToText(_)
            | ExprKind::TextQuote(_)
            | ExprKind::Join(_)
            | ExprKind::Call(..)
            | ExprKind::CallValue(..)
            | ExprKind::Foreign(..)
            | ExprKind::Closure { .. }
            | ExprKind::TextSlice(..)
            | ExprKind::ListSlice(..)
            | ExprKind::TextCharAt(..)
            | ExprKind::TextBytes(_)
            | ExprKind::ListOf(_)
            | ExprKind::MapOf(..)
            | ExprKind::SetOf(..)
            | ExprKind::New { .. }
    )
}

/// True for a value in static data. Its count never matters, so a borrow needs
/// no owner for it, and a place may keep it with no `Retain`.
fn is_static(value: &Expr) -> bool {
    match &value.kind {
        ExprKind::Text(_) => true,
        ExprKind::New { fields, .. } => fields.is_empty(),
        _ => false,
    }
}

/// Counts each read of a local, including `Change` and `Update`.
#[derive(Default)]
struct Reads(HashMap<Local, usize>);

impl Reads {
    fn add(&mut self, local: Local) {
        *self.0.entry(local).or_default() += 1;
    }
}

impl Visit for Reads {
    fn expr(&mut self, value: &mut Expr) {
        if let ExprKind::Local(local) | ExprKind::Change(local) = value.kind {
            self.add(local);
        }
        self.expr_parts(value);
    }

    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            if let Stmt::Update { local, .. } = stmt {
                self.add(*local);
            }
            self.stmt_parts(stmt);
        }
    }
}

struct Owners<'a> {
    moves: bool,
    params: usize,
    /// The locals that hold no count, in order.
    borrowed: &'a [Local],
    /// The `take` parameters of this task, whose count it holds on entry.
    take: &'a [Local],
    takes: &'a [Vec<Local>],
    locals: &'a mut Vec<Ty>,
    /// False while finding which locals are live at the head of each loop.
    rewrite: bool,
    /// Locals read before they are written, from this point on. The walk runs
    /// backward, one statement at a time.
    live: Bits,
    /// What is live at each `break` and `continue` of the open loops.
    exits: Vec<(Bits, Bits)>,
    /// What is live at the head of each loop, by order of walking.
    heads: Vec<Bits>,
    next_loop: usize,
    changed: bool,
    /// What the statement in progress reads, and how often.
    reads: HashMap<Local, usize>,
    /// True if that statement may move a local that it reads once, and that
    /// nothing reads after it. A local read twice in one statement may be
    /// borrowed by the other read, and that borrow must stay valid to the end.
    moving: bool,
    /// The temporaries of the statement in progress.
    temps: Vec<Local>,
}

impl Owners<'_> {
    /// Walks the whole task once. Each loop head starts from what the last walk
    /// found, so repeating until nothing changes finds what is live. Nested
    /// loops cost one walk per level of depth, not 2^depth.
    fn walk(&mut self, body: &mut [Stmt]) {
        self.changed = false;
        self.next_loop = 0;
        self.live.clear();
        self.flow(body);
    }

    /// Statements run forward, so a walk of what is live runs backward.
    fn flow(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts.iter_mut().rev() {
            let start = self.temps.len();
            self.flow_stmt(stmt);
            if self.temps.len() > start && !matches!(stmt, Stmt::Return(_)) {
                let clears = self.temps.drain(start..).rev().map(Stmt::Clear);
                let done = std::mem::replace(stmt, Stmt::Unreachable);
                *stmt = Stmt::Block(std::iter::once(done).chain(clears).collect());
            }
            self.temps.truncate(start);
        }
    }

    fn flow_stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Set(local, value) => {
                self.live.remove(*local);
                let mut reads = Reads::default();
                reads.expr(value);
                self.start(reads);
                if self.rewrite {
                    self.append(*local, value);
                    self.give(value);
                }
                self.finish();
            }
            Stmt::Bind(local, value) => {
                self.live.remove(*local);
                self.borrow_one(value);
            }
            Stmt::Return(value) => {
                self.live.clear();
                self.give_one(value);
            }
            Stmt::Exit(value) | Stmt::Fail(value, _) => {
                self.live.clear();
                self.borrow_one(value);
            }
            Stmt::Print(value) | Stmt::Expr(value) => self.borrow_one(value),
            Stmt::Update { local, op, args, .. } => {
                let mut reads = Reads::default();
                reads.add(*local);
                args.iter_mut().for_each(|arg| reads.expr(arg));
                self.start(reads);
                self.update_args(*op, args);
                self.finish();
            }
            Stmt::Clear(local) => self.live.remove(*local),
            Stmt::Block(stmts) => self.flow(stmts),
            Stmt::If { branches, otherwise } => {
                let after = self.live.clone();
                self.flow(otherwise);
                for (condition, then) in branches.iter_mut().rev() {
                    let next = std::mem::replace(&mut self.live, after.clone());
                    self.flow(then);
                    self.live.add_all(&next);
                    self.borrow_one(condition);
                }
            }
            Stmt::While { condition, body } => {
                let head = self.enter_loop();
                self.flow(body);
                let after = self.leave_loop();
                self.live.add_all(&after);
                self.borrow_one(condition);
                self.end_loop(head);
            }
            Stmt::For { start, end, body, .. } => {
                let head = self.enter_loop();
                self.flow(body);
                let after = self.leave_loop();
                self.live.add_all(&after);
                self.end_loop(head);
                self.borrow_one(end);
                self.borrow_one(start);
            }
            Stmt::Break => self.live.clone_from(&self.exits.last().expect("in a loop").0),
            Stmt::Continue => self.live.clone_from(&self.exits.last().expect("in a loop").1),
            Stmt::Placeholder(_) | Stmt::Trap(..) | Stmt::Unreachable => self.live.clear(),
            Stmt::PrintText(_) => {}
        }
    }

    /// Sets up the body of a loop, whose end flows to the loop's head.
    fn enter_loop(&mut self) -> usize {
        let head = self.next_loop;
        self.next_loop += 1;
        if head == self.heads.len() {
            self.heads.push(Bits::new(self.live.0.len() * 64));
        }
        self.exits.push((self.live.clone(), self.heads[head].clone()));
        self.live.clone_from(&self.heads[head]);
        head
    }

    /// Gives what is live after the loop.
    fn leave_loop(&mut self) -> Bits {
        self.exits.pop().expect("in a loop").0
    }

    fn end_loop(&mut self, head: usize) {
        if self.heads[head] != self.live {
            self.heads[head].clone_from(&self.live);
            self.changed = true;
        }
    }

    fn give_one(&mut self, value: &mut Expr) {
        let mut reads = Reads::default();
        reads.expr(value);
        self.start(reads);
        if self.rewrite {
            self.give(value);
        }
        self.finish();
    }

    fn borrow_one(&mut self, value: &mut Expr) {
        let mut reads = Reads::default();
        reads.expr(value);
        self.start(reads);
        if self.rewrite {
            self.expr(value);
        }
        self.finish();
    }

    /// Starts one statement. `self.live` holds what is live after it.
    fn start(&mut self, reads: Reads) {
        self.reads = reads.0;
        self.moving = self.rewrite && self.moves;
    }

    /// Ends one statement, with `self.live` as what is live before it.
    fn finish(&mut self) {
        self.moving = false;
        std::mem::take(&mut self.reads).into_keys().for_each(|local| self.live.add(local));
    }

    fn update_args(&mut self, op: Update, args: &mut [Expr]) {
        if !self.rewrite {
            return;
        }
        for arg in args {
            // `MapRemove` only looks at its key, and `Put` hands back a count
            // that the local already holds.
            if matches!(op, Update::MapRemove | Update::Put) {
                self.expr(arg);
            } else {
                self.give(arg);
            }
        }
    }

    fn can_move(&self, local: Local) -> bool {
        self.moving
            && local >= self.params
            && self.reads.get(&local) == Some(&1)
            && !self.live.has(local)
            && self.borrowed.binary_search(&local).is_err()
    }

    /// `t = "{t}..."` moves `t` into the join, so the text may grow in place,
    /// by decision 222. The `Set` writes `t` next, and a local that `Set`
    /// writes holds its own count: a `change` parameter holds the caller's,
    /// and a local that `Update::Take` filled holds the slot's.
    fn append(&self, local: Local, value: &mut Expr) {
        if self.moving
            && self.reads.get(&local) == Some(&1)
            && let ExprKind::Join(parts) = &mut value.kind
            && matches!(parts[0].kind, ExprKind::Local(first) if first == local)
        {
            parts[0].kind = ExprKind::Move(local);
        }
    }

    /// Makes `value` give a count to what keeps it.
    fn give(&mut self, value: &mut Expr) {
        if !value.ty.is_counted() {
            return self.expr(value);
        }
        // A `take` parameter is read once, by the move that starts its task.
        if let ExprKind::Local(local) = value.kind
            && self.take.contains(&local)
        {
            return;
        }
        if self.moves {
            if let ExprKind::Join(parts) = &mut value.kind
                && let ExprKind::Local(first) = parts[0].kind
                && self.can_move(first)
            {
                parts[0].kind = ExprKind::Move(first);
            }
            if is_new(value) {
                return self.parts(value);
            }
            match &mut value.kind {
                ExprKind::Local(local) if self.can_move(*local) => {
                    value.kind = ExprKind::Move(*local);
                    return;
                }
                ExprKind::If(condition, then, otherwise) => {
                    self.expr(condition);
                    self.give(then);
                    return self.give(otherwise);
                }
                ExprKind::Seq(setup, inner) => {
                    self.nested(setup);
                    return self.give(inner);
                }
                _ => {}
            }
        }
        self.expr(value);
        let (span, ty) = (value.span, value.ty);
        let inner = std::mem::replace(value, Expr { kind: ExprKind::None, ty, span });
        value.kind = ExprKind::Retain(inner.into());
    }

    /// Statements inside an expression. What they hold may be read later in
    /// the same statement, so they move nothing.
    fn nested(&mut self, stmts: &mut [Stmt]) {
        let moving = std::mem::take(&mut self.moving);
        self.stmts(stmts);
        self.moving = moving;
    }

    /// Lowers the parts of `value`, giving a count to each one it keeps.
    fn parts(&mut self, value: &mut Expr) {
        match &mut value.kind {
            ExprKind::Call(task, _, args) => {
                for (param, arg) in args.iter_mut().enumerate() {
                    if self.takes[*task].contains(&param) {
                        self.give(arg);
                    } else {
                        self.expr(arg);
                    }
                }
            }
            // A task value takes every argument, by decision 183.
            ExprKind::CallValue(callee, args) => {
                self.expr(callee);
                args.iter_mut().for_each(|arg| self.give(arg));
            }
            ExprKind::ListOf(items) => items.iter_mut().for_each(|item| self.give(item)),
            ExprKind::MapOf(entries, _) => {
                for (key, value) in entries {
                    self.give(key);
                    self.give(value);
                }
            }
            ExprKind::New { fields, .. } | ExprKind::Closure { fields, .. } => {
                fields.iter_mut().for_each(|(_, value)| self.give(value));
            }
            ExprKind::Seq(setup, inner) => {
                self.nested(setup);
                self.expr(inner);
            }
            _ => self.expr_parts(value),
        }
    }
}

impl Visit for Owners<'_> {
    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::Set(_, value) | Stmt::Return(value) => self.give(value),
                Stmt::Update { op, args, .. } => self.update_args(*op, args),
                _ => self.stmt_parts(stmt),
            }
        }
    }

    /// Lowers a value that is only borrowed.
    fn expr(&mut self, value: &mut Expr) {
        self.parts(value);
        if is_new(value) && value.ty.is_counted() && !is_static(value) {
            self.locals.push(value.ty);
            let (local, span, ty) = (self.locals.len() - 1, value.span, value.ty);
            let inner = std::mem::replace(value, Expr { kind: ExprKind::None, ty, span });
            value.kind = ExprKind::Temp(local, inner.into());
            self.temps.push(local);
        }
    }
}
