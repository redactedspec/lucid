//! WebAssembly backend, for `lucid build --target web`. Lowers MIR to a module
//! that imports the runtime's memory and tasks, and wraps both modules in a
//! JavaScript loader. See "WebAssembly" in docs/compiler.md.
//!
//! Built so far: numbers, `bool`, `none`, text values, control flow, calls,
//! `print`, and traps with no trace. Anything else is E0901, until M6 step 4.

use crate::ast::BinaryOp;
use crate::diag::{Diagnostic, Lines, Source, Span};
use crate::mir::{Expr, ExprKind, Local, Program, Stmt, Task, Ty};
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection,
    Function, FunctionSection, GlobalType, ImportSection, InstructionSink, MemoryType, Module,
    TypeSection, ValType,
};

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
    ("lucid_fs_rename", &[Ptr, Ptr], None),
    ("lucid_fs_failure", &[], Some(I64)),
    ("lucid_fs_detail", &[], Some(Ptr)),
    ("lucid_bytes_valid", &[Ptr], Some(Small)),
    ("lucid_bytes_text", &[Ptr], Some(Ptr)),
];

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

/// Compiles the program to a JavaScript file that holds the loader and both
/// modules. See "Output" in docs/compiler.md.
pub fn script(program: &Program, source: &Source) -> Result<String, Vec<Diagnostic>> {
    let (module, data) = module(program, source)?;
    Ok(LOADER
        .replace("@DATA@", &data.to_string())
        .replace("@RUNTIME@", &base64(RUNTIME))
        .replace("@PROGRAM@", &base64(&module)))
}

/// The program module, and how many bytes of static data it needs.
fn module(program: &Program, source: &Source) -> Result<(Vec<u8>, usize), Vec<Diagnostic>> {
    let lines = Lines::new(source);
    let mut shared = Shared {
        types: TypeSection::new(),
        type_ids: HashMap::new(),
        data: Vec::new(),
        placed: HashMap::new(),
        lines: &lines,
        missing: None,
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
    for (name, params, result) in IMPORTS {
        let params: Vec<ValType> = params.iter().map(|&abi| abi_type(abi)).collect();
        let results: Vec<ValType> = result.iter().map(|&abi| abi_type(abi)).collect();
        let ty = shared.type_id(params, results);
        imports.import("rt", name, EntityType::Function(ty));
    }

    let mut functions = FunctionSection::new();
    let mut code = CodeSection::new();
    for task in program.tasks.iter().chain([&program.main]) {
        let params = task.locals[..task.params].iter().map(|&ty| val(ty)).collect();
        let ty = shared.type_id(params, vec![val(task.returns)]);
        functions.function(ty);
        code.function(&Lower::task(&mut shared, task));
    }
    if let Some(span) = shared.missing {
        let note = "M6 builds the rest of the web target, see docs/roadmap.md";
        let message = "this parses, but cannot run on the web yet";
        return Err(vec![Diagnostic::new("E0901", message, span).note(note)]);
    }

    let mut exports = ExportSection::new();
    let main = (IMPORTS.len() + program.tasks.len()) as u32;
    exports.export("main", ExportKind::Func, main);
    let mut data = DataSection::new();
    let offset = ConstExpr::global_get(MEMORY_BASE);
    data.active(0, &offset, shared.data.iter().copied());

    let mut module = Module::new();
    module.section(&shared.types);
    module.section(&imports);
    module.section(&functions);
    module.section(&exports);
    module.section(&code);
    module.section(&data);
    Ok((module.finish(), shared.data.len()))
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
    /// Counted locals that are not parameters or borrows, released on return.
    owned: Vec<Local>,
    /// How many blocks are open. A block's label is the depth inside it.
    depth: u32,
    /// The labels of `continue` and `break` of each open loop.
    loops: Vec<(u32, u32)>,
    /// What a return stores, before it leaves the block whose label is 1.
    result: u32,
}

impl<'a, 'b> Lower<'a, 'b> {
    fn task(shared: &'a mut Shared<'b>, task: &'a Task) -> Function {
        let locals = task.locals.iter().map(|&ty| val(ty)).collect();
        let mut lower = Lower {
            shared,
            code: Vec::new(),
            locals,
            owned: Vec::new(),
            depth: 0,
            loops: Vec::new(),
            result: 0,
        };
        lower.result = lower.scratch(val(task.returns));
        if task.change.is_empty() {
            for local in task.params..task.locals.len() {
                if task.locals[local].is_counted() && task.borrowed.binary_search(&local).is_err() {
                    lower.owned.push(local);
                }
            }
        } else {
            lower.missing(task.span);
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
        lower.ins().local_get(result).end();
        let extra = lower.locals[task.params..].iter().copied();
        let mut function = Function::new_with_locals_types(extra);
        function.raw(lower.code);
        function
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
            Stmt::Update { span, .. } => {
                self.missing(*span);
                return true;
            }
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
            ExprKind::Call(task, _, args) => {
                for arg in args {
                    if matches!(arg.kind, ExprKind::Change(_)) {
                        return self.missing(arg.span);
                    }
                    self.expr(arg);
                }
                self.ins().call((IMPORTS.len() + task) as u32);
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
                let slice = self.scratch(ValType::I64);
                self.ins().local_tee(slice).i64_eqz();
                self.trap_if("text slice out of bounds", expr.span);
                self.ins().local_get(slice);
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
            _ => self.missing(expr.span),
        }
    }

    /// Calls a runtime task on one counted value.
    fn runtime_of(&mut self, task: &str, value: &Expr) {
        self.ptr(value);
        self.call_runtime(task);
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
        if ty == Ty::Text {
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
