# Compiler Architecture

The compiler is written in Rust until M9, when its front end moves to Lucid (decision 16). The lexer, parser, type checker, and ownership analysis are hand-written. Machine code comes from Cranelift, and browser code comes from a WebAssembly backend.

## Status

M3 is in progress. `lucid run` compiles `int`, `float`, the sized numbers and wrapping math, `bool`, `none`, `text`, `list`, `map`, `set`, `type`, `choice`, `match`, unions, `is` with narrowing, interpolation, text slices, `let`, `var`, `if`, `while`, `for` over ranges, text, and collections, top-level tasks with recursion and defaults, generic tasks, types, and aliases, traits with bounds, `any` values, `self` with dot calls, and `print`. It reports E0901 for anything else. `check.rs` binds names, checks types, and lowers to MIR in one pass, because with no modules those stages have nothing to do alone. It splits when they arrive. A generic body is checked once, with each parameter as an opaque `Ty::Param`. Flattening means that only the real type arguments show what is a union and what is counted, so `check.rs` records intent as `Wrap`, `View`, `Member`, and `Holds`, and `mono.rs` picks the code for each, starting from every task with no type parameters. `Ty` keeps compound types behind leaked references, so it stays `Copy`. `Ty::Data` holds a name, an index into the checker's table, and type arguments, so a recursive type needs no cycle in `Ty`. A `match` lowers to an `if` chain on a hidden local, and the last arm has no test, since the exhaustiveness check proves what remains. `Ty::Union` holds flat members in a fixed order, so two spellings are equal. Narrowing is a stack of facts in the checker, cut back at the end of each block. An assignment resets entries in place, and a loop first resets each name its body assigns. List, map, and set tasks stay built into `check.rs`. They can move to Lucid once `change` parameters exist. `mono.rs` also replaces `==`, `<`, showing, and `in` on a compound value with a call: to a hand-written task if the type has one, or else to a task from `derive.rs`. It also gives each new map or set with a compound key its hash and equal tasks. `mono.rs` lists the types that reach each `any`. A task on an `any` tests its tag against that list, so `run` builds these tasks last, and again if building them boxed a new type. A task of a trait has a signature in `check.rs` and no body, and a call to one is `ExprKind::Method` until `mono.rs` knows the receiver. Trait checks are structural, so `Checker::satisfied` runs at each use, and walks a type's fields for a derived trait. `ownership.rs` is the naive stage: it gives each new heap value an owner, and "Counting" in `mir.rs` states the rules that a backend follows.

## Pipeline

```text
source text
  -> lexer          tokens with NEWLINE, INDENT, DEDENT
  -> parser         AST, with error recovery
  -> resolver       names bound across modules (in check.rs for now)
  -> type checker   inference, narrowing, exhaustiveness, MIR (check.rs)
  -> monomorphizer  concrete tasks and types only (mono.rs)
  -> ownership      owners of new values (naive). Moves, exclusivity, reuse are planned
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
        check.rs   names, types, lowering to MIR
        mono.rs    one concrete task per use of a generic
        derive.rs  derived tasks, as MIR for one concrete type
        mir.rs     typed, structured operations
        ownership.rs   owners of new heap values
        native.rs  Cranelift backend
        wasm.rs        planned: WebAssembly backend
    tests/golden.rs    runs everything in `tests/` and `examples/`
runtime/           Rust crate `lucid_rt`: allocator, counting, text, traps
std/               planned: standard library in Lucid
tests/             golden tests
benchmarks/        Lucid and Rust versions of each program
```

## Runtime

`lucid_rt` is small and has a C ABI. Showing, comparing, and hashing a compound value use no runtime type information: each is a derived task. Today it provides printing, trap reporting, allocation, reference counting, and text, list, map, and record primitives. Task scheduling is planned.

A task with counted locals has one exit block, which releases them, so code size does not grow with returns times locals. Every other task returns directly.

Every heap value starts with a count and a kind, so one `lucid_retain` and `lucid_release` serve all of them, and the kind tells `lucid_release` what to release inside. A text is one allocation: this header, a byte length, then its UTF-8 bytes. Every text literal allocates when it runs, except directly under `print`. A text slice, a list slice, or a loop character is a new value with its contents copied. `check.rs` lowers a `for` over text or a collection to a `while` over a hidden position.

Every integer lives in a 64-bit register, sign- or zero-extended from its own width. So a comparison, slot, or key needs no case per width, and only `u64` needs unsigned instructions. Math on a narrow type runs in 64 bits, where the result is exact, and traps if extending its low bits changes it. An `f32` is a real 32-bit float in registers, and its bits sit zero-extended in a slot.

A list is one allocation: the header, a length, a capacity, then 8-byte slots. Every element type uses one slot size, so `list[byte]` costs 8 bytes per byte for now. A map is one allocation too, with the layout of a compact dict: entries in insertion order, then a table of `2 * cap` indexes into them. A removed entry stays as a gap until the next rebuild. A set is a map with unused values. Hashes come from `RandomState`, keyed once per process. A map with a compound key also holds the addresses of two compiled tasks, which the runtime calls to hash a key and to compare two. An entry keeps its hash, so a rebuild calls neither. A derived hash starts from a state, and adds each length, tag, and part through `lucid_hash_int` and `lucid_hash_text`, each one a SipHash step. Entries of a map or set start alone, and `lucid_hash_sum` adds them, so order has no effect. `find` keeps no reference into the map during a task, since a key may hold that same map. A derived task reads a value by its position, not by its key, so it runs no key task for it: that keeps a key that is not equal to itself from trapping, and keeps nested maps from taking exponential time.

