//! Cranelift backend. Lowers MIR to machine code, and runs it in this process.

use crate::ast::BinaryOp;
use crate::diag::{Diagnostic, Lines, Span};
use crate::mir::{Expr, ExprKind, Local, Program, Stmt, Task, Ty};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block, FuncRef, GlobalValue, InstBuilder, Signature, StackSlotData, StackSlotKind,
    TrapCode, Type, Value, types,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, default_libcall_names};
use std::collections::HashMap;

const CODEGEN: &str = "code generation failed";

/// False only in the benchmark build. See "Commands" in docs/compiler.md.
const CHECK_OVERFLOW: bool = !cfg!(feature = "bench-unchecked");

/// Stack kept free below the limit, for runtime calls and for one task's frame.
const STACK_MARGIN: usize = 1024 * 1024;

/// The stack check runs after the prologue, so a frame must fit in the margin.
/// Half the margin stays spare, for runtime calls.
const MAX_FRAME: usize = STACK_MARGIN / 2;

/// Marks the end of paths that cannot continue, such as after `lucid_trap`.
const UNREACHABLE: TrapCode = TrapCode::unwrap_user(1);

/// Compiles the program and runs it on this thread, whose stack is
/// `stack_bytes` long. `path` and `src` place trap messages.
pub fn run(
    program: &Program,
    path: &str,
    src: &str,
    stack_bytes: usize,
) -> Result<(), Vec<Diagnostic>> {
    let here = 0u8;
    // ponytail: a limit in the code fits one JIT thread. `lucid build` and
    // `spawn` need a per-thread limit that the runtime sets up.
    let stack_limit = (&raw const here) as usize - (stack_bytes - STACK_MARGIN);
    let entry = compile(program, path, src, stack_limit, stack_bytes / 2)?;
    // SAFETY: `compile` built `entry` with no parameters and a one-byte `none`
    // result. Its module is leaked, so the code stays mapped.
    let entry = unsafe { std::mem::transmute::<*const u8, extern "C" fn() -> u8>(entry) };
    entry();
    lucid_rt::lucid_check_leaks();
    Ok(())
}

fn clif(ty: Ty) -> Type {
    match ty {
        Ty::Int | Ty::Text => types::I64,
        Ty::Float => types::F64,
        Ty::Bool | Ty::None => types::I8,
        Ty::Error => unreachable!("errors stop before the backend"),
    }
}

struct Runtime {
    print: FuncId,
    print_int: FuncId,
    print_float: FuncId,
    print_bool: FuncId,
    trap: FuncId,
    retain: FuncId,
    release: FuncId,
    text_new: FuncId,
    text_join: FuncId,
    text_print: FuncId,
    text_compare: FuncId,
    text_len: FuncId,
    text_is_empty: FuncId,
    text_slice: FuncId,
    text_byte_len: FuncId,
    text_char_at: FuncId,
    int_text: FuncId,
    float_text: FuncId,
    bool_text: FuncId,
}

fn signature_of(module: &JITModule, task: &Task) -> Signature {
    let mut signature = module.make_signature();
    signature.params.extend(task.locals[..task.params].iter().map(|&t| AbiParam::new(clif(t))));
    signature.returns.push(AbiParam::new(clif(task.returns)));
    signature
}

