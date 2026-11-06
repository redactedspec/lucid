//! Cranelift backend. Lowers MIR to machine code, and runs it in this process.

use crate::ast::BinaryOp;
use crate::diag::{Diagnostic, Lines, Span};
use crate::mir::{Expr, ExprKind, HashOp, KeyTasks, Local, Program, Stmt, Task, Ty, Update};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block, FuncRef, GlobalValue, InstBuilder, MemFlagsData, Signature, StackSlotData,
    StackSlotKind, TrapCode, Type, Value, types,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, default_libcall_names};
use std::collections::HashMap;

const CODEGEN: &str = "code generation failed";
const OUT_OF_BOUNDS: &str = "list index out of bounds";

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
        Ty::Int
        | Ty::Byte
        | Ty::Text
        | Ty::List(_)
        | Ty::Map(..)
        | Ty::Set(_)
        | Ty::Data(..)
        | Ty::Union(_) => types::I64,
        Ty::Float => types::F64,
        Ty::Bool | Ty::None => types::I8,
        Ty::Param(..) => unreachable!("`mono.rs` replaces every parameter"),
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
    text_bytes: FuncId,
    text_quote: FuncId,
    list_new: FuncId,
    list_push: FuncId,
    list_set: FuncId,
    list_pop: FuncId,
    list_swap: FuncId,
    list_has: FuncId,
    list_slice: FuncId,
    map_new: FuncId,
    map_set: FuncId,
    map_remove: FuncId,
    map_find: FuncId,
    map_next: FuncId,
    map_key: FuncId,
    set_from: FuncId,
    hash_int: FuncId,
    hash_text: FuncId,
    hash_sum: FuncId,
    record_new: FuncId,
    record_set: FuncId,
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
    builder.symbol("lucid_text_bytes", lucid_rt::lucid_text_bytes as *const u8);
    builder.symbol("lucid_text_quote", lucid_rt::lucid_text_quote as *const u8);
    builder.symbol("lucid_list_new", lucid_rt::lucid_list_new as *const u8);
    builder.symbol("lucid_list_push", lucid_rt::lucid_list_push as *const u8);
    builder.symbol("lucid_list_set", lucid_rt::lucid_list_set as *const u8);
    builder.symbol("lucid_list_pop", lucid_rt::lucid_list_pop as *const u8);
    builder.symbol("lucid_list_swap", lucid_rt::lucid_list_swap as *const u8);
    builder.symbol("lucid_list_has", lucid_rt::lucid_list_has as *const u8);
    builder.symbol("lucid_list_slice", lucid_rt::lucid_list_slice as *const u8);
    builder.symbol("lucid_map_new", lucid_rt::lucid_map_new as *const u8);
    builder.symbol("lucid_map_set", lucid_rt::lucid_map_set as *const u8);
    builder.symbol("lucid_map_remove", lucid_rt::lucid_map_remove as *const u8);
    builder.symbol("lucid_map_find", lucid_rt::lucid_map_find as *const u8);
    builder.symbol("lucid_map_next", lucid_rt::lucid_map_next as *const u8);
    builder.symbol("lucid_map_key", lucid_rt::lucid_map_key as *const u8);
    builder.symbol("lucid_set_from", lucid_rt::lucid_set_from as *const u8);
    builder.symbol("lucid_hash_int", lucid_rt::lucid_hash_int as *const u8);
    builder.symbol("lucid_hash_text", lucid_rt::lucid_hash_text as *const u8);
    builder.symbol("lucid_hash_sum", lucid_rt::lucid_hash_sum as *const u8);
    builder.symbol("lucid_record_new", lucid_rt::lucid_record_new as *const u8);
    builder.symbol("lucid_record_set", lucid_rt::lucid_record_set as *const u8);
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
    let with_two = [value[0], int[0], int[0]];
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
        text_slice: import("lucid_text_slice", &with_two, &value),
        text_byte_len: import("lucid_text_byte_len", &value, &int),
        text_char_at: import("lucid_text_char_at", &[value[0], int[0]], &value),
        text_bytes: import("lucid_text_bytes", &value, &value),
        text_quote: import("lucid_text_quote", &value, &value),
        list_new: import("lucid_list_new", &[int[0], byte[0]], &value),
        list_push: import("lucid_list_push", &two, &value),
        list_set: import("lucid_list_set", &with_two, &value),
        list_pop: import("lucid_list_pop", &value, &value),
        list_swap: import("lucid_list_swap", &with_two, &value),
        list_has: import("lucid_list_has", &two, &[AbiParam::new(types::I8)]),
        list_slice: import("lucid_list_slice", &with_two, &value),
        map_new: import("lucid_map_new", &[byte[0], byte[0], value[0], value[0]], &value),
        map_set: import("lucid_map_set", &with_two, &value),
        map_remove: import("lucid_map_remove", &two, &value),
        map_find: import("lucid_map_find", &two, &value),
        map_next: import("lucid_map_next", &two, &int),
        map_key: import("lucid_map_key", &two, &int),
        set_from: import("lucid_set_from", &with_two, &value),
        hash_int: import("lucid_hash_int", &[int[0], int[0]], &int),
        hash_text: import("lucid_hash_text", &[int[0], value[0]], &int),
        hash_sum: import("lucid_hash_sum", &[int[0], int[0]], &int),
        record_new: import("lucid_record_new", &[int[0], int[0], int[0]], &value),
        record_set: import("lucid_record_set", &with_two, &value),
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

    fn func(&mut self, id: FuncId) -> FuncRef {
        match self.funcs.get(&id) {
            Some(&func) => func,
            None => {
                let func = self.module.declare_func_in_func(id, self.body.func);
                *self.funcs.entry(id).or_insert(func)
            }
        }
    }

    fn call(&mut self, id: FuncId, args: &[Value]) -> Option<Value> {
        let func = self.func(id);
        let call = self.body.ins().call(func, args);
        self.body.inst_results(call).first().copied()
    }

    /// The addresses of the tasks that a map keeps, or null for each.
    fn key_tasks(&mut self, tasks: KeyTasks) -> [Value; 2] {
        let Some((hash, equal)) = tasks else {
            let null = self.body.ins().iconst(self.pointer, 0);
            return [null, null];
        };
        [hash, equal].map(|task| {
            let func = self.func(self.ids[task]);
            self.body.ins().func_addr(self.pointer, func)
        })
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
            Stmt::Update { local, op, args, span } => self.update(*local, *op, args, *span),
            Stmt::Clear(local) => {
                let empty = self.body.ins().iconst(self.pointer, 0);
                self.store(*local, empty);
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
            Ty::Int | Ty::Byte => self.call(self.runtime.print_int, &[value]),
            Ty::Float => self.call(self.runtime.print_float, &[value]),
            Ty::Bool => self.call(self.runtime.print_bool, &[value]),
            Ty::Text => self.call(self.runtime.text_print, &[value]),
            Ty::None => {
                self.print_text("none");
                None
            }
            _ => unreachable!("the checker rejects printing any other type"),
        };
    }

    /// Lowers a value to the 8-byte form that every runtime slot has.
    fn slot(&mut self, expr: &Expr) -> Value {
        let value = self.expr(expr);
        match expr.ty {
            Ty::Float => self.body.ins().bitcast(types::I64, MemFlagsData::new(), value),
            Ty::Bool | Ty::None => self.body.ins().uextend(types::I64, value),
            _ => value,
        }
    }

    /// The reverse of `slot`.
    fn read_slot(&mut self, slot: Value, ty: Ty) -> Value {
        match ty {
            Ty::Float => self.body.ins().bitcast(types::F64, MemFlagsData::new(), slot),
            Ty::Bool | Ty::None => self.body.ins().ireduce(types::I8, slot),
            _ => slot,
        }
    }

    fn update(&mut self, local: Local, op: Update, args: &[Expr], span: Span) {
        let mut values = vec![self.body.use_var(self.vars[local])];
        for arg in args {
            let value = self.slot(arg);
            values.push(value);
        }
        let runtime = self.runtime;
        let task = match op {
            Update::ListPush => runtime.list_push,
            Update::ListSet => runtime.list_set,
            Update::ListSwap => runtime.list_swap,
            Update::MapSet => runtime.map_set,
            Update::MapRemove => runtime.map_remove,
            Update::FieldSet => runtime.record_set,
            Update::UnionSet => unreachable!("`mono.rs` replaces these"),
            Update::ListPop => runtime.list_pop,
        };
        let result = self.call(task, &values).expect("it returns the new value");
        if matches!(op, Update::ListSet | Update::ListSwap) {
            let outside = self.body.ins().icmp_imm_s(IntCC::Equal, result, 0);
            self.trap_if(outside, OUT_OF_BOUNDS, span);
        }
        self.body.def_var(self.vars[local], result);
    }

    fn list_of(&mut self, items: &[Expr], item: Ty) -> Value {
        let cap = self.body.ins().iconst(types::I64, items.len() as i64);
        let counted = self.body.ins().iconst(types::I8, i64::from(item.is_counted()));
        let mut list =
            self.call(self.runtime.list_new, &[cap, counted]).expect("it returns a list");
        for item in items {
            let value = self.slot(item);
            let pushed = self.call(self.runtime.list_push, &[list, value]);
            list = pushed.expect("it returns a list");
        }
        list
    }

    fn map_of(&mut self, entries: &[(Expr, Expr)], key: Ty, value: Ty, tasks: KeyTasks) -> Value {
        let keys = self.body.ins().iconst(types::I8, i64::from(key.is_counted()));
        let values = self.body.ins().iconst(types::I8, i64::from(value.is_counted()));
        let [hash, equal] = self.key_tasks(tasks);
        let new = self.call(self.runtime.map_new, &[keys, values, hash, equal]);
        let mut map = new.expect("it returns a map");
        for (key, value) in entries {
            let key = self.slot(key);
            let value = self.slot(value);
            let set = self.call(self.runtime.map_set, &[map, key, value]);
            map = set.expect("it returns a map");
        }
        map
    }

    /// Every value runs before the record exists, so that no field sees it.
    fn new_record(&mut self, tag: usize, counted: usize, fields: &[(usize, Expr)]) -> Value {
        let values: Vec<(usize, Value)> =
            fields.iter().map(|(slot, value)| (*slot, self.slot(value))).collect();
        let tag = self.body.ins().iconst(types::I64, tag as i64);
        let len = self.body.ins().iconst(types::I64, fields.len() as i64);
        let held = self.body.ins().iconst(types::I64, counted as i64);
        let record = self.call(self.runtime.record_new, &[tag, len, held]);
        let record = record.expect("it returns a record");
        for (slot, value) in values {
            if slot < counted {
                self.call(self.runtime.retain, &[value]);
            }
            let flags = MemFlagsData::trusted();
            self.body.ins().store(flags, value, record, slot_offset(slot));
        }
        record
    }

    fn load_slot(&mut self, address: Value, offset: i32, ty: Ty) -> Value {
        let flags = MemFlagsData::trusted();
        let slot = self.body.ins().load(types::I64, flags, address, offset);
        self.read_slot(slot, ty)
    }

    /// The bounds check is inline: the unsigned compare also rejects a negative.
    fn list_get(&mut self, list: &Expr, index: &Expr, ty: Ty, span: Span) -> Value {
        let list = self.expr(list);
        let index = self.expr(index);
        let flags = MemFlagsData::trusted();
        let len = self.body.ins().load(types::I64, flags, list, lucid_rt::LEN_OFFSET);
        let outside = self.body.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index, len);
        self.trap_if(outside, OUT_OF_BOUNDS, span);
        let offset = self.body.ins().ishl_imm_s(index, 3);
        let address = self.body.ins().iadd(list, offset);
        self.load_slot(address, lucid_rt::SLOTS_OFFSET, ty)
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
            ExprKind::Call(task, _, args) => {
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
                let task = self.runtime.text_slice;
                self.slice(task, [text, start, end], "text slice out of bounds", expr.span)
            }
            ExprKind::ListSlice(list, start, end) => {
                let task = self.runtime.list_slice;
                self.slice(task, [list, start, end], "list slice out of bounds", expr.span)
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
            ExprKind::TextBytes(value) => {
                let value = self.expr(value);
                self.call(self.runtime.text_bytes, &[value]).expect("it returns a list")
            }
            ExprKind::TextQuote(value) => {
                let value = self.expr(value);
                self.call(self.runtime.text_quote, &[value]).expect("it returns a text")
            }
            ExprKind::ListOf(items) => {
                let Ty::List(item) = expr.ty else { unreachable!("it has a list type") };
                self.list_of(items, *item)
            }
            ExprKind::MapOf(entries, tasks) => {
                let Ty::Map(key, value) = expr.ty else { unreachable!("it has a map type") };
                self.map_of(entries, *key, *value, *tasks)
            }
            ExprKind::SetOf(list, tasks) => {
                let list = self.expr(list);
                let [hash, equal] = self.key_tasks(*tasks);
                self.call(self.runtime.set_from, &[list, hash, equal]).expect("it returns a set")
            }
            ExprKind::Hash(op, state, value) => {
                let state = self.expr(state);
                let value = self.slot(value);
                let task = match op {
                    HashOp::Int => self.runtime.hash_int,
                    HashOp::Text => self.runtime.hash_text,
                    HashOp::Sum => self.runtime.hash_sum,
                };
                self.call(task, &[state, value]).expect("it returns a hash")
            }
            ExprKind::Len(value) => {
                let value = self.expr(value);
                let flags = MemFlagsData::trusted();
                self.body.ins().load(types::I64, flags, value, lucid_rt::LEN_OFFSET)
            }
            ExprKind::ListGet(list, index) => self.list_get(list, index, expr.ty, expr.span),
            ExprKind::ListHas(value, list) => {
                let value = self.slot(value);
                let list = self.expr(list);
                self.call(self.runtime.list_has, &[list, value]).expect("it returns a bool")
            }
            ExprKind::MapGet(map, key) => {
                let map = self.expr(map);
                let key = self.slot(key);
                let found = self.call(self.runtime.map_find, &[map, key]);
                let found = found.expect("it returns an address");
                let missing = self.body.ins().icmp_imm_s(IntCC::Equal, found, 0);
                self.trap_if(missing, "map key not found", expr.span);
                self.load_slot(found, 0, expr.ty)
            }
            ExprKind::MapHas(key, map) => {
                let key = self.slot(key);
                let map = self.expr(map);
                let found = self.call(self.runtime.map_find, &[map, key]);
                self.body.ins().icmp_imm_s(
                    IntCC::NotEqual,
                    found.expect("it returns an address"),
                    0,
                )
            }
            ExprKind::MapNext(map, position) => {
                let map = self.expr(map);
                let position = self.expr(position);
                self.call(self.runtime.map_next, &[map, position]).expect("it returns an int")
            }
            ExprKind::MapKey(map, position) => {
                let map = self.expr(map);
                let position = self.expr(position);
                let key = self.call(self.runtime.map_key, &[map, position]);
                self.read_slot(key.expect("it returns a key"), expr.ty)
            }
            ExprKind::New { tag, counted, fields } => self.new_record(*tag, *counted, fields),
            ExprKind::Field(value, slot) => {
                let value = self.expr(value);
                self.load_slot(value, slot_offset(*slot), expr.ty)
            }
            ExprKind::Tag(value) => {
                let value = self.expr(value);
                let flags = MemFlagsData::trusted();
                self.body.ins().load(types::I64, flags, value, lucid_rt::TAG_OFFSET)
            }
            ExprKind::TagIn(value, tags) => {
                let value = self.expr(value);
                let flags = MemFlagsData::trusted();
                let tag = self.body.ins().load(types::I64, flags, value, lucid_rt::TAG_OFFSET);
                let mut found = self.body.ins().iconst(types::I8, 0);
                for &wanted in tags {
                    let same = self.body.ins().icmp_imm_s(IntCC::Equal, tag, wanted as i64);
                    found = self.body.ins().bor(found, same);
                }
                found
            }
            ExprKind::Seq(setup, value) => {
                self.stmts(setup);
                self.expr(value)
            }
            ExprKind::Temp(local, value) => {
                let value = self.expr(value);
                self.store(*local, value);
                value
            }
            ExprKind::Member { .. }
            | ExprKind::Wrap(_)
            | ExprKind::View(_)
            | ExprKind::Holds(..)
            | ExprKind::Method(..) => unreachable!("`mono.rs` replaces these"),
        }
    }

    /// Traps at `span` if `task` gives null for the range.
    fn slice(&mut self, task: FuncId, args: [&Expr; 3], message: &str, span: Span) -> Value {
        let args = args.map(|arg| self.expr(arg));
        let slice = self.call(task, &args).expect("it returns a value");
        let outside = self.body.ins().icmp_imm_s(IntCC::Equal, slice, 0);
        self.trap_if(outside, message, span);
        slice
    }

    fn text_of(&mut self, expr: &Expr) -> Value {
        let value = self.expr(expr);
        let text = match expr.ty {
            Ty::Int | Ty::Byte => self.call(self.runtime.int_text, &[value]),
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

fn slot_offset(slot: usize) -> i32 {
    let offset = slot * size_of::<i64>() + lucid_rt::RECORD_SLOTS_OFFSET as usize;
    i32::try_from(offset).expect("the source fits in memory")
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
