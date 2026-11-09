//! WebAssembly backend, for `lucid build --target web`. Lowers MIR to a module
//! that imports the runtime's memory and tasks, and wraps both modules in a
//! JavaScript loader. See "WebAssembly" in docs/compiler.md.
//!
//! A user `extern` and `.pointer()` are E0905 here, by decision 162.

use crate::ast::BinaryOp;
use crate::diag::{Diagnostic, Lines, Source, Span};
use crate::mir::{Expr, ExprKind, HashOp, KeyTasks, Local, Program, Stmt, Task, Ty, Update};
use crate::native::slot_offset;
use lucid_rt::{CAP_OFFSET, COUNT_OFFSET, LEN_OFFSET, SLOTS_OFFSET, TAG_OFFSET};
use std::borrow::Cow;
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ElementSection, Elements, EntityType,
    ExportKind, ExportSection, Function, FunctionSection, GlobalType, ImportSection,
    InstructionSink, MemArg, MemoryType, Module, RefType, TableType, TypeSection, ValType,
};

const OUT_OF_BOUNDS: &str = "list index out of bounds";

/// The runtime, built for `wasm32-unknown-unknown` by `build.rs`.
const RUNTIME: &[u8] = include_bytes!(env!("LUCID_RT_WASM"));

/// What the loader runs. See "Output" in docs/compiler.md.
const LOADER: &str = include_str!("loader.js");

/// How a runtime task takes or gives a value on `wasm32`, by the C ABI.
#[derive(Clone, Copy, PartialEq)]
enum Abi {
    /// A pointer or `usize`: an `i32`, which compiled code holds as an `i64`.
    Ptr,
    /// A `bool`, `u8`, or `u32`: an `i32`, as compiled code holds a `bool`.
    Small,
    I64,
    F64,
    F32,
}

use Abi::{F32, F64, I64, Ptr, Small};

/// Every runtime task, in import order, with its parameters and result. A wrong
/// entry fails when the program loads, which every web golden test does.
const IMPORTS: &[(&str, &[Abi], Option<Abi>)] = &[
    ("lucid_print", &[Ptr, Ptr], None),
    ("lucid_print_int", &[I64], None),
    ("lucid_print_uint", &[I64], None),
    ("lucid_print_float", &[F64], None),
    ("lucid_print_f32", &[F32], None),
    ("lucid_print_bool", &[Small], None),
    ("lucid_trap", &[Ptr, Ptr, Ptr], None),
    ("lucid_trap_text", &[Ptr, Ptr, Ptr, Ptr], None),
    ("lucid_exit", &[Ptr], None),
    ("lucid_retain", &[Ptr], None),
    ("lucid_release", &[Ptr], None),
    ("lucid_text_new", &[Ptr, Ptr], Some(Ptr)),
    ("lucid_channel", &[I64], Some(Ptr)),
    ("lucid_send", &[Ptr, Ptr], None),
    ("lucid_receive", &[Ptr], Some(Ptr)),
    ("lucid_close", &[Ptr], None),
    ("lucid_lock", &[Ptr], Some(Ptr)),
    ("lucid_lock_read", &[Ptr], Some(Ptr)),
    ("lucid_lock_take", &[Ptr], Some(Ptr)),
    ("lucid_lock_put", &[Ptr, Ptr], None),
    ("lucid_thread_failed", &[], Some(Small)),
    ("lucid_thread_failure", &[], Some(Ptr)),
    ("lucid_text_join", &[Ptr, Ptr], Some(Ptr)),
    ("lucid_text_append", &[Ptr, Ptr], Some(Ptr)),
    ("lucid_text_append_int", &[Ptr, I64], Some(Ptr)),
    ("lucid_text_print", &[Ptr], None),
    ("lucid_text_compare", &[Ptr, Ptr], Some(I64)),
    ("lucid_text_len", &[Ptr], Some(I64)),
    ("lucid_text_is_empty", &[Ptr], Some(Small)),
    ("lucid_text_slice", &[Ptr, I64, I64], Some(Ptr)),
    ("lucid_text_byte_len", &[Ptr], Some(I64)),
    ("lucid_text_char_at", &[Ptr, I64], Some(Ptr)),
    ("lucid_text_bytes", &[Ptr], Some(Ptr)),
    ("lucid_text_quote", &[Ptr], Some(Ptr)),
    ("lucid_text_split", &[Ptr, Ptr], Some(Ptr)),
    ("lucid_text_upper", &[Ptr], Some(Ptr)),
    ("lucid_text_lower", &[Ptr], Some(Ptr)),
    ("lucid_text_starts_with", &[Ptr, Ptr], Some(Small)),
    ("lucid_text_ends_with", &[Ptr, Ptr], Some(Small)),
    ("lucid_text_find", &[Ptr, Ptr], Some(I64)),
    ("lucid_text_trim", &[Ptr], Some(Ptr)),
    ("lucid_list_new", &[Ptr, Small, Small], Some(Ptr)),
    ("lucid_list_push", &[Ptr, I64], Some(Ptr)),
    ("lucid_list_unique", &[Ptr], Some(Ptr)),
    ("lucid_record_unique", &[Ptr], Some(Ptr)),
    ("lucid_list_room", &[Ptr], Some(Ptr)),
    ("lucid_list_pop", &[Ptr], Some(Ptr)),
    ("lucid_list_swap", &[Ptr, I64, I64], Some(Ptr)),
    ("lucid_list_has", &[Ptr, I64], Some(Small)),
    ("lucid_list_slice", &[Ptr, I64, I64], Some(Ptr)),
    ("lucid_map_new", &[Small, Small, Ptr, Ptr], Some(Ptr)),
    ("lucid_map_set", &[Ptr, I64, I64], Some(Ptr)),
    ("lucid_map_remove", &[Ptr, I64], Some(Ptr)),
    ("lucid_map_find", &[Ptr, I64], Some(Ptr)),
    ("lucid_map_next", &[Ptr, I64], Some(I64)),
    ("lucid_map_key", &[Ptr, I64], Some(I64)),
    ("lucid_map_value", &[Ptr, I64], Some(I64)),
    ("lucid_map_position", &[Ptr, I64], Some(I64)),
    ("lucid_set_from", &[Ptr, Ptr, Ptr], Some(Ptr)),
    ("lucid_hash_int", &[I64, I64], Some(I64)),
    ("lucid_hash_text", &[I64, Ptr], Some(I64)),
    ("lucid_hash_sum", &[I64, I64], Some(I64)),
    ("lucid_record_new", &[I64, Small, Small], Some(Ptr)),
    ("lucid_record_set", &[Ptr, Ptr, I64], Some(Ptr)),
    ("lucid_int_text", &[I64], Some(Ptr)),
    ("lucid_uint_text", &[I64], Some(Ptr)),
    ("lucid_float_text", &[F64], Some(Ptr)),
    ("lucid_f32_text", &[F32], Some(Ptr)),
    ("lucid_bool_text", &[Small], Some(Ptr)),
    ("lucid_args", &[], Some(Ptr)),
    ("lucid_env_has", &[Ptr], Some(Small)),
    ("lucid_env_get", &[Ptr], Some(Ptr)),
    ("lucid_text_is_int", &[Ptr], Some(Small)),
    ("lucid_text_int", &[Ptr], Some(I64)),
    ("lucid_text_float", &[Ptr], Some(F64)),
    ("lucid_text_json", &[Ptr], Some(Ptr)),
    ("lucid_time_now", &[], Some(I64)),
    ("lucid_time_clock", &[], Some(I64)),
    ("lucid_time_sleep", &[I64], None),
    ("lucid_fs_read", &[Ptr], Some(Ptr)),
    ("lucid_fs_write", &[Ptr, Ptr, Small], None),
    ("lucid_fs_exists", &[Ptr], Some(Small)),
    ("lucid_fs_list", &[Ptr], Some(Ptr)),
    ("lucid_fs_make_dir", &[Ptr], None),
    ("lucid_fs_remove", &[Ptr], None),
    ("lucid_fs_real_path", &[Ptr], Some(Ptr)),
    ("lucid_fs_rename", &[Ptr, Ptr], None),
    ("lucid_fs_failure", &[], Some(I64)),
    ("lucid_fs_detail", &[], Some(Ptr)),
    ("lucid_bytes_valid", &[Ptr], Some(Small)),
    ("lucid_bytes_text", &[Ptr], Some(Ptr)),
    ("lucid_dom_create", &[Ptr], Some(I64)),
    ("lucid_dom_text", &[Ptr], Some(I64)),
    ("lucid_dom_set_text", &[I64, Ptr], None),
    ("lucid_dom_set", &[I64, Ptr, Ptr], None),
    ("lucid_dom_unset", &[I64, Ptr], None),
    ("lucid_dom_on", &[I64, Ptr, I64], None),
    ("lucid_dom_off", &[I64, Ptr], None),
    ("lucid_dom_append", &[I64, I64], None),
    ("lucid_dom_replace", &[I64, I64], None),
    ("lucid_dom_remove", &[I64], None),
    ("lucid_dom_step", &[Ptr], None),
];