Compiled code reads a list's length and elements inline. Every change goes through a runtime task that takes the local's count and returns the value to store. That task is where copy-on-write happens: it writes in place if the count is one, and otherwise copies. So MIR changes a collection only through `Stmt::Update`, never in an expression. That is why `.pop` lowers to statements, like `match`, and can only be the whole value of one. `.get` changes nothing, so it stays an expression: `ExprKind::Seq` runs any setup, then an `If` tests and reads. A write through a path such as `x[i].push(1)` first reads every index and value, then takes `x[i]` out of its slot, which holds null until the write puts it back. So the element has one count, and needs no copy.

A `type` or `choice` value is a record: the header, a variant tag, a length, a count of counted slots, then 8-byte slots. Counted fields take the first slots, so a record needs no per-type description to know what it holds. Compiled code builds a record and reads its slots inline. A write goes through `lucid_record_set`, which copies if the record is shared. A variant with no fields allocates too, for now.

A union value is a record with one slot, and its tag is a type id: a position in the checker's list of union members. Ids mean the same in every union, so moving a value to a wider or narrower union needs no code, and `is` compares tags. A narrowed read loads the slot with no test, so the checker's narrowing rules are all that make it safe. An `any` value is the same record, and its tag can be a type that is in no union, or a whole union, which then holds another record.

`lucid_release` uses no recursion. A value whose count reaches zero waits on a list linked through its own count field, because a recursive `choice` can nest as deep as memory allows.

In a debug build of `lucid_rt`, which is what `cargo test` runs, a released value keeps its memory and is marked dead. So a double free, or a retain of a dead value, traps with `runtime bug`. `lucid run` also traps at exit if any value is still live.

A trap site is a cold block that passes `lucid_trap` a message holding its source position. Every task checks the stack pointer on entry, after its prologue, so its frame must fit in the 1 MiB margin. E0902 reads the compiled frame size, and rejects a task over half the margin. At M2 the limit is a constant in the code, because `lucid run` has one thread with a known stack. `lucid build` and `spawn` need a limit per thread.

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
sh benchmarks/run.sh                          # build, then 15 paired rounds, needs python
sh benchmarks/run.sh fib                      # one program
```

| Command | Behavior | State |
| --- | --- | --- |
| `lucid run file.lcd` | Compile with JIT and run | Works for the features under "Status" |
| `lucid check [--json] file.lcd` | Report diagnostics only | Syntax, name, and type errors, see `agents.md`. Takes a file until packages exist at M5 |
| `lucid build` | Produce a native executable | Planned |
| `lucid build --target web` | Produce WebAssembly and a loader | Planned |
| `lucid test` | Run `test` blocks | Planned |
| `lucid fmt` | Format, no options | Planned |
| `lucid fix` | Apply suggested fixes | Planned |
| `lucid explain CODE` | Explain an error code | Planned |
| `lucid context MODULE` | Print a compact API | Planned |
| `lucid doc` | Generate docs | Planned |

The `bench-unchecked` cargo feature removes `+`, `-`, and `*` overflow checks, only so that `benchmarks/run.sh` can measure their cost. It must never ship: `docs/security.md` promises traps in all builds.

Exit codes: 0 for success, 1 for compile errors, 2 for bad usage, and 101 for a trap or a compiler bug. If the reader of compiler output has gone, the output is dropped and the exit code does not change.

## Testing

- **Golden run tests:** `tests/run/name.lcd` with `name.out` for expected output.
- **Golden error tests:** `tests/fail/name.lcd` with `name.err` for expected diagnostics, under `lucid run`.
- **Golden trap tests:** `tests/trap/name.lcd` with `name.err` for the trap message, and exit code 101.
- **Error code check:** `compiler/tests/golden.rs` fails if a code in the source has no golden test or is missing from `docs/agents.md`.
- **Examples:** everything in `examples/` must parse. Later milestones raise this to compile.
- **Ownership-off mode:** the whole suite runs with ownership optimizations disabled, and output must match.
- **Memory checks:** the debug runtime counts live values, so every golden test also fails on a leak or double free.
- **Fuzzing:** add from M5. Parser and checker must never panic.

## Development machine

Checked on 2026-11-04, on Windows 11 x86-64:

- Rust 1.97.1, target `x86_64-pc-windows-msvc`. A test binary linked and ran.
- Visual Studio Build Tools and Windows SDK 10.0.26100 are installed.
- `rust-lld` ships with the Rust toolchain.
- The `wasm32-unknown-unknown` target is missing. Add it at M6: `rustup target add wasm32-unknown-unknown`.
- clang, gcc, and LLVM are not installed. The compiler does not need them.
- **Trap:** `/usr/bin/link.exe` in Git Bash is the GNU `link` tool, not the MSVC linker. `lucid build` must find the MSVC linker through `vswhere`, or use `rust-lld`.
