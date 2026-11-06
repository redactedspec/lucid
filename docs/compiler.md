# Compiler Architecture

The compiler is written in Rust until M9, when its front end moves to Lucid (decision 16). The lexer, parser, type checker, and ownership analysis are hand-written. Machine code comes from Cranelift, and browser code comes from a WebAssembly backend.

## Status

M4 is done, and M5 is next. `lucid run` compiles `int`, `float`, the sized numbers and wrapping math, `bool`, `none`, `text`, `list`, `map`, `set`, `type`, `choice`, `match`, unions, `is` with narrowing, interpolation, text slices, `let`, `var`, `if`, `while`, `for` over ranges, text, and collections, tasks with recursion and defaults, `change` and `take` parameters, `for change` over lists, lambdas, nested tasks, and task values, generic tasks, types, and aliases, traits with bounds, `any` values, `self` with dot calls, and `print`. It reports E0901 for anything else. `check.rs` binds names, checks types, and lowers to MIR in one pass, because with no modules those stages have nothing to do alone. It splits when they arrive. A generic body is checked once, with each parameter as an opaque `Ty::Param`. Flattening means that only the real type arguments show what is a union and what is counted, so `check.rs` records intent as `Wrap`, `View`, `Member`, and `Holds`, and `mono.rs` picks the code for each, starting from every task with no type parameters. `Ty` keeps compound types behind leaked references, so it stays `Copy`. `Ty::Data` holds a name, an index into the checker's table, and type arguments, so a recursive type needs no cycle in `Ty`. A `match` lowers to an `if` chain on a hidden local, and the last arm has no test, since the exhaustiveness check proves what remains. `Ty::Union` holds flat members in a fixed order, so two spellings are equal. Narrowing is a stack of facts in the checker, cut back at the end of each block. An assignment resets entries in place, and a loop first resets each name its body assigns. List, map, and set tasks stay built into `check.rs`. They can move to Lucid now that `change` parameters exist. `mono.rs` also replaces `==`, `<`, showing, and `in` on a compound value with a call: to a hand-written task if the type has one, or else to a task from `derive.rs`. It also gives each new map or set with a compound key its hash and equal tasks. `mono.rs` lists the types that reach each `any`. A task on an `any` tests its tag against that list, so `run` builds these tasks last, and again if building them boxed a new type. A task of a trait has a signature in `check.rs` and no body, and a call to one is `ExprKind::Method` until `mono.rs` knows the receiver. Trait checks are structural, so `Checker::satisfied` runs at each use, and walks a type's fields for a derived trait. `ownership.rs` gives each new heap value an owner, makes every count explicit with `Move` and `Retain`, and finds moves with a backward walk of what is live. First it turns each local that can only hold a path into a read parameter into `Stmt::Bind`, which holds no count, so `Task::borrowed` lists what no exit releases. The walk repeats until each loop head settles, which takes a pass per level of nesting. "Counting" in `mir.rs` states the rules that a backend follows. `check.rs` lets go of the names in a block with `Stmt::Clear`, and `ownership.rs` does the same for temporaries after their statement.

## Pipeline

```text
source text
  -> lexer          tokens with NEWLINE, INDENT, DEDENT
  -> parser         AST, with error recovery
  -> resolver       names bound across modules (in check.rs for now)
  -> type checker   inference, narrowing, exhaustiveness, exclusivity, MIR (check.rs)
  -> monomorphizer  concrete tasks and types only (mono.rs)
  -> ownership      owners of new values, last-use moves, borrow elision
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
        ownership.rs   owners and counts of heap values, last-use moves
        native.rs  Cranelift backend
        wasm.rs        planned: WebAssembly backend
    tests/golden.rs    runs everything in `tests/` and `examples/`
runtime/           Rust crate `lucid_rt`: allocator, counting, text, traps
std/               planned: standard library in Lucid
tests/             golden tests
benchmarks/        Lucid and Rust versions of each program
    agent-trial/   tasks, reference answers, and runner of the M3 trial
```

## Runtime

`lucid_rt` is small and has a C ABI. Showing, comparing, and hashing a compound value use no runtime type information: each is a derived task. Today it provides printing, trap reporting, allocation, reference counting, and text, list, map, and record primitives. Task scheduling is planned.

A task with counted locals has one exit block, which releases them, so code size does not grow with returns times locals. Every other task returns directly. Compiled code changes a count inline, and calls `lucid_release` only for the last one. A debug build calls the runtime for every count, so that its checks run, which means that `cargo test --release` is what tests the inline form.

