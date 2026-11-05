# Compiler Architecture

The compiler is written in Rust until M9, when its front end moves to Lucid (decision 16). The lexer, parser, type checker, and ownership analysis are hand-written. Machine code comes from Cranelift, and browser code comes from a WebAssembly backend.

## Status

As of M1, the lexer and parser cover the full grammar. `lucid check` runs only those two stages, so it reports syntax errors only. `lucid run` compiles programs made of `print` statements of plain text, and reports E0901 for anything else. `native.rs` reads the syntax tree directly, and the stages between the parser and the backend are planned.

## Pipeline

```text
source text
  -> lexer          tokens with NEWLINE, INDENT, DEDENT
  -> parser         AST, with error recovery
  -> resolver       names bound across modules
  -> type checker   inference, narrowing, exhaustiveness
  -> monomorphizer  concrete tasks and types only
  -> MIR            simple, explicit, structured operations
  -> ownership      moves, count operations, exclusivity, reuse
  -> backend        Cranelift (native, JIT) or wasm-encoder (web)
```

**Key decision:** MIR stays structured (blocks, ifs, loops) and is not a free control-flow graph. Lucid has no `goto`, so nothing is lost. WebAssembly needs structured control flow, so its backend stays small, and Cranelift can build its own graph from the same input. Both backends share one MIR.

## Repository layout

Start with few files. Split a file only when it hurts.

```text
compiler/          Rust crate `lucid`: the CLI and compiler
    src/
        main.rs    CLI
        diag.rs    diagnostics
        lexer.rs
        ast.rs
        parser.rs
        resolve.rs     planned
        check.rs       planned
        mono.rs        planned
        mir.rs         planned
        ownership.rs   planned
        native.rs  Cranelift backend
        wasm.rs        planned: WebAssembly backend
    tests/golden.rs    runs everything in `tests/` and `examples/`
runtime/           Rust crate `lucid_rt`: allocator, counting, text, traps
std/               planned: standard library in Lucid
tests/             golden tests
benchmarks/        planned: Lucid and Rust versions of each program
```

## Runtime

`lucid_rt` is small and has a C ABI. It provides allocation, reference counting, text and list primitives, trap reporting, and task scheduling.

- `lucid run` links it directly, since the JIT runs inside the compiler process.
- `lucid build` embeds a prebuilt `lucid_rt` static library in the compiler and passes it to the system linker.
- The web target compiles it to `wasm32-unknown-unknown`.

## Allowed dependencies

| Crate | Purpose |
| --- | --- |
| `cranelift-codegen`, `cranelift-frontend`, `cranelift-module` | Native code generation |
| `cranelift-jit` | `lucid run` |
| `cranelift-object` | Object files for `lucid build` (added when needed) |
| `target-lexicon` | Target descriptions (added when needed) |
| `wasm-encoder` | WebAssembly output (added when needed) |

Cranelift crates are at 0.137, the latest stable release at M1. No parser generators and no CLI framework.

## Commands

Run from the repository root:

```sh
cargo run -p lucid -- run examples/file.lcd   # the compiler
cargo test                                    # unit and golden tests
cargo clippy --all-targets                    # lints
cargo fmt                                     # formatting
```

| Command | Behavior | State |
| --- | --- | --- |
| `lucid run file.lcd` | Compile with JIT and run | Works for `print` of plain text |
| `lucid check [--json] file.lcd` | Report diagnostics only | Syntax errors only. Takes a file until packages exist at M5 |
| `lucid build` | Produce a native executable | Planned |
| `lucid build --target web` | Produce WebAssembly and a loader | Planned |
| `lucid test` | Run `test` blocks | Planned |
| `lucid fmt` | Format, no options | Planned |
| `lucid fix` | Apply suggested fixes | Planned |
| `lucid explain CODE` | Explain an error code | Planned |
| `lucid context MODULE` | Print a compact API | Planned |
| `lucid doc` | Generate docs | Planned |

Exit codes: 0 for success, 1 for compile errors, 2 for bad usage, and 101 for a trap or a compiler bug. If the reader of compiler output has gone, the output is dropped and the exit code does not change.

## Testing

- **Golden run tests:** `tests/run/name.lcd` with `name.out` for expected output.
- **Golden error tests:** `tests/fail/name.lcd` with `name.err` for expected diagnostics, under `lucid run`.
- **Error code check:** `compiler/tests/golden.rs` fails if a code in the source has no golden test or is missing from `docs/agents.md`.
- **Examples:** everything in `examples/` must parse. Later milestones raise this to compile.
- **Ownership-off mode:** the whole suite runs with ownership optimizations disabled, and output must match.
- **Memory checks:** a debug runtime counts allocations, and reports leaks and double frees for each test.
- **Fuzzing:** add from M5. Parser and checker must never panic.

## Development machine

Checked on 2026-11-04, on Windows 11 x86-64:

- Rust 1.97.1, target `x86_64-pc-windows-msvc`. A test binary linked and ran.
- Visual Studio Build Tools and Windows SDK 10.0.26100 are installed.
- `rust-lld` ships with the Rust toolchain.
- The `wasm32-unknown-unknown` target is missing. Add it at M6: `rustup target add wasm32-unknown-unknown`.
- clang, gcc, and LLVM are not installed. The compiler does not need them.
- **Trap:** `/usr/bin/link.exe` in Git Bash is the GNU `link` tool, not the MSVC linker. `lucid build` must find the MSVC linker through `vswhere`, or use `rust-lld`.