/// Panics on a failure: valid input reaching here makes that a compiler bug.
fn compile(
    program: &Program,
    path: &str,
    src: &str,
    stack_limit: usize,
    max_main_frame: usize,
) -> Result<*const u8, Vec<Diagnostic>> {
    let flags = [
        ("opt_level", "speed"),
        // Windows commits stack one page at a time, so large frames must probe.
        ("enable_probestack", "true"),
        ("probestack_strategy", "inline"),
    ];
    let mut builder = JITBuilder::with_flags(&flags, default_libcall_names()).expect(CODEGEN);
    builder.symbol("lucid_print", lucid_rt::lucid_print as *const u8);
    builder.symbol("lucid_print_int", lucid_rt::lucid_print_int as *const u8);
    builder.symbol("lucid_print_float", lucid_rt::lucid_print_float as *const u8);
    builder.symbol("lucid_print_bool", lucid_rt::lucid_print_bool as *const u8);
    builder.symbol("lucid_trap", lucid_rt::lucid_trap as *const u8);
    builder.symbol("lucid_retain", lucid_rt::lucid_retain as *const u8);
    builder.symbol("lucid_release", lucid_rt::lucid_release as *const u8);
    builder.symbol("lucid_text_new", lucid_rt::lucid_text_new as *const u8);
    builder.symbol("lucid_text_join", lucid_rt::lucid_text_join as *const u8);
    builder.symbol("lucid_text_print", lucid_rt::lucid_text_print as *const u8);
    builder.symbol("lucid_text_compare", lucid_rt::lucid_text_compare as *const u8);
    builder.symbol("lucid_text_len", lucid_rt::lucid_text_len as *const u8);
    builder.symbol("lucid_text_is_empty", lucid_rt::lucid_text_is_empty as *const u8);
    builder.symbol("lucid_text_slice", lucid_rt::lucid_text_slice as *const u8);
    builder.symbol("lucid_text_byte_len", lucid_rt::lucid_text_byte_len as *const u8);
    builder.symbol("lucid_text_char_at", lucid_rt::lucid_text_char_at as *const u8);
    builder.symbol("lucid_int_text", lucid_rt::lucid_int_text as *const u8);
    builder.symbol("lucid_float_text", lucid_rt::lucid_float_text as *const u8);
    builder.symbol("lucid_bool_text", lucid_rt::lucid_bool_text as *const u8);
    let mut module = JITModule::new(builder);

    let pointer = module.target_config().pointer_type();
    let mut import = |name: &str, params: &[AbiParam], returns: &[AbiParam]| {
        let mut signature = module.make_signature();
        signature.params.extend_from_slice(params);
        signature.returns.extend_from_slice(returns);
        module.declare_function(name, Linkage::Import, &signature).expect(CODEGEN)
    };
    let text = [AbiParam::new(pointer), AbiParam::new(types::I64)];
    let (int, float) = ([AbiParam::new(types::I64)], [AbiParam::new(types::F64)]);
    let (byte, value) = ([AbiParam::new(types::I8).uext()], [AbiParam::new(pointer)]);
    let two = [value[0], value[0]];
    let runtime = Runtime {
        print: import("lucid_print", &text, &[]),
        print_int: import("lucid_print_int", &int, &[]),
        print_float: import("lucid_print_float", &float, &[]),
        print_bool: import("lucid_print_bool", &byte, &[]),
        trap: import("lucid_trap", &text, &[]),
        retain: import("lucid_retain", &value, &[]),
        release: import("lucid_release", &value, &[]),
        text_new: import("lucid_text_new", &text, &value),
        text_join: import("lucid_text_join", &text, &value),
        text_print: import("lucid_text_print", &value, &[]),
        text_compare: import("lucid_text_compare", &two, &int),
        text_len: import("lucid_text_len", &value, &int),
        text_is_empty: import("lucid_text_is_empty", &value, &[AbiParam::new(types::I8)]),
        text_slice: import("lucid_text_slice", &[value[0], int[0], int[0]], &value),
        text_byte_len: import("lucid_text_byte_len", &value, &int),
        text_char_at: import("lucid_text_char_at", &[value[0], int[0]], &value),
        int_text: import("lucid_int_text", &int, &value),
        float_text: import("lucid_float_text", &float, &value),
        bool_text: import("lucid_bool_text", &byte, &value),
    };

    let mut ids = Vec::with_capacity(program.tasks.len());
    for task in &program.tasks {
        let signature = signature_of(&module, task);
        ids.push(module.declare_anonymous_function(&signature).expect(CODEGEN));
    }
    let signature = signature_of(&module, &program.main);
    let main = module.declare_anonymous_function(&signature).expect(CODEGEN);

    let lines = Lines::new(src);
    let mut diags = Vec::new();
    let mut context = module.make_context();
    let mut builder_context = FunctionBuilderContext::new();
    let tasks = program.tasks.iter().zip(&ids).map(|(task, id)| (task, *id));
    for (task, id) in tasks.chain([(&program.main, main)]) {
        context.func.signature = signature_of(&module, task);
        let body = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let (module, runtime, ids, lines) = (&mut module, &runtime, &ids, &lines);
        let data_id = module.declare_anonymous_data(false, false).expect(CODEGEN);
        let data = module.declare_data_in_func(data_id, body.func);
        let (vars, loops, funcs, text) = (Vec::new(), Vec::new(), HashMap::new(), Vec::new());
        let (owned, exit) = (Vec::new(), None);
        Lower {
            module,
            body,
            pointer,
            runtime,
            ids,
            vars,
            owned,
            exit,
            loops,
            funcs,
            data,
            text,
            path,
            lines,
        }
        .task(task, data_id, stack_limit);
        module.define_function(id, &mut context).expect(CODEGEN);
        let code = context.compiled_code().expect(CODEGEN);
        let frame = code.buffer.frame_layout().expect(CODEGEN).frame_to_fp_offset as usize;
        if frame > if id == main { max_main_frame } else { MAX_FRAME } {
            let note = "split it into smaller tasks";
            diags.push(Diagnostic::new("E0902", "this task is too large", task.span).note(note));
        }
        module.clear_context(&mut context);
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    module.finalize_definitions().expect(CODEGEN);
    let entry = module.get_finalized_function(main);
    // Leaked on purpose: `entry` points into memory that the module owns.
    std::mem::forget(module);
    Ok(entry)
}

struct Lower<'a, 'b> {
    module: &'a mut JITModule,
    body: FunctionBuilder<'b>,
    pointer: Type,
    runtime: &'a Runtime,
    ids: &'a [FuncId],
    vars: Vec<Variable>,
    /// Counted locals that are not parameters, released on return.
    owned: Vec<Variable>,
    /// With `owned`, every return goes here, so each release appears once.
    exit: Option<Block>,
    /// The `continue` and `break` targets of the open loops.
    loops: Vec<(Block, Block)>,
    funcs: HashMap<FuncId, FuncRef>,
    /// All static text of this task, in one object: programs have many trap sites.
    data: GlobalValue,
    text: Vec<u8>,
    path: &'a str,
    lines: &'a Lines<'a>,
}

impl Lower<'_, '_> {
    fn task(mut self, task: &Task, data_id: DataId, stack_limit: usize) {
        let entry = self.body.create_block();
        self.body.append_block_params_for_function_params(entry);
        self.body.switch_to_block(entry);
        self.body.seal_block(entry);
        self.vars = task.locals.iter().map(|&ty| self.body.declare_var(clif(ty))).collect();
        for local in 0..task.params {
            let value = self.body.block_params(entry)[local];
            self.body.def_var(self.vars[local], value);
        }
        for local in task.params..task.locals.len() {
            if task.locals[local].is_counted() {
                let empty = self.body.ins().iconst(self.pointer, 0);
                self.body.def_var(self.vars[local], empty);
                self.owned.push(self.vars[local]);
            }
        }
        if !self.owned.is_empty() {
            let exit = self.body.create_block();
            self.body.append_block_param(exit, clif(task.returns));
            self.exit = Some(exit);
        }

        let sp = self.body.ins().get_stack_pointer(self.pointer);
        let limit = self.body.ins().iconst(self.pointer, stack_limit as i64);
        let low = self.body.ins().icmp(IntCC::UnsignedLessThan, sp, limit);
        self.trap_if(low, "stack overflow", task.span);

        if !self.stmts(&task.body) {
            if task.returns == Ty::None {
                let none = self.none();
                self.leave(none);
            } else {
                // The checker proved that every path returns, so this block is dead.
                self.body.ins().trap(UNREACHABLE);
            }
        }
        if let Some(exit) = self.exit {
            self.body.seal_block(exit);
            self.body.switch_to_block(exit);
            for local in 0..self.owned.len() {
                let held = self.body.use_var(self.owned[local]);
                self.call(self.runtime.release, &[held]);
            }
            let value = self.body.block_params(exit)[0];
            self.body.ins().return_(&[value]);
        }
        let target = self.module.target_config();
        self.body.finalize(target);

        // The trailing byte keeps the data non-empty, so its pointer is always valid.
        self.text.push(0);
        let mut description = DataDescription::new();
        description.define(self.text.into_boxed_slice());
        self.module.define_data(data_id, &description).expect(CODEGEN);
    }

    /// Ends the task with `value`, which is already owned or is not counted.
    fn leave(&mut self, value: Value) {
        match self.exit {
            Some(exit) => {
                self.body.ins().jump(exit, &[value.into()]);
            }
            None => {
                self.body.ins().return_(&[value]);
            }
        }
    }

    /// Replaces the value of a counted local with `value`, which it owns.
    fn store(&mut self, local: Local, value: Value) {
        let old = self.body.use_var(self.vars[local]);
        self.call(self.runtime.release, &[old]);
        self.body.def_var(self.vars[local], value);
    }

    fn none(&mut self) -> Value {
        self.body.ins().iconst(types::I8, 0)
    }

    fn call(&mut self, id: FuncId, args: &[Value]) -> Option<Value> {
        let func = match self.funcs.get(&id) {
            Some(&func) => func,
            None => {
                let func = self.module.declare_func_in_func(id, self.body.func);
                *self.funcs.entry(id).or_insert(func)
            }
        };
        let call = self.body.ins().call(func, args);
        self.body.inst_results(call).first().copied()
    }

    /// Returns the address and length of `text`, kept in static data.
    fn text(&mut self, text: &str) -> [Value; 2] {
        let offset = self.text.len() as i64;
        self.text.extend_from_slice(text.as_bytes());
        let base = self.body.ins().symbol_value(self.pointer, self.data);
        let address = self.body.ins().iadd_imm_s(base, offset);
        let len = self.body.ins().iconst(types::I64, text.len() as i64);
        [address, len]
    }

    /// Ends the current block with a trap at `span`.
    fn trap(&mut self, message: &str, span: Span) {
        let (line, column) = self.lines.position(span.start);
        let text = self.text(&format!("{}:{line}:{column}: trap: {message}", self.path));
        self.call(self.runtime.trap, &text);
        self.body.ins().trap(UNREACHABLE);
    }

    /// Trap paths are cold, to keep reporting code off the hot path.
    fn trap_if(&mut self, condition: Value, message: &str, span: Span) {
        let trap = self.body.create_block();
        let next = self.body.create_block();
        self.body.set_cold_block(trap);
        self.body.ins().brif(condition, trap, &[], next, &[]);
        self.body.seal_block(trap);
        self.body.seal_block(next);
        self.body.switch_to_block(trap);
        self.trap(message, span);
        self.body.switch_to_block(next);
    }

    /// Lowers statements, and returns true if the block is already terminated.
    /// Later statements cannot run, and a block cannot hold them.
    fn stmts(&mut self, stmts: &[Stmt]) -> bool {
        stmts.iter().any(|stmt| self.stmt(stmt))
    }

    fn stmt(&mut self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Set(local, value) => {
                let counted = value.ty.is_counted();
                let value = self.expr(value);
                if counted {
                    self.call(self.runtime.retain, &[value]);
                    self.store(*local, value);
                } else {
                    self.body.def_var(self.vars[*local], value);
                }
            }
            Stmt::Block(stmts) => return self.stmts(stmts),
            Stmt::If { branches, otherwise } => {
                let done = self.body.create_block();
                for (condition, then) in branches {
                    let condition = self.expr(condition);
                    let then_block = self.body.create_block();
                    let next = self.body.create_block();
                    self.body.ins().brif(condition, then_block, &[], next, &[]);
                    self.body.seal_block(then_block);
                    self.body.seal_block(next);
                    self.body.switch_to_block(then_block);
                    if !self.stmts(then) {
                        self.body.ins().jump(done, &[]);
                    }
                    self.body.switch_to_block(next);
                }
                if !self.stmts(otherwise) {
                    self.body.ins().jump(done, &[]);
                }
                self.body.seal_block(done);
                self.body.switch_to_block(done);
            }
            Stmt::While { condition, body } => {
                let head = self.body.create_block();
                let inside = self.body.create_block();
                let done = self.body.create_block();
                self.body.ins().jump(head, &[]);
                self.body.switch_to_block(head);
                let condition = self.expr(condition);
                self.body.ins().brif(condition, inside, &[], done, &[]);
                self.body.seal_block(inside);
                self.body.switch_to_block(inside);
                self.loops.push((head, done));
                if !self.stmts(body) {
                    self.body.ins().jump(head, &[]);
                }
                self.loops.pop();
                self.body.seal_block(head);
                self.body.seal_block(done);
                self.body.switch_to_block(done);
            }
            Stmt::For { local, start, end, inclusive, body } => {
                self.for_loop(*local, start, end, *inclusive, body);
            }
            Stmt::Return(value) => {
                let counted = value.ty.is_counted();
                let value = self.expr(value);
                if counted {
                    self.call(self.runtime.retain, &[value]);
                }
                self.leave(value);
                return true;
            }
            Stmt::Break => {
                let (_, done) = *self.loops.last().expect("the checker found a loop");
                self.body.ins().jump(done, &[]);
                return true;
            }
            Stmt::Continue => {
                let (next, _) = *self.loops.last().expect("the checker found a loop");
                self.body.ins().jump(next, &[]);
                return true;
            }
            Stmt::Print(value) => self.print(value),
            Stmt::PrintText(text) => self.print_text(text),
            Stmt::Placeholder(span) => {
                self.trap("reached a `...` placeholder", *span);
                return true;
            }
            Stmt::Expr(value) => {
                self.expr(value);
            }
        }
        false
    }

    /// The bounds are read once. The step leaves the loop at `end` before it
    /// adds, so `0..=MAX` ends instead of overflowing.
    fn for_loop(&mut self, local: Local, start: &Expr, end: &Expr, inclusive: bool, body: &[Stmt]) {
        let var = self.vars[local];
        let start = self.expr(start);
        let end = self.expr(end);
        self.body.def_var(var, start);
        let head = self.body.create_block();
        let inside = self.body.create_block();
        let step = self.body.create_block();
        let next = self.body.create_block();
        let done = self.body.create_block();
        self.body.ins().jump(head, &[]);

        self.body.switch_to_block(head);
        let i = self.body.use_var(var);
        let cc = if inclusive { IntCC::SignedLessThanOrEqual } else { IntCC::SignedLessThan };
        let enter = self.body.ins().icmp(cc, i, end);
        self.body.ins().brif(enter, inside, &[], done, &[]);
        self.body.seal_block(inside);

        self.body.switch_to_block(inside);
        self.loops.push((step, done));
        if !self.stmts(body) {
            self.body.ins().jump(step, &[]);
        }
        self.loops.pop();
        self.body.seal_block(step);

        self.body.switch_to_block(step);
        let i = self.body.use_var(var);
        let last = self.body.ins().icmp(IntCC::Equal, i, end);
        self.body.ins().brif(last, done, &[], next, &[]);
        self.body.seal_block(next);

        self.body.switch_to_block(next);
        let i = self.body.ins().iadd_imm_s(i, 1);
        self.body.def_var(var, i);
        self.body.ins().jump(head, &[]);
        self.body.seal_block(head);
        self.body.seal_block(done);
        self.body.switch_to_block(done);
    }

    fn print(&mut self, expr: &Expr) {
        let value = self.expr(expr);
        match expr.ty {
            Ty::Int => self.call(self.runtime.print_int, &[value]),
            Ty::Float => self.call(self.runtime.print_float, &[value]),
            Ty::Bool => self.call(self.runtime.print_bool, &[value]),
            Ty::Text => self.call(self.runtime.text_print, &[value]),
            Ty::None => {
                self.print_text("none");
                None
            }
            Ty::Error => unreachable!("errors stop before the backend"),
        };
    }

    fn print_text(&mut self, text: &str) {
        let text = self.text(text);
        self.call(self.runtime.print, &text);
    }

    fn expr(&mut self, expr: &Expr) -> Value {
        match &expr.kind {
            ExprKind::Int(value) => self.body.ins().iconst(types::I64, *value),
            ExprKind::Float(value) => self.body.ins().f64const(*value),
            ExprKind::Bool(value) => self.body.ins().iconst(types::I8, i64::from(*value)),
            ExprKind::None => self.none(),
            ExprKind::Local(local) => self.body.use_var(self.vars[*local]),
            ExprKind::Call(task, args) => {
                let args: Vec<Value> = args.iter().map(|arg| self.expr(arg)).collect();
                self.call(self.ids[*task], &args).expect("every task returns a value")
            }
            ExprKind::Neg(value) => {
                let value = self.expr(value);
                self.body.ins().fneg(value)
            }
            ExprKind::Not(value) => {
                let value = self.expr(value);
                self.body.ins().bxor_imm_s(value, 1)
            }
            ExprKind::Binary(op, left, right) => self.binary(*op, left, right, expr.span),
            ExprKind::If(condition, then, otherwise) => {
                self.if_value(condition, then, otherwise, clif(expr.ty))
            }
            ExprKind::ToFloat(value) => {
                let value = self.expr(value);
                self.body.ins().fcvt_from_sint(types::F64, value)
            }
            ExprKind::ToInt(value) => {
                let value = self.expr(value);
                let low = self.body.ins().f64const(-(2f64.powi(63)));
                let high = self.body.ins().f64const(2f64.powi(63));
                // Both comparisons are false for NaN.
                let above = self.body.ins().fcmp(FloatCC::GreaterThanOrEqual, value, low);
                let below = self.body.ins().fcmp(FloatCC::LessThan, value, high);
                let fits = self.body.ins().band(above, below);
                let outside = self.body.ins().icmp_imm_s(IntCC::Equal, fits, 0);
                self.trap_if(outside, "float does not fit in `int`", expr.span);
                self.body.ins().fcvt_to_sint(types::I64, value)
            }
            ExprKind::Text(text) => {
                let text = self.text(text);
                self.call(self.runtime.text_new, &text).expect("it returns a text")
            }
            ExprKind::ToText(value) => self.text_of(value),
            ExprKind::Join(parts) => self.join(parts),
            ExprKind::TextLen(value) => {
                let value = self.expr(value);
                self.call(self.runtime.text_len, &[value]).expect("it returns an int")
            }
            ExprKind::TextIsEmpty(value) => {
                let value = self.expr(value);
                self.call(self.runtime.text_is_empty, &[value]).expect("it returns a bool")
            }
            ExprKind::TextSlice(text, start, end) => {
                let text = self.expr(text);
                let start = self.expr(start);
                let end = self.expr(end);
                let slice = self.call(self.runtime.text_slice, &[text, start, end]);
                let slice = slice.expect("it returns a text");
                let outside = self.body.ins().icmp_imm_s(IntCC::Equal, slice, 0);
                self.trap_if(outside, "text slice out of bounds", expr.span);
                slice
            }
            ExprKind::TextByteLen(value) => {
                let value = self.expr(value);
                self.call(self.runtime.text_byte_len, &[value]).expect("it returns an int")
            }
            ExprKind::TextCharAt(text, offset) => {
                let text = self.expr(text);
                let offset = self.expr(offset);
                self.call(self.runtime.text_char_at, &[text, offset]).expect("it returns a text")
            }
            ExprKind::Temp(local, value) => {
                let value = self.expr(value);
                self.store(*local, value);
                value
            }
        }
    }

    fn text_of(&mut self, expr: &Expr) -> Value {
        let value = self.expr(expr);
        let text = match expr.ty {
            Ty::Int => self.call(self.runtime.int_text, &[value]),
            Ty::Float => self.call(self.runtime.float_text, &[value]),
            Ty::Bool => self.call(self.runtime.bool_text, &[value]),
            Ty::None => {
                let text = self.text("none");
                self.call(self.runtime.text_new, &text)
            }
            _ => unreachable!("the checker converts only these types"),
        };
        text.expect("it returns a text")
    }

    /// The parts go in a stack slot, which E0902 counts as part of the frame.
    fn join(&mut self, parts: &[Expr]) -> Value {
        let size = u32::try_from(parts.len() * 8).expect("the source fits in memory");
        let slot = StackSlotData::new(StackSlotKind::ExplicitSlot, size, 3);
        let slot = self.body.create_sized_stack_slot(slot);
        for (index, part) in parts.iter().enumerate() {
            let value = self.expr(part);
            self.body.ins().stack_store(self.pointer, value, slot, index as i32 * 8);
        }
        let address = self.body.ins().stack_addr(self.pointer, slot, 0);
        let count = self.body.ins().iconst(types::I64, parts.len() as i64);
        self.call(self.runtime.text_join, &[address, count]).expect("it returns a text")
    }

    fn if_value(&mut self, condition: &Expr, then: &Expr, otherwise: &Expr, ty: Type) -> Value {
        let condition = self.expr(condition);
        let then_block = self.body.create_block();
        let else_block = self.body.create_block();
        let done = self.body.create_block();
        let result = self.body.append_block_param(done, ty);
        self.body.ins().brif(condition, then_block, &[], else_block, &[]);
        self.body.seal_block(then_block);
        self.body.seal_block(else_block);
        for (block, value) in [(then_block, then), (else_block, otherwise)] {
            self.body.switch_to_block(block);
            let value = self.expr(value);
            self.body.ins().jump(done, &[value.into()]);
        }
        self.body.seal_block(done);
        self.body.switch_to_block(done);
        result
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, right: &Expr, span: Span) -> Value {
        let float = left.ty == Ty::Float;
        let a = self.expr(left);
        let b = self.expr(right);
        if left.ty == Ty::Text {
            let cc = int_cc(op).expect("the checker allows only comparisons on text");
            let order = self.call(self.runtime.text_compare, &[a, b]);
            return self.body.ins().icmp_imm_s(cc, order.expect("it returns an int"), 0);
        }
        if let Some(cc) = int_cc(op) {
            let ins = self.body.ins();
            return if float { ins.fcmp(float_cc(cc), a, b) } else { ins.icmp(cc, a, b) };
        }
        match (op, float) {
            (BinaryOp::Add, true) => self.body.ins().fadd(a, b),
            (BinaryOp::Sub, true) => self.body.ins().fsub(a, b),
            (BinaryOp::Mul, true) => self.body.ins().fmul(a, b),
            (BinaryOp::Div, true) => self.body.ins().fdiv(a, b),
            (BinaryOp::Add, false) if !CHECK_OVERFLOW => self.body.ins().iadd(a, b),
            (BinaryOp::Sub, false) if !CHECK_OVERFLOW => self.body.ins().isub(a, b),
            (BinaryOp::Mul, false) if !CHECK_OVERFLOW => self.body.ins().imul(a, b),
            (BinaryOp::Add, false) => {
                let (value, overflow) = self.body.ins().sadd_overflow(a, b);
                self.checked(value, overflow, span)
            }
            (BinaryOp::Sub, false) => {
                let (value, overflow) = self.body.ins().ssub_overflow(a, b);
                self.checked(value, overflow, span)
            }
            (BinaryOp::Mul, false) => {
                let (value, overflow) = self.body.ins().smul_overflow(a, b);
                self.checked(value, overflow, span)
            }
            (BinaryOp::FloorDiv, false) => self.floor_div(a, b, span),
            (BinaryOp::Rem, false) => self.floor_rem(a, b, span),
            _ => unreachable!("the checker allows only these operators"),
        }
    }

    fn checked(&mut self, value: Value, overflow: Value, span: Span) -> Value {
        self.trap_if(overflow, "integer overflow", span);
        value
    }

    fn check_divisor(&mut self, b: Value, span: Span) {
        let zero = self.body.ins().icmp_imm_s(IntCC::Equal, b, 0);
        self.trap_if(zero, "division by zero", span);
    }

    /// True if a truncated remainder `r` needs correcting for floor rounding:
    /// it is not zero, and its sign differs from the divisor's.
    fn rounded_up(&mut self, r: Value, b: Value) -> Value {
        let nonzero = self.body.ins().icmp_imm_s(IntCC::NotEqual, r, 0);
        let signs = self.body.ins().bxor(r, b);
        let differ = self.body.ins().icmp_imm_s(IntCC::SignedLessThan, signs, 0);
        self.body.ins().band(nonzero, differ)
    }

    fn floor_div(&mut self, a: Value, b: Value, span: Span) -> Value {
        self.check_divisor(b, span);
        let lowest = self.body.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
        let minus_one = self.body.ins().icmp_imm_s(IntCC::Equal, b, -1);
        let overflow = self.body.ins().band(lowest, minus_one);
        self.trap_if(overflow, "integer overflow", span);
        let q = self.body.ins().sdiv(a, b);
        let product = self.body.ins().imul(q, b);
        let r = self.body.ins().isub(a, product);
        let round = self.rounded_up(r, b);
        let round = self.body.ins().uextend(types::I64, round);
        self.body.ins().isub(q, round)
    }

    fn floor_rem(&mut self, a: Value, b: Value, span: Span) -> Value {
        self.check_divisor(b, span);
        let r = self.body.ins().srem(a, b);
        let round = self.rounded_up(r, b);
        let corrected = self.body.ins().iadd(r, b);
        self.body.ins().select(round, corrected, r)
    }
}

fn int_cc(op: BinaryOp) -> Option<IntCC> {
    Some(match op {
        BinaryOp::Eq => IntCC::Equal,
        BinaryOp::Ne => IntCC::NotEqual,
        BinaryOp::Lt => IntCC::SignedLessThan,
        BinaryOp::Le => IntCC::SignedLessThanOrEqual,
        BinaryOp::Gt => IntCC::SignedGreaterThan,
        BinaryOp::Ge => IntCC::SignedGreaterThanOrEqual,
        _ => return None,
    })
}

fn float_cc(cc: IntCC) -> FloatCC {
    match cc {
        IntCC::Equal => FloatCC::Equal,
        IntCC::NotEqual => FloatCC::NotEqual,
        IntCC::SignedLessThan => FloatCC::LessThan,
        IntCC::SignedLessThanOrEqual => FloatCC::LessThanOrEqual,
        IntCC::SignedGreaterThan => FloatCC::GreaterThan,
        _ => FloatCC::GreaterThanOrEqual,
    }
}
