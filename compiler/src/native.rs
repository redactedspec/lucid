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

/// Compiled code changes counts itself, except in a debug build of `lucid_rt`,
/// where its calls find a use after free.
const INLINE_COUNTS: bool = !cfg!(debug_assertions);

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
        | Ty::I8
        | Ty::I16
        | Ty::I32
        | Ty::U16
        | Ty::U32
        | Ty::U64
        | Ty::Text
        | Ty::List(_)
        | Ty::Map(..)
        | Ty::Set(_)
        | Ty::Data(..)
        | Ty::Union(_)
        | Ty::Any(_)
        | Ty::Task(..) => types::I64,
        Ty::Float => types::F64,
        Ty::F32 => types::F32,
        Ty::Bool | Ty::None => types::I8,
        Ty::Param(..) => unreachable!("`mono.rs` replaces every parameter"),
        Ty::Error => unreachable!("errors stop before the backend"),
    }
}

struct Runtime {
    print: FuncId,
    print_int: FuncId,
    print_uint: FuncId,
    print_float: FuncId,
    print_f32: FuncId,
    print_bool: FuncId,
    trap: FuncId,
    retain: FuncId,
    release: FuncId,
    text_new: FuncId,
    text_join: FuncId,
    text_append: FuncId,
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
    map_value: FuncId,
    map_position: FuncId,
    set_from: FuncId,
    hash_int: FuncId,
    hash_text: FuncId,
    hash_sum: FuncId,
    record_new: FuncId,
    record_set: FuncId,
    int_text: FuncId,
    uint_text: FuncId,
    float_text: FuncId,
    f32_text: FuncId,
    bool_text: FuncId,
}