/// A load or store at `offset`, aligned to `2^align` bytes.
fn mem(offset: i32, align: u32) -> MemArg {
    MemArg { offset: offset as u64, align, memory_index: 0 }
}

fn abi_type(abi: Abi) -> ValType {
    match abi {
        Ptr | Small => ValType::I32,
        I64 => ValType::I64,
        F64 => ValType::F64,
        F32 => ValType::F32,
    }
}

/// How compiled code holds a value: as on native, in 64 bits, except a float
/// in its own type, and a `bool` or `none` in an `i32`.
fn val(ty: Ty) -> ValType {
    match ty {
        Ty::Float => ValType::F64,
        Ty::F32 => ValType::F32,
        Ty::Bool | Ty::None => ValType::I32,
        Ty::Param(..) => unreachable!("`mono.rs` replaces every parameter"),
        Ty::Error => unreachable!("errors stop before the backend"),
        _ => ValType::I64,
    }
}

/// The global that holds where the program's static data starts.
const MEMORY_BASE: u32 = 0;
/// The global that holds where the program's tasks start in the shared table.
const TABLE_BASE: u32 = 1;

/// A task's parameters, and its results: its own, then each `change`
/// parameter's value when it ends, by decision 167.
fn signature(task: &Task) -> (Vec<ValType>, Vec<ValType>) {
    let params = task.locals[..task.params].iter().map(|&ty| val(ty)).collect();
    let changed = task.change.iter().map(|&local| val(task.locals[local]));
    (params, [val(task.returns)].into_iter().chain(changed).collect())
}

/// What the loader needs besides the program module. See "Traps" in
/// docs/compiler.md.
struct Loaded {
    /// Bytes of static data.
    data: usize,
    /// Each call between tasks, by function index and offset in the module,
    /// as JavaScript: `"index:offset": "file:line:column in task"`, or `""`
    /// for a call that a trace leaves out.
    sites: String,
    /// The place of each task, by function index, for a stack overflow.
    tasks: String,
}

/// Compiles the program to a JavaScript file that holds the loader and both
/// modules. See "Output" in docs/compiler.md.
pub fn script(program: &Program, source: &Source) -> Result<String, Vec<Diagnostic>> {
    let (module, extra) = module(program, source)?;
    Ok(LOADER
        .replace("@DATA@", &extra.data.to_string())
        .replace("@TABLE@", &program.tasks.len().to_string())
        .replace("@SITES@", &extra.sites)
        .replace("@TASKS@", &extra.tasks)
        .replace("@RUNTIME@", &base64(RUNTIME))
        .replace("@PROGRAM@", &base64(&module)))
}

/// The length of `n` in LEB128, as a section or body size is written.
fn leb_len(n: usize) -> usize {
    (usize::BITS - n.leading_zeros()).max(1).div_ceil(7) as usize
}

/// The program module, and what the loader needs with it.
fn module(program: &Program, source: &Source) -> Result<(Vec<u8>, Loaded), Vec<Diagnostic>> {
    let lines = Lines::new(source);
    let mut shared = Shared {
        types: TypeSection::new(),
        type_ids: HashMap::new(),
        data: Vec::new(),
        placed: HashMap::new(),
        lines: &lines,
        missing: None,
        threads: None,
    };
    let mut imports = ImportSection::new();
    let memory = MemoryType {
        minimum: 0,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    };
    imports.import("rt", "memory", memory);
    let base = GlobalType { val_type: ValType::I32, mutable: false, shared: false };
    imports.import("env", "memory_base", base);
    let table = TableType {
        element_type: RefType::FUNCREF,
        table64: false,
        minimum: 0,
        maximum: None,
        shared: false,
    };
    imports.import("rt", "__indirect_function_table", table);
    imports.import("env", "table_base", base);
    for (name, params, result) in IMPORTS {
        let params: Vec<ValType> = params.iter().map(|&abi| abi_type(abi)).collect();
        let results: Vec<ValType> = result.iter().map(|&abi| abi_type(abi)).collect();
        let ty = shared.type_id(params, results);
        imports.import("rt", name, EntityType::Function(ty));
    }

    let mut functions = FunctionSection::new();
    let mut bodies = Vec::new();
    let mut places = Vec::new();
    for task in program.tasks.iter().chain([&program.main]) {
        let (params, results) = signature(task);
        functions.function(shared.type_id(params, results));
        bodies.push(Lower::task(&mut shared, task));
        let (file, line, column) = lines.place(task.span.start);
        places.push(format!(
            "{}: {:?}",
            IMPORTS.len() + places.len(),
            format!("{file}:{line}:{column}")
        ));
    }
    if let Some(span) = shared.threads {
        let message = "`spawn` cannot run on the web";
        let note = "threads in a page need cross-origin isolation, so build this program without `--target web`";
        return Err(vec![Diagnostic::new("E0907", message, span).note(note)]);
    }
    if let Some(span) = shared.missing {
        let message = "a user `extern` or `.pointer()` cannot run on the web";
        let note = "WebAssembly reaches no C library, so build this program without `--target web`";
        return Err(vec![Diagnostic::new("E0905", message, span).note(note)]);
    }

    let mut exports = ExportSection::new();
    let main = (IMPORTS.len() + program.tasks.len()) as u32;
    exports.export("main", ExportKind::Func, main);
    // Every task, so a key task or task value has a place in the table.
    let mut elements = ElementSection::new();
    let tasks: Vec<u32> =
        (0..program.tasks.len()).map(|task| (IMPORTS.len() + task) as u32).collect();
    elements.active(
        None,
        &ConstExpr::global_get(TABLE_BASE),
        Elements::Functions(Cow::Owned(tasks)),
    );
    let mut data = DataSection::new();
    let offset = ConstExpr::global_get(MEMORY_BASE);
    data.active(0, &offset, shared.data.iter().copied());

    let mut module = Module::new();
    module.section(&shared.types);
    module.section(&imports);
    module.section(&functions);
    module.section(&exports);
    module.section(&elements);

    // Each call's offset in the module, as an engine gives it in a trace: the
    // code section's header, then each body's size and local declarations.
    let sizes: Vec<usize> = bodies.iter().map(|(function, ..)| function.byte_len()).collect();
    let count = bodies.len();
    let section = leb_len(count) + sizes.iter().map(|&size| leb_len(size) + size).sum::<usize>();
    let mut at = module.as_slice().len() + 1 + leb_len(section) + leb_len(count);
    let mut sites = Vec::new();
    let mut code = CodeSection::new();
    for (index, (function, len, calls)) in bodies.iter().enumerate() {
        let size = function.byte_len();
        let start = at + leb_len(size) + size - len;
        for (offset, text) in calls {
            sites.push(format!("\"{}:{}\": {text:?}", IMPORTS.len() + index, start + offset));
        }
        at += leb_len(size) + size;
        code.function(function);
    }
    module.section(&code);
    module.section(&data);
    let extra = Loaded {
        data: shared.data.len(),
        sites: format!("{{{}}}", sites.join(", ")),
        tasks: format!("{{{}}}", places.join(", ")),
    };
    Ok((module.finish(), extra))
}

/// Standard base64, with padding, as `atob` and Node's `Buffer` read it.
fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word =
            chunk.iter().enumerate().fold(0u32, |word, (i, &b)| word | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            let digit =
                if i <= chunk.len() { DIGITS[(word >> (18 - 6 * i) & 63) as usize] } else { b'=' };
            text.push(digit as char);
        }
    }
    text
}

