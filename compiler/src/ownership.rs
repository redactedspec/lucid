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

use crate::diag::{Lines, Span, json_string};
use crate::mir::{Expr, ExprKind, Local, Program, Stmt, Task, Ty, Update, Visit};
use std::collections::{BTreeSet, HashMap};

pub fn run(program: &mut Program, moves: bool) {
    let takes: Vec<Vec<Local>> = program
        .tasks
        .iter_mut()
        .map(|task| {
            task.take.retain(|_| moves);
            task.take.clone()
        })
        .collect();
    for task in program.tasks.iter_mut().chain([&mut program.main]) {
        if moves {
            task.borrowed = borrow_locals(task);
        }
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
            moving: None,
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
pub fn render_costs(file: &str, src: &str, costs: &BTreeSet<Cost>, json: bool) -> String {
    let lines = Lines::new(src);
    let mut items = costs.iter().map(|cost| {
        let (line, column) = lines.position(cost.start);
        let meaning = match cost.kind {
            "copy" => "is held twice, so the next write copies it",
            _ => "gets one more count here",
        };
        let (kind, ty) = (cost.kind, &cost.ty);
        if json {
            let (file, ty) = (json_string(file), json_string(ty));
            format!(
                "{{\"kind\":\"{kind}\",\"type\":{ty},\"file\":{file},\"line\":{line},\"column\":{column}}}"
            )
        } else {
            format!("{file}:{line}:{column}: {kind}: this `{ty}` {meaning}\n")
        }
    });
    if json {
        format!("[{}]\n", items.collect::<Vec<_>>().join(","))
    } else {
        items.by_ref().collect()
    }
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

/// Finds locals that need no count, and gives them `Stmt::Bind`. Such a local
/// only ever holds a path into a read parameter, or into another such local,
/// and nothing changes it. The caller keeps a read parameter alive and fixed
/// for the whole call, so what it holds stays valid. Returns them in order.
fn borrow_locals(task: &mut Task) -> Vec<Local> {
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
    let can = finder.can;
    Bind { can: &can }.stmts(&mut task.body);
    (0..can.len()).filter(|&local| can[local]).collect()
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
                Stmt::Update { local, .. } => self.strike(*local),
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

/// What the statement in progress may move: locals that it reads once, and
/// that nothing reads after it. A local read twice in one statement may be
/// borrowed by the other read, and that borrow must stay valid to the end.
struct Moving {
    reads: HashMap<Local, usize>,
    after: Bits,
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
    moving: Option<Moving>,
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
                self.give_one(value);
            }
            Stmt::Bind(local, value) => {
                self.live.remove(*local);
                self.borrow_one(value);
            }
            Stmt::Return(value) => {
                self.live.clear();
                self.give_one(value);
            }
            Stmt::Print(value) | Stmt::Expr(value) => self.borrow_one(value),
            Stmt::Update { local, op, args, .. } => {
                let mut reads = Reads::default();
                reads.add(*local);
                args.iter_mut().for_each(|arg| reads.expr(arg));
                self.start(&reads);
                self.update_args(*op, args);
                self.finish(reads);
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
        self.start(&reads);
        if self.rewrite {
            self.give(value);
        }
        self.finish(reads);
    }

    fn borrow_one(&mut self, value: &mut Expr) {
        let mut reads = Reads::default();
        reads.expr(value);
        self.start(&reads);
        if self.rewrite {
            self.expr(value);
        }
        self.finish(reads);
    }

    /// Starts one statement. `self.live` holds what is live after it.
    fn start(&mut self, reads: &Reads) {
        if self.rewrite && self.moves {
            self.moving = Some(Moving { reads: reads.0.clone(), after: self.live.clone() });
        }
    }

    /// Ends one statement, with `self.live` as what is live before it.
    fn finish(&mut self, reads: Reads) {
        self.moving = None;
        reads.0.into_keys().for_each(|local| self.live.add(local));
    }

    fn update_args(&mut self, op: Update, args: &mut [Expr]) {
        if !self.rewrite {
            return;
        }
        for arg in args {
            // `MapRemove` only looks at its key.
            if op == Update::MapRemove {
                self.expr(arg);
            } else {
                self.give(arg);
            }
        }
    }

    fn can_move(&self, local: Local) -> bool {
        self.moving.as_ref().is_some_and(|moving| {
            local >= self.params
                && moving.reads.get(&local) == Some(&1)
                && !moving.after.has(local)
                && self.borrowed.binary_search(&local).is_err()
        })
    }

    /// Makes `value` give a count to what keeps it.
    fn give(&mut self, value: &mut Expr) {
        if !value.ty.is_counted() {
            return self.expr(value);
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
                ExprKind::Local(local) if self.take.contains(local) => return,
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
        let moving = self.moving.take();
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
