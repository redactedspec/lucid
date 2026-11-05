//! Cranelift backend. At M1 the only supported program is a list of `print`
//! statements of plain text, so this reads the tree directly. MIR arrives at M2.

use crate::ast::{ExprKind, File, StmtKind, TextPart};
use crate::diag::{Diagnostic, Span};
use cranelift_codegen::ir::{AbiParam, InstBuilder, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, Linkage, Module, default_libcall_names};

const CODEGEN: &str = "code generation failed";

/// Compiles the program, and runs it in this process.
pub fn run(file: &File) -> Result<(), Vec<Diagnostic>> {
    let texts = print_list(file)?;
    let entry = compile(&texts);
    // SAFETY: `compile` built `entry` as a task with no parameters and no
    // result. Its module is leaked, so the code stays mapped.
    let entry = unsafe { std::mem::transmute::<*const u8, extern "C" fn()>(entry) };
    entry();
    Ok(())
}

/// Returns the text of each `print`, or an error for every other statement.
fn print_list(file: &File) -> Result<Vec<&str>, Vec<Diagnostic>> {
    let mut texts = Vec::new();
    let mut diags = Vec::new();
    for stmt in &file.stmts {
        let StmtKind::Print(value) = &stmt.kind else {
            diags.push(unsupported("this statement", stmt.span));
            continue;
        };
        match &value.kind {
            ExprKind::Text(parts) => match parts.as_slice() {
                [] => texts.push(""),
                [TextPart::Literal(text)] => texts.push(text),
                _ => diags.push(unsupported("text interpolation", value.span)),
            },
            _ => diags.push(unsupported("`print` of a non-text value", value.span)),
        }
    }
    if diags.is_empty() { Ok(texts) } else { Err(diags) }
}

fn unsupported(what: &str, span: Span) -> Diagnostic {
    Diagnostic::new("E0901", format!("{what} parses, but cannot run yet"), span)
        .note("only `print` of plain text runs at this milestone; see docs/roadmap.md")
}

/// Panics on a failure: valid input reaching here makes that a compiler bug.
fn compile(texts: &[&str]) -> *const u8 {
    let mut builder = JITBuilder::new(default_libcall_names()).expect("no JIT for this host");
    builder.symbol("lucid_print", lucid_rt::lucid_print as *const u8);
    let mut module = JITModule::new(builder);

    let pointer = module.target_config().pointer_type();
    let mut print = module.make_signature();
    print.params.push(AbiParam::new(pointer));
    print.params.push(AbiParam::new(types::I64));
    let print = module.declare_function("lucid_print", Linkage::Import, &print).expect(CODEGEN);

    let signature = module.make_signature();
    let main = module.declare_function("main", Linkage::Local, &signature).expect(CODEGEN);
    let mut context = module.make_context();
    context.func.signature = signature;

    let mut builder_context = FunctionBuilderContext::new();
    let mut body = FunctionBuilder::new(&mut context.func, &mut builder_context);
    let block = body.create_block();
    body.switch_to_block(block);
    body.seal_block(block);
    let print = module.declare_func_in_func(print, body.func);
    for text in texts {
        let data = module.declare_anonymous_data(false, false).expect(CODEGEN);
        let mut description = DataDescription::new();
        // The trailing byte keeps the data non-empty, so its pointer is always valid.
        description.define([text.as_bytes(), &[0]].concat().into_boxed_slice());
        module.define_data(data, &description).expect(CODEGEN);

        let data = module.declare_data_in_func(data, body.func);
        let address = body.ins().symbol_value(pointer, data);
        let len = body.ins().iconst(types::I64, text.len() as i64);
        body.ins().call(print, &[address, len]);
    }
    body.ins().return_(&[]);
    body.finalize(module.target_config());

    module.define_function(main, &mut context).expect(CODEGEN);
    module.finalize_definitions().expect(CODEGEN);
    let entry = module.get_finalized_function(main);
    // Leaked on purpose: `entry` points into memory that the module owns.
    std::mem::forget(module);
    entry
}