Every heap value starts with a count and a kind, so one `lucid_retain` and `lucid_release` serve all of them, and the kind tells `lucid_release` what to release inside. A text is one allocation: this header, a byte length, a capacity, then its UTF-8 bytes. The kind also says whether a text is all ASCII, found on first need, which makes a character position a byte position. A text literal and a variant with no fields are static data, with a count of `1 << 62`, so nothing writes to them or frees them, and `ownership.rs` gives them no temporary. `lucid_rt` builds those bytes, so the layout stays in one crate. In `s = "{s}x"`, where the join is the last use of `s`, `ExprKind::Join` starts with a `Move`, and `lucid_text_append` grows `s` in place, doubling its room, one part at a time, and `lucid_text_append_int` writes an `int` part with no text of its own. A text slice, a list slice, or a loop character is a new value with its contents copied. `check.rs` lowers a `for` over text or a collection to a `while` over a hidden position.

Every integer lives in a 64-bit register, sign- or zero-extended from its own width. So a comparison, slot, or key needs no case per width, and only `u64` needs unsigned instructions. Math on a narrow type runs in 64 bits, where the result is exact, and traps if extending its low bits changes it. An `f32` is a real 32-bit float in registers, and its bits sit zero-extended in a slot.

A list is one allocation: the header, a length, a capacity, then 8-byte slots, or one-byte slots for a `list[byte]`, which the list knows, and which compiled code knows from the type. A map is one allocation too, with the layout of a compact dict: entries in insertion order, then a table of `2 * cap` indexes into them. A removed entry stays as a gap until the next rebuild. A set is a map with unused values. Hashes come from `RandomState`, keyed once per process. A map with a compound key also holds the addresses of two compiled tasks, which the runtime calls to hash a key and to compare two. An entry keeps its hash, so a rebuild calls neither. A derived hash starts from a state, and adds each length, tag, and part through `lucid_hash_int` and `lucid_hash_text`, each one a SipHash step. Entries of a map or set start alone, and `lucid_hash_sum` adds them, so order has no effect. `find` keeps no reference into the map during a task, since a key may hold that same map. A derived task reads a value by its position, not by its key, so it runs no key task for it: that keeps a key that is not equal to itself from trapping, and keeps nested maps from taking exponential time.

Compiled code reads a list's length and elements inline. A list set, a list push, and a field set are inline too. Each first makes room in a branch of its own, which calls the runtime only if the value is shared or full, and then stores on the common path. That split matters: Cranelift places a pure value where it is used, so a write with one arm that copies and one that does not would run its whole value in each arm. A value keeps its float registers through a load and a store for the same reason of cost. Any other change goes through a runtime task that takes the local's count and returns the value to store. Those tasks are where copy-on-write happens: each writes in place if the count is one, and otherwise copies. So MIR changes a collection only through `Stmt::Update`, never in an expression. That is why `.pop` lowers to statements, like `match`, and can only be the whole value of one. `.get` changes nothing, so it stays an expression: `ExprKind::Seq` runs any setup, then an `If` tests and reads. A write through a path such as `x[i].push(1)` first reads every index and value, then takes `x[i]` out of its slot, which holds null until the write puts it back. The slot's own count moves to a hidden local and back, through `Update::Take` and `Update::Put`, so no count changes, and a map step, which has no slot to move, retains instead. So the element has one count, and needs no copy.

### Parameters

A read parameter is a borrowed value. A `take` parameter of a top-level task arrives with a count from the caller, listed in `Task::take`. A lifted task, and a written `equal`, cannot get one, so each takes its own count when it starts.

A `change` argument is a pointer to a stack slot of the caller. The caller stores the local there, and loads it back after the call. The task loads it on entry, and stores it in its one exit block, so it is an ordinary variable in between, and its count moves with it. This is the copy-in copy-out of Swift `inout`, and the checker's exclusivity rules make it correct: nothing can see the value during the call. MIR marks this with `Task::change` and `ExprKind::Change`, which only `Call` holds. A task value has no such case, since a task with a `change` parameter has no value form.

A path such as `grid[i].row` uses the `Place` of a write. Its `setup` steps run for every argument, in order, with any other value moved to a hidden local, and then its `take` steps, the call, and `finish`. So the call lowers to `ExprKind::Seq`, and a hidden local keeps its result. Two paths through the same fields share a hidden local, or the second would write back a stale copy. Two paths with equal indexes would take one slot twice, so a test runs before any `take`, and `Stmt::Trap` reports it.