/// What every task adds to.
struct Shared<'a> {
    types: TypeSection,
    type_ids: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
    /// Static data, from `memory_base`.
    data: Vec<u8>,
    /// Where each value in `data` is, so it appears once.
    placed: HashMap<Vec<u8>, u32>,
    lines: &'a Lines<'a>,
    /// The first thing that this backend cannot lower yet.
    missing: Option<Span>,
    /// The first `spawn`, which is E0907 here, by decision 176.
    threads: Option<Span>,
}

impl Shared<'_> {
    fn type_id(&mut self, params: Vec<ValType>, results: Vec<ValType>) -> u32 {
        let next = self.type_ids.len() as u32;
        *self.type_ids.entry((params, results)).or_insert_with_key(|(params, results)| {
            self.types.ty().function(params.iter().copied(), results.iter().copied());
            next
        })
    }

    /// The offset of these bytes in static data, aligned for a header.
    fn place(&mut self, bytes: Vec<u8>) -> u32 {
        if let Some(&offset) = self.placed.get(&bytes) {
            return offset;
        }
        self.data.resize(self.data.len().next_multiple_of(8), 0);
        let offset = u32::try_from(self.data.len()).expect("static data fits in memory");
        self.data.extend_from_slice(&bytes);
        self.placed.insert(bytes, offset);
        offset
    }
}

/// The index of a runtime task.
fn runtime(name: &str) -> (u32, Option<Abi>) {
    let found = IMPORTS.iter().position(|(known, ..)| *known == name);
    let index = found.unwrap_or_else(|| panic!("`{name}` is not in IMPORTS"));
    (index as u32, IMPORTS[index].2)
}

struct Lower<'a, 'b> {
    shared: &'a mut Shared<'b>,
    code: Vec<u8>,
    /// The type of each local of the function, which starts with the task's.
    locals: Vec<ValType>,
    /// The MIR type of each of the task's locals.
    types: &'a [Ty],
    /// Counted locals that are not parameters or borrows, released on return.
    owned: Vec<Local>,
    /// How many blocks are open. A block's label is the depth inside it.
    depth: u32,
    /// The labels of `continue` and `break` of each open loop.
    loops: Vec<(u32, u32)>,
    /// What a return stores, before it leaves the block whose label is 1.
    result: u32,
    /// What a trace calls this task, or empty to leave it out.
    name: String,
    /// The offset in `code` of each call to a task, with what a trace shows.
    sites: Vec<(usize, String)>,
}

impl<'a, 'b> Lower<'a, 'b> {
    /// The function, the length of its code, and its calls to tasks.
    fn task(shared: &'a mut Shared<'b>, task: &'a Task) -> (Function, usize, Vec<(usize, String)>) {
        let locals = task.locals.iter().map(|&ty| val(ty)).collect();
        let mut lower = Lower {
            shared,
            code: Vec::new(),
            locals,
            types: &task.locals,
            owned: Vec::new(),
            depth: 0,
            loops: Vec::new(),
            result: 0,
            name: crate::diag::plain(task.name),
            sites: Vec::new(),
        };
        lower.result = lower.scratch(val(task.returns));
        for local in task.params..task.locals.len() {
            if task.locals[local].is_counted() && task.borrowed.binary_search(&local).is_err() {
                lower.owned.push(local);
            }
        }
        lower.open();
        lower.ins().block(BlockType::Empty);
        if !lower.stmts(&task.body) {
            if task.returns == Ty::None {
                let result = lower.result;
                lower.ins().i32_const(0).local_set(result);
            } else {
                // The checker proved that every path returns.
                lower.ins().unreachable();
            }
        }
        lower.close();
        for local in std::mem::take(&mut lower.owned) {
            lower.ins().local_get(local as u32);
            lower.release();
        }
        let result = lower.result;
        lower.ins().local_get(result);
        for &local in &task.change {
            lower.ins().local_get(local as u32);
        }
        lower.ins().end();
        let extra = lower.locals[task.params..].iter().copied();
        let mut function = Function::new_with_locals_types(extra);
        let len = lower.code.len();
        function.raw(lower.code);
        (function, len, lower.sites)
    }

    /// Records a call to a task, about to be written, for traces.
    fn site(&mut self, span: Span) {
        let text = if self.name.is_empty() {
            String::new()
        } else {
            let (file, line, column) = self.shared.lines.place(span.start);
            format!("{file}:{line}:{column} in {}", self.name)
        };
        self.sites.push((self.code.len(), text));
    }