fn signature_of(module: &JITModule, task: &Task) -> Signature {
    let mut signature = module.make_signature();
    for (local, &ty) in task.locals[..task.params].iter().enumerate() {
        let pointer = module.target_config().pointer_type();
        let ty = if task.change.contains(&local) { pointer } else { clif(ty) };
        signature.params.push(AbiParam::new(ty));
    }
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
    builder.symbol("lucid_print_uint", lucid_rt::lucid_print_uint as *const u8);
    builder.symbol("lucid_print_float", lucid_rt::lucid_print_float as *const u8);
    builder.symbol("lucid_print_f32", lucid_rt::lucid_print_f32 as *const u8);
    builder.symbol("lucid_print_bool", lucid_rt::lucid_print_bool as *const u8);
    builder.symbol("lucid_trap", lucid_rt::lucid_trap as *const u8);
    builder.symbol("lucid_retain", lucid_rt::lucid_retain as *const u8);
    builder.symbol("lucid_release", lucid_rt::lucid_release as *const u8);
    builder.symbol("lucid_text_new", lucid_rt::lucid_text_new as *const u8);
    builder.symbol("lucid_text_join", lucid_rt::lucid_text_join as *const u8);
    builder.symbol("lucid_text_append", lucid_rt::lucid_text_append as *const u8);
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
    builder.symbol("lucid_map_value", lucid_rt::lucid_map_value as *const u8);
    builder.symbol("lucid_map_position", lucid_rt::lucid_map_position as *const u8);
    builder.symbol("lucid_set_from", lucid_rt::lucid_set_from as *const u8);
    builder.symbol("lucid_hash_int", lucid_rt::lucid_hash_int as *const u8);
    builder.symbol("lucid_hash_text", lucid_rt::lucid_hash_text as *const u8);
    builder.symbol("lucid_hash_sum", lucid_rt::lucid_hash_sum as *const u8);
    builder.symbol("lucid_record_new", lucid_rt::lucid_record_new as *const u8);
    builder.symbol("lucid_record_set", lucid_rt::lucid_record_set as *const u8);
    builder.symbol("lucid_int_text", lucid_rt::lucid_int_text as *const u8);
    builder.symbol("lucid_uint_text", lucid_rt::lucid_uint_text as *const u8);
    builder.symbol("lucid_float_text", lucid_rt::lucid_float_text as *const u8);
    builder.symbol("lucid_f32_text", lucid_rt::lucid_f32_text as *const u8);
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
    let small_float = [AbiParam::new(types::F32)];
    let two = [value[0], value[0]];
    let with_two = [value[0], int[0], int[0]];
    let runtime = Runtime {
        print: import("lucid_print", &text, &[]),
        print_int: import("lucid_print_int", &int, &[]),
        print_uint: import("lucid_print_uint", &int, &[]),
        print_float: import("lucid_print_float", &float, &[]),
        print_f32: import("lucid_print_f32", &small_float, &[]),
        print_bool: import("lucid_print_bool", &byte, &[]),
        trap: import("lucid_trap", &text, &[]),
        retain: import("lucid_retain", &value, &[]),
        release: import("lucid_release", &value, &[]),
        text_new: import("lucid_text_new", &text, &value),
        text_join: import("lucid_text_join", &text, &value),
        text_append: import("lucid_text_append", &text, &value),
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
        map_value: import("lucid_map_value", &two, &int),
        map_position: import("lucid_map_position", &two, &int),
        set_from: import("lucid_set_from", &with_two, &value),
        hash_int: import("lucid_hash_int", &[int[0], int[0]], &int),
        hash_text: import("lucid_hash_text", &[int[0], value[0]], &int),
        hash_sum: import("lucid_hash_sum", &[int[0], int[0]], &int),
        record_new: import("lucid_record_new", &[int[0], int[0], int[0]], &value),
        record_set: import("lucid_record_set", &with_two, &value),
        int_text: import("lucid_int_text", &int, &value),
        uint_text: import("lucid_uint_text", &int, &value),
        float_text: import("lucid_float_text", &float, &value),
        f32_text: import("lucid_f32_text", &small_float, &value),
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
    let mut statics = HashMap::new();
    let tasks = program.tasks.iter().zip(&ids).map(|(task, id)| (task, *id));
    for (task, id) in tasks.chain([(&program.main, main)]) {
        context.func.signature = signature_of(&module, task);
        let body = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let (module, runtime, ids, lines) = (&mut module, &runtime, &ids, &lines);
        let statics = &mut statics;
        let data_id = module.declare_anonymous_data(false, false).expect(CODEGEN);
        let data = module.declare_data_in_func(data_id, body.func);
        let (vars, loops, funcs, text) = (Vec::new(), Vec::new(), HashMap::new(), Vec::new());
        let (owned, changed, exit) = (Vec::new(), Vec::new(), None);
        Lower {
            module,
            body,
            pointer,
            runtime,
            ids,
            vars,
            types: &task.locals,
            owned,
            changed,
            exit,
            loops,
            funcs,
            data,
            text,
            statics,
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
    /// The type of each local.
    types: &'a [Ty],
    /// Counted locals that are not parameters, released on return.
    owned: Vec<Variable>,
    /// Each `change` parameter, with the address of the caller's stack slot.
    changed: Vec<(Variable, Value)>,
    /// With `owned` or `changed`, every return goes here, so each release and
    /// store appears once.
    exit: Option<Block>,
    /// The `continue` and `break` targets of the open loops.
    loops: Vec<(Block, Block)>,
    funcs: HashMap<FuncId, FuncRef>,
    /// All static text of this task, in one object: programs have many trap sites.
    data: GlobalValue,
    text: Vec<u8>,
    /// Values in static data, each once in the program.
    statics: &'a mut HashMap<Vec<u8>, DataId>,
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
            let mut value = self.body.block_params(entry)[local];
            if task.change.contains(&local) {
                self.changed.push((self.vars[local], value));
                let flags = MemFlagsData::trusted();
                value = self.body.ins().load(clif(task.locals[local]), flags, value, 0);
            }
            self.body.def_var(self.vars[local], value);
        }
        for local in task.params..task.locals.len() {
            if task.locals[local].is_counted() {
                let empty = self.body.ins().iconst(self.pointer, 0);
                self.body.def_var(self.vars[local], empty);
                if task.borrowed.binary_search(&local).is_err() {
                    self.owned.push(self.vars[local]);
                }
            }
        }
        if !self.owned.is_empty() || !self.changed.is_empty() {
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
                self.release(held);
            }
            for index in 0..self.changed.len() {
                let (var, address) = self.changed[index];
                let held = self.body.use_var(var);
                self.body.ins().store(MemFlagsData::trusted(), held, address, 0);
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
        self.release(old);
        self.body.def_var(self.vars[local], value);
    }

    fn retain(&mut self, value: Value) {
        if !INLINE_COUNTS {
            self.call(self.runtime.retain, &[value]);
            return;
        }
        let flags = MemFlagsData::trusted();
        let count = self.body.ins().load(types::I64, flags, value, lucid_rt::COUNT_OFFSET);
        let count = self.body.ins().iadd_imm_s(count, 1);
        self.body.ins().store(flags, count, value, lucid_rt::COUNT_OFFSET);
    }

    /// Gives up a count of `value`, which may be null. Only the last count
    /// calls the runtime.
    fn release(&mut self, value: Value) {
        if !INLINE_COUNTS {
            self.call(self.runtime.release, &[value]);
            return;
        }
        let held = self.body.create_block();
        let shared = self.body.create_block();
        let last = self.body.create_block();
        let done = self.body.create_block();
        self.body.set_cold_block(last);
        self.body.ins().brif(value, held, &[], done, &[]);
        self.body.seal_block(held);
        self.body.switch_to_block(held);
        let flags = MemFlagsData::trusted();
        let count = self.body.ins().load(types::I64, flags, value, lucid_rt::COUNT_OFFSET);
        let more = self.body.ins().icmp_imm_s(IntCC::NotEqual, count, 1);
        self.body.ins().brif(more, shared, &[], last, &[]);
        self.body.seal_block(shared);
        self.body.seal_block(last);
        self.body.switch_to_block(shared);
        let count = self.body.ins().iadd_imm_s(count, -1);
        self.body.ins().store(flags, count, value, lucid_rt::COUNT_OFFSET);
        self.body.ins().jump(done, &[]);
        self.body.switch_to_block(last);
        self.call(self.runtime.release, &[value]);
        self.body.ins().jump(done, &[]);
        self.body.seal_block(done);
        self.body.switch_to_block(done);
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

    /// The address of a value in static data, which counts write to.
    fn static_value(&mut self, image: Vec<u8>) -> Value {
        let id = match self.statics.get(&image) {
            Some(&id) => id,
            None => {
                let id = self.module.declare_anonymous_data(true, false).expect(CODEGEN);
                let mut description = DataDescription::new();
                description.define(image.clone().into_boxed_slice());
                description.set_align(8);
                self.module.define_data(id, &description).expect(CODEGEN);
                self.statics.insert(image, id);
                id
            }
        };
        let data = self.module.declare_data_in_func(id, self.body.func);
        self.body.ins().symbol_value(self.pointer, data)
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
                    self.store(*local, value);
                } else {
                    self.body.def_var(self.vars[*local], value);
                }
            }
            Stmt::Bind(local, value) => {
                let value = self.expr(value);
                self.body.def_var(self.vars[*local], value);
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
                let value = self.expr(value);
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
            Stmt::Unreachable => {
                self.body.ins().trap(UNREACHABLE);
                return true;
            }
            Stmt::Placeholder(span) => {
                self.trap("reached a `...` placeholder", *span);
                return true;
            }
            Stmt::Trap(message, span) => {
                self.trap(message, *span);
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
            Ty::U64 => self.call(self.runtime.print_uint, &[value]),
            ty if ty.is_integer() => self.call(self.runtime.print_int, &[value]),
            Ty::Float => self.call(self.runtime.print_float, &[value]),
            Ty::F32 => self.call(self.runtime.print_f32, &[value]),
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
            Ty::F32 => {
                let bits = self.body.ins().bitcast(types::I32, MemFlagsData::new(), value);
                self.body.ins().uextend(types::I64, bits)
            }
            Ty::Bool | Ty::None => self.body.ins().uextend(types::I64, value),
            _ => value,
        }
    }

    /// The reverse of `slot`.
    fn read_slot(&mut self, slot: Value, ty: Ty) -> Value {
        match ty {
            Ty::Float => self.body.ins().bitcast(types::F64, MemFlagsData::new(), slot),
            Ty::F32 => {
                let bits = self.body.ins().ireduce(types::I32, slot);
                self.body.ins().bitcast(types::F32, MemFlagsData::new(), bits)
            }
            Ty::Bool | Ty::None => self.body.ins().ireduce(types::I8, slot),
            _ => slot,
        }
    }

    /// True if `value` has one count. A debug runtime's dead value has another,
    /// so it reaches the runtime's own check.
    fn is_unique(&mut self, value: Value) -> Value {
        let flags = MemFlagsData::trusted();
        let count = self.body.ins().load(types::I64, flags, value, lucid_rt::COUNT_OFFSET);
        self.body.ins().icmp_imm_s(IntCC::Equal, count, 1)
    }

    /// Runs `fast` if `test` holds, and otherwise the runtime `task`, which
    /// copies. Each gives the value that replaces `local`.
    fn update_paths(
        &mut self,
        local: Local,
        test: Value,
        fast: impl FnOnce(&mut Self) -> Value,
        task: FuncId,
        args: &[Value],
        span: Option<Span>,
    ) {
        let fast_block = self.body.create_block();
        let slow = self.body.create_block();
        let done = self.body.create_block();
        let result = self.body.append_block_param(done, types::I64);
        self.body.set_cold_block(slow);
        self.body.ins().brif(test, fast_block, &[], slow, &[]);
        self.body.seal_block(fast_block);
        self.body.seal_block(slow);
        self.body.switch_to_block(fast_block);
        let kept = fast(self);
        self.body.ins().jump(done, &[kept.into()]);
        self.body.switch_to_block(slow);
        let copy = self.call(task, args).expect("it returns the new value");
        if let Some(span) = span {
            let outside = self.body.ins().icmp_imm_s(IntCC::Equal, copy, 0);
            self.trap_if(outside, OUT_OF_BOUNDS, span);
        }
        self.body.ins().jump(done, &[copy.into()]);
        self.body.seal_block(done);
        self.body.switch_to_block(done);
        self.body.def_var(self.vars[local], result);
    }

    /// Stores `value` in a slot of a value that this code alone holds, and
    /// gives up what the slot held, if it holds a count. The slot of a write
    /// through a path holds null then, which is safe.
    fn replace(&mut self, value: Value, address: Value, offset: i32, counted: bool) {
        let flags = MemFlagsData::trusted();
        if counted {
            let old = self.body.ins().load(types::I64, flags, address, offset);
            self.body.ins().store(flags, value, address, offset);
            self.release(old);
        } else {
            self.body.ins().store(flags, value, address, offset);
        }
    }

    fn list_set(&mut self, local: Local, args: &[Expr], span: Span) {
        let list = self.body.use_var(self.vars[local]);
        let index = self.slot(&args[0]);
        let value = self.slot(&args[1]);
        let flags = MemFlagsData::trusted();
        let len = self.body.ins().load(types::I64, flags, list, lucid_rt::LEN_OFFSET);
        let inside = self.body.ins().icmp(IntCC::UnsignedLessThan, index, len);
        let unique = self.is_unique(list);
        let test = self.body.ins().band(inside, unique);
        let counted = matches!(self.types[local], Ty::List(item) if item.is_counted());
        let fast = |lower: &mut Self| {
            let offset = lower.body.ins().ishl_imm_s(index, 3);
            let address = lower.body.ins().iadd(list, offset);
            lower.replace(value, address, lucid_rt::SLOTS_OFFSET, counted);
            list
        };
        let task = self.runtime.list_set;
        self.update_paths(local, test, fast, task, &[list, index, value], Some(span));
    }

    fn list_push(&mut self, local: Local, args: &[Expr]) {
        let list = self.body.use_var(self.vars[local]);
        let value = self.slot(&args[0]);
        let flags = MemFlagsData::trusted();
        let len = self.body.ins().load(types::I64, flags, list, lucid_rt::LEN_OFFSET);
        let cap = self.body.ins().load(types::I64, flags, list, lucid_rt::CAP_OFFSET);
        let room = self.body.ins().icmp(IntCC::UnsignedLessThan, len, cap);
        let unique = self.is_unique(list);
        let test = self.body.ins().band(room, unique);
        let fast = |lower: &mut Self| {
            let offset = lower.body.ins().ishl_imm_s(len, 3);
            let address = lower.body.ins().iadd(list, offset);
            lower.body.ins().store(flags, value, address, lucid_rt::SLOTS_OFFSET);
            let longer = lower.body.ins().iadd_imm_s(len, 1);
            lower.body.ins().store(flags, longer, list, lucid_rt::LEN_OFFSET);
            list
        };
        self.update_paths(local, test, fast, self.runtime.list_push, &[list, value], None);
    }

    /// `args[1]` is a plain `none` when a write through a path empties a slot,
    /// or writes to a `none` field. The type does not show if that slot holds
    /// a count, so only a slot that does takes the fast path.
    fn field_set(&mut self, local: Local, args: &[Expr]) {
        let ExprKind::Int(slot) = args[0].kind else { unreachable!("`mono.rs` gives a slot") };
        let emptied = matches!(args[1].kind, ExprKind::None);
        let counted = args[1].ty.is_counted() || emptied;
        let record = self.body.use_var(self.vars[local]);
        let index = self.body.ins().iconst(types::I64, slot);
        let value = self.slot(&args[1]);
        let mut test = self.is_unique(record);
        if emptied {
            let flags = MemFlagsData::trusted();
            let held = self.body.ins().load(types::I64, flags, record, lucid_rt::COUNTED_OFFSET);
            let holds = self.body.ins().icmp_imm_s(IntCC::SignedGreaterThan, held, slot);
            test = self.body.ins().band(test, holds);
        }
        let fast = |lower: &mut Self| {
            lower.replace(value, record, slot_offset(slot as usize), counted);
            record
        };
        let args = [record, index, value];
        self.update_paths(local, test, fast, self.runtime.record_set, &args, None);
    }

    fn update(&mut self, local: Local, op: Update, args: &[Expr], span: Span) {
        match op {
            Update::ListSet => return self.list_set(local, args, span),
            Update::ListPush => return self.list_push(local, args),
            Update::FieldSet => return self.field_set(local, args),
            _ => {}
        }
        let mut values = vec![self.body.use_var(self.vars[local])];
        for arg in args {
            let value = self.slot(arg);
            values.push(value);
        }
        let runtime = self.runtime;
        let task = match op {
            Update::ListSwap => runtime.list_swap,
            Update::MapSet => runtime.map_set,
            Update::MapRemove => runtime.map_remove,
            Update::UnionSet => unreachable!("`mono.rs` replaces these"),
            Update::ListPop => runtime.list_pop,
            Update::ListSet | Update::ListPush | Update::FieldSet => {
                unreachable!("handled above")
            }
        };
        let result = self.call(task, &values).expect("it returns the new value");
        if op == Update::ListSwap {
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
    fn new_record(&mut self, tag: Value, counted: usize, fields: &[(usize, Expr)]) -> Value {
        let values: Vec<(usize, Value)> =
            fields.iter().map(|(slot, value)| (*slot, self.slot(value))).collect();
        let len = self.body.ins().iconst(types::I64, fields.len() as i64);
        let held = self.body.ins().iconst(types::I64, counted as i64);
        let record = self.call(self.runtime.record_new, &[tag, len, held]);
        let record = record.expect("it returns a record");
        for (slot, value) in values {
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
            ExprKind::Float(value) if expr.ty == Ty::F32 => self.body.ins().f32const(*value as f32),
            ExprKind::Float(value) => self.body.ins().f64const(*value),
            ExprKind::Bool(value) => self.body.ins().iconst(types::I8, i64::from(*value)),
            ExprKind::None => self.none(),
            ExprKind::Local(local) => self.body.use_var(self.vars[*local]),
            ExprKind::Call(task, _, args) => self.call_task(*task, args),
            ExprKind::Neg(value) => {
                let value = self.expr(value);
                self.body.ins().fneg(value)
            }
            ExprKind::Sqrt(value) => {
                let value = self.expr(value);
                self.body.ins().sqrt(value)
            }
            ExprKind::Not(value) => {
                let value = self.expr(value);
                self.body.ins().bxor_imm_s(value, 1)
            }
            ExprKind::Binary(op, left, right) => self.binary(*op, left, right, expr.span),
            ExprKind::If(condition, then, otherwise) => {
                self.if_value(condition, then, otherwise, clif(expr.ty))
            }
            ExprKind::Convert(value) => self.convert(value, expr.ty, expr.span),
            ExprKind::Wrapping(op, left, right) => {
                let a = self.expr(left);
                let b = self.expr(right);
                let value = self.wrapped(*op, a, b);
                self.extended(value, expr.ty)
            }
            ExprKind::Text(text) => self.static_value(lucid_rt::static_text(text)),
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
            ExprKind::MapValue(map, position) => {
                let map = self.expr(map);
                let position = self.expr(position);
                let value = self.call(self.runtime.map_value, &[map, position]);
                self.read_slot(value.expect("it returns a value"), expr.ty)
            }
            ExprKind::MapFind(map, key) => {
                let map = self.expr(map);
                let key = self.slot(key);
                self.call(self.runtime.map_position, &[map, key]).expect("it returns an int")
            }
            ExprKind::New { tag, fields, .. } if fields.is_empty() => {
                self.static_value(lucid_rt::static_record(*tag))
            }
            ExprKind::New { tag, counted, fields } => {
                let tag = self.body.ins().iconst(types::I64, *tag as i64);
                self.new_record(tag, *counted, fields)
            }
            ExprKind::Closure { task, counted, fields, .. } => {
                let func = self.func(self.ids[*task]);
                let code = self.body.ins().func_addr(self.pointer, func);
                self.new_record(code, *counted, fields)
            }
            ExprKind::CallValue(callee, args) => self.call_value(callee, args),
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
            ExprKind::Move(local) => {
                let value = self.body.use_var(self.vars[*local]);
                let empty = self.body.ins().iconst(self.pointer, 0);
                self.body.def_var(self.vars[*local], empty);
                value
            }
            ExprKind::Retain(value) => {
                let value = self.expr(value);
                self.retain(value);
                value
            }
            ExprKind::Change(_) => unreachable!("only `call_task` reads these"),
            ExprKind::Member { .. }
            | ExprKind::Wrap(_)
            | ExprKind::View(_)
            | ExprKind::Holds(..)
            | ExprKind::Capture(_)
            | ExprKind::Method(..) => unreachable!("`mono.rs` replaces these"),
        }
    }

    /// A `change` argument goes through a stack slot. The task loads it when it
    /// starts and stores it when it ends, so it stays in a register between.
    fn call_task(&mut self, task: usize, args: &[Expr]) -> Value {
        let mut values = Vec::with_capacity(args.len());
        let mut slots = Vec::new();
        for arg in args {
            let ExprKind::Change(local) = arg.kind else {
                let value = self.expr(arg);
                values.push(value);
                continue;
            };
            let data = StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3);
            let slot = self.body.create_sized_stack_slot(data);
            let value = self.body.use_var(self.vars[local]);
            self.body.ins().stack_store(self.pointer, value, slot, 0);
            values.push(self.body.ins().stack_addr(self.pointer, slot, 0));
            slots.push((local, slot, clif(arg.ty)));
        }
        let result = self.call(self.ids[task], &values).expect("every task returns a value");
        for (local, slot, ty) in slots {
            let value = self.body.ins().stack_load(self.pointer, ty, slot, 0);
            self.body.def_var(self.vars[local], value);
        }
        result
    }

    /// The record comes first in the call, so that the code finds its captures.
    fn call_value(&mut self, callee: &Expr, args: &[Expr]) -> Value {
        let Ty::Task(params, returns) = callee.ty else { unreachable!("it has a task type") };
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(self.pointer));
        signature.params.extend(params.iter().map(|&ty| AbiParam::new(clif(ty))));
        signature.returns.push(AbiParam::new(clif(*returns)));
        let signature = self.body.import_signature(signature);
        let record = self.expr(callee);
        let mut values = vec![record];
        for arg in args {
            let value = self.expr(arg);
            values.push(value);
        }
        let flags = MemFlagsData::trusted();
        let code = self.body.ins().load(self.pointer, flags, record, lucid_rt::TAG_OFFSET);
        let call = self.body.ins().call_indirect(signature, code, &values);
        self.body.inst_results(call)[0]
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
            Ty::U64 => self.call(self.runtime.uint_text, &[value]),
            ty if ty.is_integer() => self.call(self.runtime.int_text, &[value]),
            Ty::Float => self.call(self.runtime.float_text, &[value]),
            Ty::F32 => self.call(self.runtime.f32_text, &[value]),
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
        let task = match parts[0].kind {
            ExprKind::Move(_) => self.runtime.text_append,
            _ => self.runtime.text_join,
        };
        self.call(task, &[address, count]).expect("it returns a text")
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
        let (ty, float) = (left.ty, left.ty.is_float());
        let a = self.expr(left);
        if let ExprKind::Int(divisor) = right.kind
            && divisor > 0
            && divisor.count_ones() == 1
            && matches!(op, BinaryOp::FloorDiv | BinaryOp::Rem)
        {
            // Both round down, so this holds below zero too. A divisor like
            // this cannot trap, and the result is in range.
            let shift = i64::from(divisor.trailing_zeros());
            return match (op, ty) {
                (BinaryOp::Rem, _) => self.body.ins().band_imm_s(a, divisor - 1),
                (_, Ty::U64) => self.body.ins().ushr_imm_s(a, shift),
                _ => self.body.ins().sshr_imm_s(a, shift),
            };
        }
        let b = self.expr(right);
        if ty == Ty::Text {
            let cc = int_cc(op).expect("the checker allows only comparisons on text");
            let order = self.call(self.runtime.text_compare, &[a, b]);
            return self.body.ins().icmp_imm_s(cc, order.expect("it returns an int"), 0);
        }
        if let Some(cc) = int_cc(op) {
            let cc = if ty == Ty::U64 { cc.unsigned() } else { cc };
            let ins = self.body.ins();
            return if float { ins.fcmp(float_cc(cc), a, b) } else { ins.icmp(cc, a, b) };
        }
        match (op, float) {
            (BinaryOp::Add, true) => self.body.ins().fadd(a, b),
            (BinaryOp::Sub, true) => self.body.ins().fsub(a, b),
            (BinaryOp::Mul, true) => self.body.ins().fmul(a, b),
            (BinaryOp::Div, true) => self.body.ins().fdiv(a, b),
            (BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul, false) => {
                self.checked_math(op, a, b, ty, span)
            }
            (BinaryOp::FloorDiv, false) if ty == Ty::U64 => {
                self.check_divisor(b, span);
                self.body.ins().udiv(a, b)
            }
            (BinaryOp::Rem, false) if ty == Ty::U64 => {
                self.check_divisor(b, span);
                self.body.ins().urem(a, b)
            }
            (BinaryOp::FloorDiv, false) => {
                let value = self.floor_div(a, b, span);
                self.check_fits(value, ty, span);
                value
            }
            (BinaryOp::Rem, false) => self.floor_rem(a, b, span),
            _ => unreachable!("the checker allows only these operators"),
        }
    }

    fn wrapped(&mut self, op: BinaryOp, a: Value, b: Value) -> Value {
        match op {
            BinaryOp::Add => self.body.ins().iadd(a, b),
            BinaryOp::Sub => self.body.ins().isub(a, b),
            BinaryOp::Mul => self.body.ins().imul(a, b),
            _ => unreachable!("only these operators wrap"),
        }
    }

    /// The low bits of `value`, extended the way that `ty` holds them. A value
    /// of 64 bits comes back as it is.
    fn extended(&mut self, value: Value, ty: Ty) -> Value {
        let (narrow, signed) = match ty {
            Ty::I8 => (types::I8, true),
            Ty::I16 => (types::I16, true),
            Ty::I32 => (types::I32, true),
            Ty::Byte => (types::I8, false),
            Ty::U16 => (types::I16, false),
            Ty::U32 => (types::I32, false),
            _ => return value,
        };
        let low = self.body.ins().ireduce(narrow, value);
        if signed {
            self.body.ins().sextend(types::I64, low)
        } else {
            self.body.ins().uextend(types::I64, low)
        }
    }

    /// Traps unless `ty` can hold a result computed in 64 bits. That result
    /// is exact for any type of 32 bits or fewer, even `u32` times `u32` as an
    /// unsigned value, so a result that fits keeps all its bits when extended.
    fn check_fits(&mut self, value: Value, ty: Ty, span: Span) {
        let extended = self.extended(value, ty);
        if extended != value {
            let overflow = self.body.ins().icmp(IntCC::NotEqual, extended, value);
            self.trap_if(overflow, "integer overflow", span);
        }
    }

    fn checked_math(&mut self, op: BinaryOp, a: Value, b: Value, ty: Ty, span: Span) -> Value {
        if ty == Ty::Int && !CHECK_OVERFLOW {
            return self.wrapped(op, a, b);
        }
        let ins = self.body.ins();
        let (value, overflow) = match (ty, op) {
            (Ty::Int, BinaryOp::Add) => ins.sadd_overflow(a, b),
            (Ty::Int, BinaryOp::Sub) => ins.ssub_overflow(a, b),
            (Ty::Int, _) => ins.smul_overflow(a, b),
            (Ty::U64, BinaryOp::Add) => ins.uadd_overflow(a, b),
            (Ty::U64, BinaryOp::Sub) => ins.usub_overflow(a, b),
            (Ty::U64, _) => ins.umul_overflow(a, b),
            _ => {
                let value = self.wrapped(op, a, b);
                self.check_fits(value, ty, span);
                return value;
            }
        };
        self.checked(value, overflow, span)
    }

    /// The number `value` as a `to`. See `ExprKind::Convert`.
    fn convert(&mut self, value: &Expr, to: Ty, span: Span) -> Value {
        let from = value.ty;
        let value = self.expr(value);
        let ins = self.body.ins();
        match (from.is_float(), to.is_float()) {
            (true, true) if to == Ty::Float => ins.fpromote(types::F64, value),
            (true, true) => ins.fdemote(types::F32, value),
            (false, true) if from == Ty::U64 => ins.fcvt_from_uint(clif(to), value),
            (false, true) => ins.fcvt_from_sint(clif(to), value),
            (false, false) => self.convert_int(value, from, to, span),
            (true, false) => {
                let wide = if from == Ty::F32 { ins.fpromote(types::F64, value) } else { value };
                self.float_to_int(wide, to, span)
            }
        }
    }

    fn convert_int(&mut self, value: Value, from: Ty, to: Ty, span: Span) -> Value {
        let (low, high) = to.int_range();
        let (from_low, from_high) = from.int_range();
        let message = format!("integer does not fit in `{}`", to.name());
        if from == Ty::U64 {
            let cc = IntCC::UnsignedGreaterThan;
            let above = self.body.ins().icmp_imm_s(cc, value, high as i64);
            self.trap_if(above, &message, span);
            return value;
        }
        if from_low < low {
            let below = self.body.ins().icmp_imm_s(IntCC::SignedLessThan, value, low as i64);
            self.trap_if(below, &message, span);
        }
        if high < from_high {
            let above = self.body.ins().icmp_imm_s(IntCC::SignedGreaterThan, value, high as i64);
            self.trap_if(above, &message, span);
        }
        value
    }

    /// Rounds toward zero. The bounds are exact, except that `low - 1` has no
    /// `float` for `int`, whose test is then `>= low`. A NaN fails either test.
    fn float_to_int(&mut self, value: Value, to: Ty, span: Span) -> Value {
        let (low, high) = to.int_range();
        let floor = (low - 1) as f64;
        let exact = floor as i128 == low - 1;
        let cc = if exact { FloatCC::GreaterThan } else { FloatCC::GreaterThanOrEqual };
        let floor = self.body.ins().f64const(floor);
        let ceiling = self.body.ins().f64const((high + 1) as f64);
        let above = self.body.ins().fcmp(cc, value, floor);
        let below = self.body.ins().fcmp(FloatCC::LessThan, value, ceiling);
        let fits = self.body.ins().band(above, below);
        let outside = self.body.ins().icmp_imm_s(IntCC::Equal, fits, 0);
        self.trap_if(outside, &format!("float does not fit in `{}`", to.name()), span);
        if to == Ty::U64 {
            self.body.ins().fcvt_to_uint(types::I64, value)
        } else {
            self.body.ins().fcvt_to_sint(types::I64, value)
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