`for change` lowers to a `while` over a position, using one `Place` for the list. The loop name is a plain local, which is why narrowing, writes, and `change` work on it with no new code. `put_back` rewrites the body after lowering, so `break`, `continue`, and `return` first store it back. A `return` also runs the `Place`'s `finish`, after its value.

Exclusivity has three parts in `check.rs`, and all compare names and field paths, from `Expr::path`. A path stops at its first index. `check_write` looks at each write against plain `for` loops. `check_busy_reads` runs on each statement inside `for change`. `check_changed` runs when a statement ends, once dot calls have shown which receivers change, and compares each `change` with what `Stmt::values` holds. `check_overlap` compares the `change` arguments of one call by their `Place`.

### Task values

A lambda or nested task is lifted to a task of its own, after those of the file, by `check.rs`. It keeps the type parameters of the task around it. `Frame` holds the checker state of each task around the one being checked. A name found in one of them goes through every task between, as a capture of each. A task value is a record whose tag is the address of its code, and whose slots hold captures, counted ones first. So `lucid_release` and the record code need no new case. Its code takes that record as parameter 0, and reads a capture as a field of it. A nested task reads its own name as parameter 0, so recursion needs no capture. A top-level task used as a value gets a small lifted task that drops the record and calls it, unless it has a `change` parameter, which is an error. A call loads the tag, and makes an indirect call with a signature built from the type. A value with no captures allocates, for now.

A `type` or `choice` value is a record: the header, a variant tag, a length, a count of counted slots, then 8-byte slots. Counted fields take the first slots, so a record needs no per-type description to know what it holds. Compiled code builds a record and reads its slots inline. A write goes through `lucid_record_set`, which copies if the record is shared. A variant with no fields is static data.

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
cargo test --release                          # the same, with inline counts
cargo clippy --all-targets                    # lints
cargo fmt                                     # formatting
sh benchmarks/run.sh                          # build, then 15 paired rounds, needs python
sh benchmarks/run.sh fib                      # one program
ROUNDS=3 sh benchmarks/run.sh                 # fewer rounds
```

| Command | Behavior | State |
| --- | --- | --- |
| `lucid run file.lcd` | Compile with JIT and run | Works for the features under "Status" |
| `lucid check [--json] file.lcd` | Report diagnostics only | Syntax, name, and type errors, see `agents.md`. Takes a file until packages exist at M5 |
| `lucid check [--json] --costs file.lcd` | List copy and count sites | Works, from the MIR after `ownership.rs`. A task that `derive.rs` wrote stays out |
| `lucid build` | Produce a native executable | Planned |
| `lucid build --target web` | Produce WebAssembly and a loader | Planned |
| `lucid test` | Run `test` blocks | Planned |
| `lucid fmt` | Format, no options | Planned |
| `lucid fix` | Apply suggested fixes | Planned |
| `lucid explain CODE` | Explain an error code | Planned |
| `lucid context MODULE` | Print a compact API | Planned |
| `lucid doc` | Generate docs | Planned |

`run.py` also reports peak memory, on Windows only, with the peak of an empty program taken off the Lucid figure, since `lucid run` holds the compiler. The `LUCID_NAIVE` environment variable turns ownership optimizations off, for tests and for measuring them.

The `bench-unchecked` cargo feature removes `+`, `-`, and `*` overflow checks, only so that `benchmarks/run.sh` can measure their cost. It must never ship: `docs/security.md` promises traps in all builds.

Exit codes: 0 for success, 1 for compile errors, 2 for bad usage, and 101 for a trap or a compiler bug. If the reader of compiler output has gone, the output is dropped and the exit code does not change.

## Testing

- **Golden run tests:** `tests/run/name.lcd` with `name.out` for expected output.
- **Golden error tests:** `tests/fail/name.lcd` with `name.err` for expected diagnostics, under `lucid run`.
- **Golden cost tests:** `tests/costs/name.lcd` with `name.out` for what `lucid check --costs` reports.
- **Golden trap tests:** `tests/trap/name.lcd` with `name.err` for the trap message, and exit code 101.
- **Error code check:** `compiler/tests/golden.rs` fails if a code in the source has no golden test or is missing from `docs/agents.md`.
- **Examples:** everything in `examples/` must parse. Later milestones raise this to compile.
- **Agent guide:** every `lucid` block in `docs/agent-guide.md` must run with exit code 0, and the guide must stay under the token budget, at about 4 bytes per token.
- **Ownership-off mode:** the run and trap tests run a second time with `LUCID_NAIVE` set, which turns ownership optimizations off. Output must match.
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