    fn ins(&mut self) -> InstructionSink<'_> {
        InstructionSink::new(&mut self.code)
    }

    /// A new local of the function, for a value that code here reads twice.
    fn scratch(&mut self, ty: ValType) -> u32 {
        self.locals.push(ty);
        (self.locals.len() - 1) as u32
    }

    /// Records the first unbuilt feature, and ends this path.
    fn missing(&mut self, span: Span) {
        self.shared.missing.get_or_insert(span);
        self.ins().unreachable();
    }

    /// Opens a block, which the caller then writes, and gives its label.
    fn open(&mut self) -> u32 {
        self.depth += 1;
        self.depth
    }

    fn close(&mut self) {
        self.ins().end();
        self.depth -= 1;
    }

    fn br(&mut self, label: u32) {
        let depth = self.depth - label;
        self.ins().br(depth);
    }

    /// Calls a runtime task, whose arguments are on the stack in its form, and
    /// gives its result in the form of compiled code.
    fn call_runtime(&mut self, name: &str) {
        let (index, result) = runtime(name);
        self.ins().call(index);
        if result == Some(Ptr) {
            self.ins().i64_extend_i32_u();
        }
    }

    /// An address or length, from the 64 bits that compiled code holds.
    fn wrap_ptr(&mut self) {
        self.ins().i32_wrap_i64();
    }

    fn ptr(&mut self, expr: &Expr) {
        self.expr(expr);
        self.wrap_ptr();
    }

    /// The address and length of bytes in static data, as runtime arguments.
    fn static_bytes(&mut self, bytes: &[u8]) {
        let offset = self.shared.place(bytes.to_vec());
        self.ins().global_get(MEMORY_BASE).i32_const(offset as i32).i32_add();
        self.ins().i32_const(bytes.len() as i32);
    }

    /// The address of a value in static data, as compiled code holds it.
    fn static_value(&mut self, image: Vec<u8>) {
        let offset = self.shared.place(image);
        self.ins().global_get(MEMORY_BASE).i32_const(offset as i32).i32_add();
        self.ins().i64_extend_i32_u();
    }

    /// Gives up the count of the value on the stack, which may be null.
    fn release(&mut self) {
        self.wrap_ptr();
        self.call_runtime("lucid_release");
    }

    /// Replaces a counted local with the value on the stack, which it owns.
    fn store(&mut self, local: Local) {
        self.ins().local_get(local as u32);
        self.release();
        self.ins().local_set(local as u32);
    }

    /// Ends the path with a trap at `span`.
    fn trap(&mut self, message: &str, span: Span) {
        let (file, line, column) = self.shared.lines.place(span.start);
        self.static_bytes(format!("{file}:{line}:{column}: trap: {message}").as_bytes());
        // The frame is for a native trace, which the loader gives here instead.
        self.ins().i32_const(0);
        self.call_runtime("lucid_trap");
        self.ins().unreachable();
    }

    /// Traps if the `i32` on the stack is not zero.
    fn trap_if(&mut self, message: &str, span: Span) {
        self.open();
        self.ins().if_(BlockType::Empty);
        self.trap(message, span);
        self.close();
    }

    /// Lowers statements, and returns true if the path cannot go on past them.
    fn stmts(&mut self, stmts: &[Stmt]) -> bool {
        stmts.iter().any(|stmt| self.stmt(stmt))
    }

    fn stmt(&mut self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Set(local, value) => {
                self.expr(value);
                if value.ty.is_counted() {
                    self.store(*local);
                } else {
                    self.ins().local_set(*local as u32);
                }
            }
            Stmt::Bind(local, value) => {
                self.expr(value);
                self.ins().local_set(*local as u32);
            }
            Stmt::Clear(local) => {
                self.ins().i64_const(0);
                self.store(*local);
            }
            Stmt::Block(stmts) => return self.stmts(stmts),
            Stmt::If { branches, otherwise } => {
                // Flat, so an `else if` chain of any length nests no deeper.
                let done = self.open();
                self.ins().block(BlockType::Empty);
                for (condition, then) in branches {
                    self.expr(condition);
                    self.open();
                    self.ins().if_(BlockType::Empty);
                    if !self.stmts(then) {
                        self.br(done);
                    }
                    self.close();
                }
                self.stmts(otherwise);
                self.close();
            }
            Stmt::While { condition, body } => {
                let done = self.open();
                self.ins().block(BlockType::Empty);
                let head = self.open();
                self.ins().loop_(BlockType::Empty);
                self.expr(condition);
                let out = self.depth - done;
                self.ins().i32_eqz().br_if(out);
                self.loops.push((head, done));
                if !self.stmts(body) {
                    self.br(head);
                }
                self.loops.pop();
                self.close();
                self.close();
            }
            Stmt::For { local, start, end, inclusive, body } => {
                self.for_loop(*local, start, end, *inclusive, body);
            }
            Stmt::Return(value) => {
                self.expr(value);
                let result = self.result;
                self.ins().local_set(result);
                self.br(1);
                return true;
            }
            Stmt::Exit(value) => {
                self.ptr(value);
                self.call_runtime("lucid_exit");
                self.ins().unreachable();
                return true;
            }
            Stmt::Break => {
                let (_, done) = *self.loops.last().expect("the checker found a loop");
                self.br(done);
                return true;
            }
            Stmt::Continue => {
                let (next, _) = *self.loops.last().expect("the checker found a loop");
                self.br(next);
                return true;
            }
            Stmt::Print(value) => self.print(value),
            Stmt::PrintText(text) => self.print_text(text),
            Stmt::Unreachable => {
                self.ins().unreachable();
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
            Stmt::Fail(value, span) => {
                let (file, line, column) = self.shared.lines.place(span.start);
                self.static_bytes(format!("{file}:{line}:{column}: trap: ").as_bytes());
                self.ptr(value);
                self.ins().i32_const(0);
                self.call_runtime("lucid_trap_text");
                self.ins().unreachable();
                return true;
            }
            Stmt::Expr(value) => {
                self.expr(value);
                self.ins().drop();
            }
            Stmt::Update { local, op, args, span } => self.update(*local, *op, args, *span),
        }
        false
    }

    /// As on native: the bounds are read once, and the step leaves the loop
    /// at `end` before it adds, so `0..=MAX` ends instead of overflowing.
    fn for_loop(&mut self, local: Local, start: &Expr, end: &Expr, inclusive: bool, body: &[Stmt]) {
        let i = local as u32;
        self.expr(start);
        self.ins().local_set(i);
        let last = self.scratch(ValType::I64);
        self.expr(end);
        self.ins().local_set(last);
        let done = self.open();
        self.ins().block(BlockType::Empty);
        let head = self.open();
        self.ins().loop_(BlockType::Empty);
        self.ins().local_get(i).local_get(last);
        if inclusive {
            self.ins().i64_gt_s();
        } else {
            self.ins().i64_ge_s();
        }
        let out = self.depth - done;
        self.ins().br_if(out);
        let step = self.open();
        self.ins().block(BlockType::Empty);
        self.loops.push((step, done));
        self.stmts(body);
        self.loops.pop();
        self.close();
        let out = self.depth - done;
        self.ins().local_get(i).local_get(last).i64_eq().br_if(out);
        self.ins().local_get(i).i64_const(1).i64_add().local_set(i);
        self.br(head);
        self.close();
        self.close();
    }

    fn print(&mut self, value: &Expr) {
        if value.ty == Ty::None {
            self.expr(value);
            self.ins().drop();
            return self.print_text("none");
        }
        if value.ty == Ty::Text {
            self.ptr(value);
        } else {
            self.expr(value);
        }
        let task = match value.ty {
            Ty::U64 => "lucid_print_uint",
            ty if ty.is_integer() => "lucid_print_int",
            Ty::Float => "lucid_print_float",
            Ty::F32 => "lucid_print_f32",
            Ty::Bool => "lucid_print_bool",
            Ty::Text => "lucid_text_print",
            _ => unreachable!("the checker rejects printing any other type"),
        };
        self.call_runtime(task);
    }

    fn print_text(&mut self, text: &str) {
        self.static_bytes(text.as_bytes());
        self.call_runtime("lucid_print");
    }

    /// Leaves one value on the stack, in the form that `val` gives its type.
    fn expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Int(value) => {
                self.ins().i64_const(*value);
            }
            ExprKind::Float(value) if expr.ty == Ty::F32 => {
                self.ins().f32_const((*value as f32).into());
            }
            ExprKind::Float(value) => {
                self.ins().f64_const((*value).into());
            }
            ExprKind::Bool(value) => {
                self.ins().i32_const(i32::from(*value));
            }
            ExprKind::None => {
                self.ins().i32_const(0);
            }
            ExprKind::Local(local) => {
                self.ins().local_get(*local as u32);
            }
            // A `change` argument comes back as an extra result, by decision 167.
            ExprKind::Call(task, _, args) => {
                let mut changed = Vec::new();
                for arg in args {
                    if let ExprKind::Change(local) = arg.kind {
                        self.ins().local_get(local as u32);
                        changed.push(local as u32);
                    } else {
                        self.expr(arg);
                    }
                }
                self.site(expr.span);
                self.ins().call((IMPORTS.len() + task) as u32);
                for local in changed.into_iter().rev() {
                    self.ins().local_set(local);
                }
            }
            ExprKind::CallValue(callee, args) => self.call_value(callee, args, expr.span),
            ExprKind::Closure { task, counted, fields, .. } => {
                let task = *task;
                self.new_record(
                    |lower| {
                        lower.table_index(task);
                        lower.ins().i64_extend_i32_u();
                    },
                    *counted,
                    fields,
                );
            }
            ExprKind::New { tag, fields, .. } if fields.is_empty() => {
                self.static_value(lucid_rt::static_record(*tag));
            }
            ExprKind::New { tag, counted, fields } => {
                let tag = *tag as i64;
                self.new_record(
                    |lower| {
                        lower.ins().i64_const(tag);
                    },
                    *counted,
                    fields,
                );
            }
            ExprKind::Foreign(name, args) => self.foreign(name, args, expr.span),
            ExprKind::ListPointer(_) => self.missing(expr.span),
            ExprKind::ListSlice(list, start, end) => {
                self.ptr(list);
                self.expr(start);
                self.expr(end);
                self.call_runtime("lucid_list_slice");
                self.nonnull("list slice out of bounds", expr.span);
            }
            ExprKind::TextBytes(value) => self.runtime_of("lucid_text_bytes", value),
            ExprKind::ListOf(items) => {
                let Ty::List(item) = expr.ty else { unreachable!("it has a list type") };
                self.ins().i32_const(items.len() as i32);
                self.ins().i32_const(i32::from(item.is_counted()));
                self.ins().i32_const(i32::from(*item == Ty::Byte));
                self.call_runtime("lucid_list_new");
                for item in items {
                    self.wrap_ptr();
                    self.slot(item);
                    self.call_runtime("lucid_list_push");
                }
            }
            ExprKind::MapOf(entries, tasks) => {
                let Ty::Map(key, value) = expr.ty else { unreachable!("it has a map type") };
                self.ins().i32_const(i32::from(key.is_counted()));
                self.ins().i32_const(i32::from(value.is_counted()));
                self.key_tasks(*tasks);
                self.call_runtime("lucid_map_new");
                for (key, value) in entries {
                    self.wrap_ptr();
                    self.slot(key);
                    self.slot(value);
                    self.call_runtime("lucid_map_set");
                }
            }
            ExprKind::SetOf(list, tasks) => {
                self.ptr(list);
                self.key_tasks(*tasks);
                self.call_runtime("lucid_set_from");
            }
            ExprKind::Hash(op, state, value) => {
                self.expr(state);
                let task = match op {
                    HashOp::Int => {
                        self.slot(value);
                        "lucid_hash_int"
                    }
                    HashOp::Text => {
                        self.ptr(value);
                        "lucid_hash_text"
                    }
                    HashOp::Sum => {
                        self.expr(value);
                        "lucid_hash_sum"
                    }
                };
                self.call_runtime(task);
            }
            ExprKind::Len(value) => {
                self.ptr(value);
                self.ins().i64_load(mem(LEN_OFFSET, 3));
            }
            ExprKind::ListGet(list, index) => self.list_get(list, index, expr.ty, expr.span),
            ExprKind::ListHas(value, list) => {
                let held = self.scratch(ValType::I64);
                self.slot(value);
                self.ins().local_set(held);
                self.ptr(list);
                self.ins().local_get(held);
                self.call_runtime("lucid_list_has");
            }
            ExprKind::MapGet(map, key) => {
                self.ptr(map);
                self.slot(key);
                self.call_runtime("lucid_map_find");
                self.nonnull("map key not found", expr.span);
                self.wrap_ptr();
                self.load(expr.ty, 0);
            }
            ExprKind::MapHas(key, map) => {
                let held = self.scratch(ValType::I64);
                self.slot(key);
                self.ins().local_set(held);
                self.ptr(map);
                self.ins().local_get(held);
                self.call_runtime("lucid_map_find");
                self.ins().i64_const(0).i64_ne();
            }
            ExprKind::MapNext(map, position) => {
                self.ptr(map);
                self.expr(position);
                self.call_runtime("lucid_map_next");
            }
            ExprKind::MapKey(map, position) | ExprKind::MapValue(map, position) => {
                self.ptr(map);
                self.expr(position);
                let task = match expr.kind {
                    ExprKind::MapKey(..) => "lucid_map_key",
                    _ => "lucid_map_value",
                };
                self.call_runtime(task);
                self.read_slot(expr.ty);
            }
            ExprKind::MapFind(map, key) => {
                self.ptr(map);
                self.slot(key);
                self.call_runtime("lucid_map_position");
            }
            ExprKind::Field(value, slot) => {
                self.ptr(value);
                self.load(expr.ty, slot_offset(*slot));
            }
            ExprKind::Tag(value) => {
                self.ptr(value);
                self.ins().i64_load(mem(TAG_OFFSET, 3));
            }
            ExprKind::TagIn(value, tags) => {
                let tag = self.scratch(ValType::I64);
                self.ptr(value);
                self.ins().i64_load(mem(TAG_OFFSET, 3)).local_set(tag).i32_const(0);
                for &wanted in tags {
                    self.ins().local_get(tag).i64_const(wanted as i64).i64_eq().i32_or();
                }
            }
            ExprKind::Neg(value) => {
                self.expr(value);
                if value.ty == Ty::F32 {
                    self.ins().f32_neg();
                } else {
                    self.ins().f64_neg();
                }
            }
            ExprKind::Sqrt(value) => {
                self.expr(value);
                if value.ty == Ty::F32 {
                    self.ins().f32_sqrt();
                } else {
                    self.ins().f64_sqrt();
                }
            }
            ExprKind::Not(value) => {
                self.expr(value);
                self.ins().i32_eqz();
            }
            ExprKind::Binary(op, left, right) => self.binary(*op, left, right, expr.span),
            ExprKind::If(condition, then, otherwise) => {
                self.expr(condition);
                self.open();
                self.ins().if_(BlockType::Result(val(expr.ty)));
                self.expr(then);
                self.ins().else_();
                self.expr(otherwise);
                self.close();
            }
            ExprKind::Convert(value) => self.convert(value, expr.ty, expr.span),
            ExprKind::Wrapping(op, left, right) => {
                self.expr(left);
                self.expr(right);
                self.wrapped(*op);
                self.extended(expr.ty);
            }
            ExprKind::Text(text) => self.static_value(lucid_rt::static_text(text)),
            ExprKind::ToText(value) => self.text_of(value),
            ExprKind::Join(parts) => self.join(parts),
            ExprKind::TextLen(value) => self.runtime_of("lucid_text_len", value),
            ExprKind::TextIsEmpty(value) => self.runtime_of("lucid_text_is_empty", value),
            ExprKind::TextByteLen(value) => self.runtime_of("lucid_text_byte_len", value),
            ExprKind::TextQuote(value) => self.runtime_of("lucid_text_quote", value),
            ExprKind::TextCharAt(text, offset) => {
                self.ptr(text);
                self.expr(offset);
                self.call_runtime("lucid_text_char_at");
            }
            ExprKind::TextSlice(text, start, end) => {
                self.ptr(text);
                self.expr(start);
                self.expr(end);
                self.call_runtime("lucid_text_slice");
                self.nonnull("text slice out of bounds", expr.span);
            }
            ExprKind::Seq(setup, value) => {
                self.stmts(setup);
                self.expr(value);
            }
            ExprKind::Temp(local, value) => {
                self.expr(value);
                self.ins().local_get(*local as u32);
                self.release();
                self.ins().local_tee(*local as u32);
            }
            ExprKind::Move(local) => {
                self.ins().local_get(*local as u32).i64_const(0).local_set(*local as u32);
            }
            ExprKind::Retain(value) => {
                self.expr(value);
                let held = self.scratch(ValType::I64);
                self.ins().local_tee(held);
                self.wrap_ptr();
                self.call_runtime("lucid_retain");
                self.ins().local_get(held);
            }
            ExprKind::Change(_) => unreachable!("only a call reads these"),
            ExprKind::Member { .. }
            | ExprKind::Wrap(_)
            | ExprKind::View(_)
            | ExprKind::Holds(..)
            | ExprKind::Capture(_)
            | ExprKind::Method(..) => unreachable!("`mono.rs` replaces these"),
        }
    }

    /// Calls a runtime task on one counted value.
    fn runtime_of(&mut self, task: &str, value: &Expr) {
        self.ptr(value);
        self.call_runtime(task);
    }

    /// Traps at `span` if the address on the stack is null, and keeps it.
    fn nonnull(&mut self, message: &str, span: Span) {
        let value = self.scratch(ValType::I64);
        self.ins().local_tee(value).i64_eqz();
        self.trap_if(message, span);
        self.ins().local_get(value);
    }

    /// A value in the 8-byte form that every runtime slot has.
    fn slot(&mut self, expr: &Expr) {
        self.expr(expr);
        let mut ins = self.ins();
        match expr.ty {
            Ty::Float => ins.i64_reinterpret_f64(),
            Ty::F32 => ins.i32_reinterpret_f32().i64_extend_i32_u(),
            Ty::Bool | Ty::None => ins.i64_extend_i32_u(),
            _ => return,
        };
    }

    /// The reverse of `slot`, for the slot on the stack.
    fn read_slot(&mut self, ty: Ty) {
        let mut ins = self.ins();
        match ty {
            Ty::Float => ins.f64_reinterpret_i64(),
            Ty::F32 => ins.i32_wrap_i64().f32_reinterpret_i32(),
            Ty::Bool | Ty::None => ins.i32_wrap_i64(),
            _ => return,
        };
    }

    /// Reads a slot at `offset` from the address on the stack, in the form
    /// that `val` gives its type. A slot is little-endian, so its low bytes
    /// are a `bool` or an `f32`.
    fn load(&mut self, ty: Ty, offset: i32) {
        let mut ins = self.ins();
        match val(ty) {
            ValType::F64 => ins.f64_load(mem(offset, 3)),
            ValType::F32 => ins.f32_load(mem(offset, 2)),
            ValType::I32 => ins.i32_load(mem(offset, 2)),
            _ => ins.i64_load(mem(offset, 3)),
        };
    }

    /// True if the value in `local` has one count.
    fn is_unique(&mut self, local: Local) {
        self.ins().local_get(local as u32);
        self.wrap_ptr();
        self.ins().i64_load(mem(COUNT_OFFSET, 3)).i64_const(1).i64_eq();
    }

    /// Unless the `i32` on the stack is true, replaces the value in `local`
    /// with what the runtime `task` gives for it, which takes its count.
    fn make_room(&mut self, local: Local, task: &str) {
        self.open();
        self.ins().i32_eqz().if_(BlockType::Empty).local_get(local as u32);
        self.wrap_ptr();
        self.call_runtime(task);
        self.ins().local_set(local as u32);
        self.close();
    }

    /// The address on the stack plus the offset of slot `index`, a `i64`
    /// under it, of a list with elements of this type, less `SLOTS_OFFSET`.
    fn list_slot(&mut self, item: Ty) {
        self.wrap_ptr();
        if item != Ty::Byte {
            self.ins().i32_const(3).i32_shl();
        }
        self.ins().i32_add();
    }

    /// Stores the slot in `value` at the address on the stack, a list slot
    /// from `list_slot`.
    fn store_item(&mut self, value: u32, item: Ty) {
        self.ins().local_get(value);
        if item == Ty::Byte {
            self.ins().i64_store8(mem(SLOTS_OFFSET, 0));
        } else {
            self.ins().i64_store(mem(SLOTS_OFFSET, 3));
        }
    }

    /// Stores the slot in `value` at `offset` from the address on the stack,
    /// in a value that this code alone holds, and gives up what was there if
    /// it holds a count.
    fn replace(&mut self, value: u32, offset: i32, counted: bool) {
        let at = mem(offset, 3);
        if !counted {
            self.ins().local_get(value).i64_store(at);
            return;
        }
        let (address, old) = (self.scratch(ValType::I32), self.scratch(ValType::I64));
        self.ins().local_tee(address).i64_load(at).local_set(old);
        self.ins().local_get(address).local_get(value).i64_store(at).local_get(old);
        self.release();
    }

    fn item_of(&self, local: Local) -> Ty {
        let Ty::List(item) = self.types[local] else { unreachable!("it has a list type") };
        *item
    }

    /// The bounds check is inline: the unsigned compare also rejects a negative.
    fn list_get(&mut self, list: &Expr, index: &Expr, ty: Ty, span: Span) {
        let (held, at) = (self.scratch(ValType::I64), self.scratch(ValType::I64));
        self.expr(list);
        self.ins().local_set(held);
        self.expr(index);
        self.ins().local_tee(at).local_get(held);
        self.wrap_ptr();
        self.ins().i64_load(mem(LEN_OFFSET, 3)).i64_ge_u();
        self.trap_if(OUT_OF_BOUNDS, span);
        self.ins().local_get(held);
        self.wrap_ptr();
        self.ins().local_get(at);
        self.list_slot(ty);
        if ty == Ty::Byte {
            self.ins().i64_load8_u(mem(SLOTS_OFFSET, 0));
        } else {
            self.load(ty, SLOTS_OFFSET);
        }
    }

    /// The `i32` index of a task in the shared table.
    fn table_index(&mut self, task: usize) {
        self.ins().global_get(TABLE_BASE).i32_const(task as i32).i32_add();
    }

    /// The table indexes of the tasks that a map keeps, or 0 for each, which
    /// the runtime reads as none.
    fn key_tasks(&mut self, tasks: KeyTasks) {
        match tasks {
            Some((hash, equal)) => {
                self.table_index(hash);
                self.table_index(equal);
            }
            None => {
                self.ins().i32_const(0).i32_const(0);
            }
        }
    }

    /// Every value runs before the record exists, so that no field sees it.
    /// `tag` pushes the record's tag.
    fn new_record(&mut self, tag: impl Fn(&mut Self), counted: usize, fields: &[(usize, Expr)]) {
        let mut values = Vec::with_capacity(fields.len());
        for (slot, value) in fields {
            let held = self.scratch(ValType::I64);
            self.slot(value);
            self.ins().local_set(held);
            values.push((*slot, held));
        }
        tag(self);
        self.ins().i32_const(fields.len() as i32).i32_const(counted as i32);
        self.call_runtime("lucid_record_new");
        let record = self.scratch(ValType::I64);
        self.ins().local_set(record);
        for (slot, held) in values {
            self.ins().local_get(record);
            self.wrap_ptr();
            self.ins().local_get(held).i64_store(mem(slot_offset(slot), 3));
        }
        self.ins().local_get(record);
    }

    /// The record comes first in the call, so that the task finds its
    /// captures. Its tag is the task's index in the table.
    fn call_value(&mut self, callee: &Expr, args: &[Expr], span: Span) {
        let Ty::Task(params, returns) = callee.ty else { unreachable!("it has a task type") };
        let params = [ValType::I64].into_iter().chain(params.iter().map(|&ty| val(ty))).collect();
        let ty = self.shared.type_id(params, vec![val(*returns)]);
        let record = self.scratch(ValType::I64);
        self.expr(callee);
        self.ins().local_tee(record);
        for arg in args {
            self.expr(arg);
        }
        self.ins().local_get(record);
        self.wrap_ptr();
        self.ins().i64_load(mem(TAG_OFFSET, 3));
        self.wrap_ptr();
        self.site(span);
        self.ins().call_indirect(0, ty);
    }

    /// A foreign call is a runtime task here: `std` and the prelude name only
    /// those. A user `extern` cannot run on the web.
    fn foreign(&mut self, name: &str, args: &[Expr], span: Span) {
        if matches!(name, "lucid_spawn_scope" | "lucid_spawn" | "lucid_wait") {
            self.shared.threads.get_or_insert(span);
            self.ins().unreachable();
            return;
        }
        let Some((_, params, result)) = IMPORTS.iter().find(|(known, ..)| *known == name) else {
            return self.missing(span);
        };
        for (arg, &abi) in args.iter().zip(*params) {
            self.expr(arg);
            if matches!(abi, Ptr | Small) && val(arg.ty) == ValType::I64 {
                self.wrap_ptr();
            }
        }
        self.call_runtime(name);
        if result.is_none() {
            self.ins().i32_const(0);
        }
    }

    fn update(&mut self, local: Local, op: Update, args: &[Expr], span: Span) {
        let value = self.scratch(ValType::I64);
        match op {
            Update::ListSet => {
                let index = self.scratch(ValType::I64);
                self.expr(&args[0]);
                self.ins().local_set(index);
                self.slot(&args[1]);
                self.ins().local_set(value).local_get(index).local_get(local as u32);
                self.wrap_ptr();
                self.ins().i64_load(mem(LEN_OFFSET, 3)).i64_ge_u();
                self.trap_if(OUT_OF_BOUNDS, span);
                self.is_unique(local);
                self.make_room(local, "lucid_list_unique");
                let item = self.item_of(local);
                self.ins().local_get(local as u32);
                self.wrap_ptr();
                self.ins().local_get(index);
                self.list_slot(item);
                if item.is_counted() {
                    self.replace(value, SLOTS_OFFSET, true);
                } else {
                    self.store_item(value, item);
                }
            }
            Update::ListPush => {
                let len = self.scratch(ValType::I64);
                self.slot(&args[0]);
                self.ins().local_set(value).local_get(local as u32);
                self.wrap_ptr();
                self.ins().i64_load(mem(LEN_OFFSET, 3)).local_tee(len).local_get(local as u32);
                self.wrap_ptr();
                self.ins().i64_load(mem(CAP_OFFSET, 3)).i64_lt_u();
                self.is_unique(local);
                self.ins().i32_and();
                self.make_room(local, "lucid_list_room");
                let item = self.item_of(local);
                self.ins().local_get(local as u32);
                self.wrap_ptr();
                self.ins().local_get(len);
                self.list_slot(item);
                self.store_item(value, item);
                self.ins().local_get(local as u32);
                self.wrap_ptr();
                self.ins().local_get(len).i64_const(1).i64_add().i64_store(mem(LEN_OFFSET, 3));
            }
            // A plain `none` may empty a slot of a union, whose type does not
            // show if that slot holds a count, so the runtime looks.
            Update::FieldSet if matches!(args[1].kind, ExprKind::None) => {
                let ExprKind::Int(slot) = args[0].kind else { unreachable!("a slot") };
                self.ins().local_get(local as u32);
                self.wrap_ptr();
                self.ins().i32_const(slot as i32);
                self.slot(&args[1]);
                self.call_runtime("lucid_record_set");
                self.ins().local_set(local as u32);
            }
            Update::FieldSet => {
                let ExprKind::Int(slot) = args[0].kind else { unreachable!("a slot") };
                self.slot(&args[1]);
                self.ins().local_set(value);
                self.is_unique(local);
                self.make_room(local, "lucid_record_unique");
                self.ins().local_get(local as u32);
                self.wrap_ptr();
                self.replace(value, slot_offset(slot as usize), args[1].ty.is_counted());
            }
            Update::Take => {
                self.is_unique(local);
                let task = match self.types[local] {
                    Ty::List(_) => "lucid_list_unique",
                    _ => "lucid_record_unique",
                };
                self.make_room(local, task);
                self.store_slot(local, &args[0], value);
            }
            Update::Put => {
                self.slot(&args[1]);
                self.ins().local_set(value);
                self.store_slot(local, &args[0], value);
            }
            Update::ListSwap | Update::MapSet | Update::MapRemove | Update::ListPop => {
                self.ins().local_get(local as u32);
                self.wrap_ptr();
                for arg in args {
                    self.slot(arg);
                }
                let task = match op {
                    Update::ListSwap => "lucid_list_swap",
                    Update::MapSet => "lucid_map_set",
                    Update::MapRemove => "lucid_map_remove",
                    _ => "lucid_list_pop",
                };
                self.call_runtime(task);
                if op == Update::ListSwap {
                    self.nonnull(OUT_OF_BOUNDS, span);
                }
                self.ins().local_set(local as u32);
            }
            Update::UnionSet => unreachable!("`mono.rs` replaces these"),
        }
    }

    /// Stores the slot in `value` in a slot of the list or record in `local`.
    fn store_slot(&mut self, local: Local, key: &Expr, value: u32) {
        self.ins().local_get(local as u32);
        if let (ExprKind::Int(slot), Ty::Data(..) | Ty::Union(_)) = (&key.kind, self.types[local]) {
            self.wrap_ptr();
            self.ins().local_get(value).i64_store(mem(slot_offset(*slot as usize), 3));
        } else {
            let item = self.item_of(local);
            self.wrap_ptr();
            self.expr(key);
            self.list_slot(item);
            self.store_item(value, item);
        }
    }

    fn text_of(&mut self, value: &Expr) {
        if value.ty == Ty::None {
            self.expr(value);
            self.ins().drop();
            self.static_bytes(b"none");
            return self.call_runtime("lucid_text_new");
        }
        self.expr(value);
        let task = match value.ty {
            Ty::U64 => "lucid_uint_text",
            ty if ty.is_integer() => "lucid_int_text",
            Ty::Float => "lucid_float_text",
            Ty::F32 => "lucid_f32_text",
            Ty::Bool => "lucid_bool_text",
            _ => unreachable!("the checker converts only these types"),
        };
        self.call_runtime(task);
    }

    /// Adds each part to a text that this statement owns: the first part if
    /// it was moved, or else a new empty text. An `int` needs no text of its
    /// own. Native code joins in one call instead, from an array on its
    /// stack, which needs the shadow stack here.
    fn join(&mut self, parts: &[Expr]) {
        let rest = if matches!(parts[0].kind, ExprKind::Move(_)) {
            self.expr(&parts[0]);
            &parts[1..]
        } else {
            self.static_bytes(b"");
            self.call_runtime("lucid_text_new");
            parts
        };
        for part in rest {
            self.wrap_ptr();
            if let ExprKind::Temp(_, inner) = &part.kind
                && let ExprKind::ToText(number) = &inner.kind
                && number.ty == Ty::Int
            {
                self.expr(number);
                self.call_runtime("lucid_text_append_int");
            } else {
                self.ptr(part);
                self.call_runtime("lucid_text_append");
            }
        }
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, right: &Expr, span: Span) {
        let ty = left.ty;
        if let ExprKind::Int(divisor) = right.kind
            && divisor > 0
            && divisor.count_ones() == 1
            && matches!(op, BinaryOp::FloorDiv | BinaryOp::Rem)
        {
            // As on native: both round down, so this holds below zero too.
            self.expr(left);
            let shift = i64::from(divisor.trailing_zeros());
            match (op, ty) {
                (BinaryOp::Rem, _) => self.ins().i64_const(divisor - 1).i64_and(),
                (_, Ty::U64) => self.ins().i64_const(shift).i64_shr_u(),
                _ => self.ins().i64_const(shift).i64_shr_s(),
            };
            return;
        }
        if matches!(ty, Ty::Text | Ty::Sql) {
            self.ptr(left);
            self.ptr(right);
            self.call_runtime("lucid_text_compare");
            self.ins().i64_const(0);
            return self.compare(op, ValType::I64, false);
        }
        self.expr(left);
        self.expr(right);
        if matches!(op, BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le)
            || matches!(op, BinaryOp::Gt | BinaryOp::Ge)
        {
            return self.compare(op, val(ty), ty == Ty::U64);
        }
        if !ty.is_float() {
            return match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => self.checked_math(op, ty, span),
                BinaryOp::FloorDiv | BinaryOp::Rem => self.divide(op, ty, span),
                BinaryOp::BitAnd => _ = self.ins().i64_and(),
                BinaryOp::BitOr => _ = self.ins().i64_or(),
                BinaryOp::BitXor => _ = self.ins().i64_xor(),
                BinaryOp::Shl | BinaryOp::Shr => self.shift(op, ty, span),
                _ => unreachable!("the checker allows only these operators"),
            };
        }
        let mut ins = self.ins();
        match (op, val(ty)) {
            (BinaryOp::Add, ValType::F64) => ins.f64_add(),
            (BinaryOp::Sub, ValType::F64) => ins.f64_sub(),
            (BinaryOp::Mul, ValType::F64) => ins.f64_mul(),
            (BinaryOp::Div, ValType::F64) => ins.f64_div(),
            (BinaryOp::Add, _) => ins.f32_add(),
            (BinaryOp::Sub, _) => ins.f32_sub(),
            (BinaryOp::Mul, _) => ins.f32_mul(),
            (BinaryOp::Div, _) => ins.f32_div(),
            _ => unreachable!("the checker allows only these operators"),
        };
    }

    /// Compares the two values on the stack, of this type.
    fn compare(&mut self, op: BinaryOp, ty: ValType, unsigned: bool) {
        let mut ins = self.ins();
        match (ty, op) {
            (ValType::I32, BinaryOp::Eq) => ins.i32_eq(),
            (ValType::I32, BinaryOp::Ne) => ins.i32_ne(),
            (ValType::I32, BinaryOp::Lt) => ins.i32_lt_s(),
            (ValType::I32, BinaryOp::Le) => ins.i32_le_s(),
            (ValType::I32, BinaryOp::Gt) => ins.i32_gt_s(),
            (ValType::I32, _) => ins.i32_ge_s(),
            (ValType::I64, BinaryOp::Eq) => ins.i64_eq(),
            (ValType::I64, BinaryOp::Ne) => ins.i64_ne(),
            (ValType::I64, BinaryOp::Lt) if unsigned => ins.i64_lt_u(),
            (ValType::I64, BinaryOp::Le) if unsigned => ins.i64_le_u(),
            (ValType::I64, BinaryOp::Gt) if unsigned => ins.i64_gt_u(),
            (ValType::I64, BinaryOp::Ge) if unsigned => ins.i64_ge_u(),
            (ValType::I64, BinaryOp::Lt) => ins.i64_lt_s(),
            (ValType::I64, BinaryOp::Le) => ins.i64_le_s(),
            (ValType::I64, BinaryOp::Gt) => ins.i64_gt_s(),
            (ValType::I64, _) => ins.i64_ge_s(),
            (ValType::F64, BinaryOp::Eq) => ins.f64_eq(),
            (ValType::F64, BinaryOp::Ne) => ins.f64_ne(),
            (ValType::F64, BinaryOp::Lt) => ins.f64_lt(),
            (ValType::F64, BinaryOp::Le) => ins.f64_le(),
            (ValType::F64, BinaryOp::Gt) => ins.f64_gt(),
            (ValType::F64, _) => ins.f64_ge(),
            (ValType::F32, BinaryOp::Eq) => ins.f32_eq(),
            (ValType::F32, BinaryOp::Ne) => ins.f32_ne(),
            (ValType::F32, BinaryOp::Lt) => ins.f32_lt(),
            (ValType::F32, BinaryOp::Le) => ins.f32_le(),
            (ValType::F32, BinaryOp::Gt) => ins.f32_gt(),
            (ValType::F32, _) => ins.f32_ge(),
            _ => unreachable!("no other type compares"),
        };
    }

    fn wrapped(&mut self, op: BinaryOp) {
        match op {
            BinaryOp::Add => self.ins().i64_add(),
            BinaryOp::Sub => self.ins().i64_sub(),
            BinaryOp::Mul => self.ins().i64_mul(),
            _ => unreachable!("only these operators wrap"),
        };
    }

    /// The low bits of the value on the stack, extended the way that `ty`
    /// holds them. A value of 64 bits stays as it is.
    fn extended(&mut self, ty: Ty) {
        let mut ins = self.ins();
        match ty {
            Ty::I8 => ins.i64_extend8_s(),
            Ty::I16 => ins.i64_extend16_s(),
            Ty::I32 => ins.i64_extend32_s(),
            Ty::Byte => ins.i64_const(0xff).i64_and(),
            Ty::U16 => ins.i64_const(0xffff).i64_and(),
            Ty::U32 => ins.i64_const(0xffff_ffff).i64_and(),
            _ => return,
        };
    }

    /// Traps unless `ty` holds the value on the stack, which stays there. As
    /// on native, a result of 32 bits or fewer is exact in 64.
    fn check_fits(&mut self, ty: Ty, span: Span) {
        if matches!(ty, Ty::Int | Ty::U64) {
            return;
        }
        let value = self.scratch(ValType::I64);
        self.ins().local_tee(value);
        self.extended(ty);
        self.ins().local_get(value).i64_ne();
        self.trap_if("integer overflow", span);
        self.ins().local_get(value);
    }

    /// The two values on the stack, in locals, to read more than once.
    fn operands(&mut self) -> (u32, u32) {
        let (a, b) = (self.scratch(ValType::I64), self.scratch(ValType::I64));
        self.ins().local_set(b).local_set(a);
        (a, b)
    }

    /// WebAssembly has no overflow flag, so each test reads the operands and
    /// the wrapped result, as Hacker's Delight 2-13 and 2-12 give them.
    fn checked_math(&mut self, op: BinaryOp, ty: Ty, span: Span) {
        if !matches!(ty, Ty::Int | Ty::U64) {
            self.wrapped(op);
            return self.check_fits(ty, span);
        }
        let (a, b) = self.operands();
        let result = self.scratch(ValType::I64);
        self.ins().local_get(a).local_get(b);
        self.wrapped(op);
        self.ins().local_set(result);
        let mut ins = self.ins();
        match (ty, op) {
            // Signs of both operands agree, and differ from the result's.
            (Ty::Int, BinaryOp::Add) => {
                ins.local_get(a).local_get(result).i64_xor();
                ins.local_get(b).local_get(result).i64_xor();
                ins.i64_and().i64_const(0).i64_lt_s();
            }
            // Signs of the operands differ, and the result's is not `a`'s.
            (Ty::Int, BinaryOp::Sub) => {
                ins.local_get(a).local_get(b).i64_xor();
                ins.local_get(a).local_get(result).i64_xor();
                ins.i64_and().i64_const(0).i64_lt_s();
            }
            (Ty::U64, BinaryOp::Add) => {
                ins.local_get(result).local_get(a).i64_lt_u();
            }
            (Ty::U64, BinaryOp::Sub) => {
                ins.local_get(a).local_get(b).i64_lt_u();
            }
            // Dividing back gives `b` unless the product overflowed. Only
            // `MIN / -1` would trap, and its case needs no division.
            (_, _) => {
                let unsigned = ty == Ty::U64;
                ins.local_get(a).i64_eqz();
                self.open();
                self.ins().if_(BlockType::Result(ValType::I32));
                self.ins().i32_const(0).else_();
                if unsigned {
                    let mut ins = self.ins();
                    ins.local_get(result).local_get(a).i64_div_u().local_get(b).i64_ne();
                } else {
                    self.ins().local_get(a).i64_const(-1).i64_eq();
                    self.open();
                    self.ins().if_(BlockType::Result(ValType::I32));
                    self.ins().local_get(b).i64_const(i64::MIN).i64_eq().else_();
                    let mut ins = self.ins();
                    ins.local_get(result).local_get(a).i64_div_s().local_get(b).i64_ne();
                    self.close();
                }
                self.close();
            }
        }
        self.trap_if("integer overflow", span);
        self.ins().local_get(result);
    }

    /// `<<` and `>>`, with the count checked as on native.
    fn shift(&mut self, op: BinaryOp, ty: Ty, span: Span) {
        let (a, b) = self.operands();
        self.ins().local_get(b).i64_const(ty.bits()).i64_ge_u();
        self.trap_if("shift count out of range", span);
        self.ins().local_get(a).local_get(b);
        if op == BinaryOp::Shl {
            self.ins().i64_shl();
            self.extended(ty);
        } else if ty.int_range().0 < 0 {
            self.ins().i64_shr_s();
        } else {
            self.ins().i64_shr_u();
        }
    }

    /// `//` and `%`, which round down. As on native.
    fn divide(&mut self, op: BinaryOp, ty: Ty, span: Span) {
        let (a, b) = self.operands();
        self.ins().local_get(b).i64_eqz();
        self.trap_if("division by zero", span);
        if ty == Ty::U64 {
            self.ins().local_get(a).local_get(b);
            if op == BinaryOp::Rem {
                self.ins().i64_rem_u();
            } else {
                self.ins().i64_div_u();
            }
            return;
        }
        // A truncated remainder needs correcting if it is not zero, and its
        // sign differs from the divisor's.
        let r = self.scratch(ValType::I64);
        let round = |lower: &mut Self| {
            let mut ins = lower.ins();
            ins.local_get(r).i64_const(0).i64_ne();
            ins.local_get(r).local_get(b).i64_xor().i64_const(0).i64_lt_s();
            ins.i32_and();
        };
        if op == BinaryOp::Rem {
            self.ins().local_get(a).local_get(b).i64_rem_s().local_set(r);
            self.ins().local_get(r).local_get(b).i64_add().local_get(r);
            round(self);
            self.ins().select();
            return;
        }
        self.ins().local_get(a).i64_const(i64::MIN).i64_eq();
        self.ins().local_get(b).i64_const(-1).i64_eq().i32_and();
        self.trap_if("integer overflow", span);
        let q = self.scratch(ValType::I64);
        self.ins().local_get(a).local_get(b).i64_div_s().local_tee(q);
        self.ins().local_get(b).i64_mul().local_set(r);
        self.ins().local_get(a).local_get(r).i64_sub().local_set(r);
        self.ins().local_get(q);
        round(self);
        self.ins().i64_extend_i32_u().i64_sub();
        self.check_fits(ty, span);
    }

    /// The number `value` as a `to`. See `ExprKind::Convert`.
    fn convert(&mut self, value: &Expr, to: Ty, span: Span) {
        let from = value.ty;
        self.expr(value);
        let mut ins = self.ins();
        match (from.is_float(), to.is_float()) {
            (true, true) if from == to => {}
            (true, true) if to == Ty::Float => {
                ins.f64_promote_f32();
            }
            (true, true) => {
                ins.f32_demote_f64();
            }
            (false, true) => {
                match (from == Ty::U64, to == Ty::Float) {
                    (true, true) => ins.f64_convert_i64_u(),
                    (true, false) => ins.f32_convert_i64_u(),
                    (false, true) => ins.f64_convert_i64_s(),
                    (false, false) => ins.f32_convert_i64_s(),
                };
            }
            (false, false) => self.convert_int(from, to, span),
            (true, false) => {
                if from == Ty::F32 {
                    ins.f64_promote_f32();
                }
                self.float_to_int(to, span);
            }
        }
    }

    fn convert_int(&mut self, from: Ty, to: Ty, span: Span) {
        let (low, high) = to.int_range();
        let (from_low, from_high) = from.int_range();
        let message = format!("integer does not fit in `{}`", to.name());
        let value = self.scratch(ValType::I64);
        self.ins().local_set(value);
        if from == Ty::U64 {
            self.ins().local_get(value).i64_const(high as i64).i64_gt_u();
            self.trap_if(&message, span);
        } else {
            if from_low < low {
                self.ins().local_get(value).i64_const(low as i64).i64_lt_s();
                self.trap_if(&message, span);
            }
            if high < from_high {
                self.ins().local_get(value).i64_const(high as i64).i64_gt_s();
                self.trap_if(&message, span);
            }
        }
        self.ins().local_get(value);
    }

    /// Rounds toward zero, with the bounds that native uses. A NaN fails both.
    fn float_to_int(&mut self, to: Ty, span: Span) {
        let (low, high) = to.int_range();
        let floor = (low - 1) as f64;
        let exact = floor as i128 == low - 1;
        let value = self.scratch(ValType::F64);
        self.ins().local_tee(value).f64_const(floor.into());
        if exact {
            self.ins().f64_gt();
        } else {
            self.ins().f64_ge();
        }
        self.ins().local_get(value).f64_const(((high + 1) as f64).into()).f64_lt();
        self.ins().i32_and().i32_eqz();
        self.trap_if(&format!("float does not fit in `{}`", to.name()), span);
        self.ins().local_get(value);
        if to == Ty::U64 {
            self.ins().i64_trunc_f64_u();
        } else {
            self.ins().i64_trunc_f64_s();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc_4648() {
        // RFC 4648, section 10.
        let cases =
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg==")];
        for (bytes, text) in
            cases.into_iter().chain([("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")])
        {
            assert_eq!(super::base64(bytes.as_bytes()), text);
        }
    }
}
