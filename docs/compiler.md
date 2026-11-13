# The Lucid Compiler: Implementation Notes

These notes explain how the compiler in this repository implements
[language.md](language.md): each pass, the runtime, the commands, how it
is tested, and how a release is made. They are for anyone changing the
compiler. Function and file names are given as they appear in the
source, as aids to finding the code, and comments in the source refer
back to sections here by name.

The compiler is written in Rust (decision 16). The lexer, parser, type
checker, and ownership pass are hand-written. Machine code comes from
Cranelift, and browser code from a WebAssembly backend. A second front
end, written in Lucid, lives in `compiler/lucid/`; see "Self-hosting".

## Status

Version 1.0.0. The Rust `lucid` compiles the whole language as
[language.md](language.md) defines it, except what is marked planned
there, such as `shared` and `weak`, which report E0901. `lucid run`
compiles a file or a package with Cranelift's JIT and runs it, and `lucid
build` writes a standalone program for Windows or Linux x86-64, or a
`.js` file for the web. Every test passes on Windows and in WSL.

The front end written in Lucid, under `compiler/lucid/`, runs `check`,
`run`, and `build`, and builds itself to a fixed point (decisions 214 to
226). The Rust `lucid` remains the program that ships.

## Pipeline

```text
source text
  -> lexer          tokens with NEWLINE, INDENT, DEDENT
  -> parser         AST, with error recovery
  -> resolver       modules joined into one tree, with unique names (resolve.rs)
  -> type checker   inference, narrowing, exhaustiveness, exclusivity, MIR (check.rs)
  -> monomorphizer  concrete tasks and types only (mono.rs)
  -> ownership      owners of new values, last-use moves, borrow elision
  -> backend        Cranelift (native, JIT) or wasm-encoder (web)
```

**Key decision:** MIR stays structured (blocks, ifs, loops) and is not a free control-flow graph. Lucid has no `goto`, so nothing is lost. WebAssembly needs structured control flow, so its backend stays small, and Cranelift can build its own graph from the same input. Both backends share one MIR.

## Repository layout

Start with few files. Split a file only when it hurts.

```text
compiler/          Rust crate `lucid`: the CLI and the front end
    src/
        main.rs    CLI
        context.rs public API for `lucid context`
        dump.rs    stage dumps for `LUCID_DUMP`, up to `resolved`
        fuzz.rs    test only: mutation fuzzing of the front end
        lexer.rs
        lsp.rs     language server for `lucid lsp`
        manifest.rs    reads `lucid.toml`
        ast.rs
        parser.rs
        resolve.rs joins modules into one tree
        check.rs   names, types, lowering to MIR
        mono.rs    one concrete task per use of a generic
        derive.rs  derived tasks, as MIR for one concrete type
        ownership.rs   owners and counts of heap values, last-use moves
    lucid/         the front end in Lucid, a package, by decision 214
    tests/golden.rs    runs everything in `tests/` and `examples/`
backend/           Rust crate `lucid_backend`: MIR and code generation
    src/
        lib.rs     linking for `lucid build`
        diag.rs    diagnostics, `Source`, and permission grants
        dump.rs    the text form of MIR, for `LUCID_DUMP`
        mir.rs     typed, structured operations
        native.rs  Cranelift backend
        wasm.rs    WebAssembly backend, for `--target web`
        loader.js  the JavaScript that `--target web` writes, with both modules
    build.rs       builds `lucid_rt` as a static library and as WebAssembly
runtime/           Rust crate `lucid_rt`: allocator, counting, text, traps
    src/exe.rs     start of a built program, only in the static library
std/               standard library in Lucid, held inside the compiler
tools/             `fmt.lcd`, the formatter in Lucid, held inside the compiler
tests/             golden tests
website/           lucidlang.dev: Astro and Starlight, built with npm, apart from cargo
benchmarks/        Lucid and Rust versions of each program
    agent-trial/   tasks, reference answers, and runner of the M3 trial
    agent-bench/   tasks, reference, runner, and scorer of the M5 agent benchmark
```

## Commands

Run from the repository root:

```sh
cargo run -p lucid -- run examples/file.lcd   # the compiler
cargo test                                    # unit and golden tests
cargo test --release                          # the same, with inline counts
LUCID_FUZZ=20000 cargo test fuzz              # a long fuzzing run
cargo clippy --all-targets                    # lints
cargo fmt                                     # formatting
sh benchmarks/run.sh                          # build, then 15 paired rounds, needs python
sh benchmarks/run.sh fib                      # one program
ROUNDS=3 sh benchmarks/run.sh                 # fewer rounds
python benchmarks/agent-bench/ref.py          # from that directory: rewrite expected/
sh try.sh lucid 07                            # in a benchmark workspace: one attempt, needs python, node, tsc
```

| Command | Behavior | State |
| --- | --- | --- |
| `lucid run [PATH [ARGUMENTS]]` | Compile with JIT and run. `ARGUMENTS` go to the program, through `lucid_rt::set_args`, since the process's own command line names the compiler | Works for the features under "Status" |
| `lucid check [--json] [PATH]` | Report diagnostics only | Syntax, name, and type errors, in the file or package and what it imports, see `agents.md` |
| `lucid check [--json] --unsafe [PATH]` | List `unsafe` blocks and `extern` tasks | Works, for a program and all it imports, dependencies too |
| `lucid check [--json] --costs [PATH]` | List copy and count sites | Works, from the MIR after `ownership.rs`. A task that `derive.rs` wrote stays out |
| `lucid build [--json] [PATH]` | Produce a native executable | Works on Windows and Linux x86-64, for the features under "Status". Writes `NAME.exe`, or `NAME` on Linux, to the current directory |
| `lucid build --target web [--json] [PATH]` | Produce WebAssembly and a loader | Works for every built feature but a user `extern` and `.pointer()`, which give E0905, and `spawn`, `std.net`, and `std.http`, which give E0907. Writes `NAME.js` to the current directory. See "WebAssembly" |
| `lucid test [--json] [PATH]` | Run `test` blocks | Works, for a file, or every file in a package's `src` |
| `lucid fmt [--check] [PATH]` | Format in place, with no style options. `--check` lists files that would change, and exits with 1 if any would | Works, for a file, or every file in a package's `src`. See "`lucid fmt`" |
| `lucid fix [--json] [PATH]` | Apply suggested fixes | Works. A `Diagnostic` holds `fixes`, each a set of edits. The command writes the edited text of each file back with its own line endings, from the text that `Source` holds |
| `lucid explain [--json] CODE` | Explain an error code | Works, from `docs/errors.md`, which the compiler holds through `include_str!` |
| `lucid context [--json] MODULE` | Print a compact API | Works for a file, a package directory, which means every file in its `src`, or `std.NAME`. `context.rs` reads tokens, not the tree, so a file with errors still shows its API |
| `lucid doc` | Generate docs | Planned |
| `lucid package [PATH]` | Write a library's `.lpk` archive and index line, for upload to a registry | Works. Checks the library first, writes `NAME-VERSION.lpk` to the current directory, and prints the index line. See "Registry" |
| `lucid --version [--json]` | Print the version | Works: `lucid 1.0.0`, or `{"version":"1.0.0"}` with `--json`. The text is `CARGO_PKG_VERSION`, and every crate has the same version, by decision 227 |
| `lucid lsp` | A language server on stdin and stdout | Works: diagnostics, formatting, go to definition, and hover. See "`lucid lsp`" under "Ecosystem" |

`run.py` also reports peak memory, on Windows only, with the peak of an empty program taken off the Lucid figure, since `lucid run` holds the compiler. The `LUCID_NAIVE` environment variable turns ownership optimizations off, for tests and for measuring them.

The `bench-unchecked` cargo feature removes `+`, `-`, and `*` overflow checks, only so that `benchmarks/run.sh` can measure their cost. It must never ship: `docs/security.md` promises traps in all builds.

`PATH` is a file or a directory. A directory means the nearest package at or above it, and no `PATH` means the current directory.

Exit codes: 0 for success, 1 for compile errors or a failed test, 2 for bad usage, and 101 for a trap or a compiler bug. If the reader of compiler output has gone, the output is dropped and the exit code does not change.

## Modules

`resolve.rs` loads the entry file and each `use ./` in turn, then rewrites each module so that the later stages see one file. `module.name` is the new name of each top-level name, and no written name has a dot, so nothing clashes. The module's name is its path from the entry file. In a program with imports the entry file gets a prefix too, its file stem, so a module cannot see its names, and messages show that form: `main.User`. A file alone keeps its names. `alias.name` becomes one name. A task with `self` keeps its name, and `check.rs` tests its visibility, since only a call's type finds it. `ast::short_name` gives the written name back where one is shown at run time or passed to C.

A file with a `use` that has no `./` finds its package by walking up to the nearest `lucid.toml`, so a dependency's files need no record of where they came from. `Loader::package` reads each manifest once, by real directory, so two spellings of a path are one package, and a diamond of dependencies loads one copy. A module of a package has the key `@name` and then its path, `lib` for `src/lib.lcd`, and no written name starts with `@`, so it cannot meet a local module of the same name. Messages, traces, and `--costs` show that key as its user writes it, through `diag::plain`. `manifest.rs` reads a small part of TOML by hand, see decision 121.

Every entry of every manifest read is checked, used or not, after the modules load. A dependency's path, `lucid.toml`, and `src` must match their spelling on disk exactly, by the rule in `exact_file`. `lucid test` on a package checks all files before it runs any test. `is_library` compares real paths, so `src/lib.lcd` follows the library rules when it is named directly too.

Permissions are checked in three places, by decision 146. `Loader::import_std` checks a `std` module's permission, and `Loader::check_allow` checks each dependency entry against both manifests. `resolve.rs` records the files of each package without `unsafe` in `Source::no_unsafe`, and `check.rs` reports E0340 at each `unsafe` block and `extern` in them. The root package's permissions go in `Source::grants`. `native.rs` passes its `files` and `env` lists, as lines of text from `manifest::runtime_grants`, to `lucid_rt::set_grants` under the JIT, and as the data `lucid_grants` in a built program, which `exe.rs` reads. The runtime checks each `std.fs` path and `std.env` name against them.

`Source` holds all the files end to end, entry file first, or the manifest and then the entry file for a package, so a span alone gives its file. `Lines::place` turns an offset into a file, line, and column.

## Checking and lowering

`check.rs` binds names, checks types, and lowers to MIR in one pass, since `resolve.rs` leaves it one tree.

A generic body is checked once, with each parameter as an opaque `Ty::Param`. Flattening means that only the real type arguments show what is a union and what is counted, so `check.rs` records intent as `Wrap`, `View`, `Member`, and `Holds`, and `mono.rs` picks the code for each.

`mono.rs` starts from the top level and each `test` block, and builds only the tasks they reach, through calls and task values, by decision 190. `lucid check` and `lucid fix` start from every task with no type parameters as well, so each one shows its errors and costs.

`Ty` keeps compound types behind leaked references, so it stays `Copy`. `Ty::Data` holds a name, an index into the checker's table, and type arguments, so a recursive type needs no cycle in `Ty`.

A `match` lowers to an `if` chain on a hidden local, and the last arm has no test, since the exhaustiveness check proves what remains. `Ty::Union` holds flat members in a fixed order, so two spellings are equal.

Narrowing is a stack of facts in the checker, cut back at the end of each block. An assignment resets entries in place, and a loop first resets each name its body assigns.

List, map, and set tasks stay built into `check.rs`. They can move to Lucid now that `change` parameters exist.

`mono.rs` also replaces `==`, `<`, showing, and `in` on a compound value with a call: to a hand-written task if the type has one, or else to a task from `derive.rs`. It also gives each new map or set with a compound key its hash and equal tasks.

`mono.rs` lists the types that reach each `any`. A task on an `any` tests its tag against that list, so `run` builds these tasks last, and again if building them boxed a new type.

A task of a trait has a signature in `check.rs` and no body, and a call to one is `ExprKind::Method` until `mono.rs` knows the receiver. Trait checks are structural, so `Checker::satisfied` runs at each use, and walks a type's fields for a derived trait.

`ownership.rs` gives each new heap value an owner, makes every count explicit with `Move` and `Retain`, and finds moves with a backward walk of what is live. First it turns each local that can only hold a path into a read parameter into `Stmt::Bind`, which holds no count, so `Task::borrowed` lists what no exit releases. The walk repeats until each loop head settles, which takes a pass per level of nesting.

"Counting" in `mir.rs` states the rules that a backend follows. `check.rs` lets go of the names in a block with `Stmt::Clear`, and `ownership.rs` does the same for temporaries after their statement.

## `unsafe`

An `extern` task has a signature and a place in `Program::tasks`, like any task, so calls need no new path. `check.rs` writes its body: one `return` of `ExprKind::Foreign`, which `native.rs` lowers to a call in the C ABI. Parameters stay 64-bit, since C reads the low part, and a result widens from its declared width. `find_symbol` looks in `ucrtbase` and `kernel32` on Windows, and through `dlsym` on Linux, in what is loaded and in `libm.so.6`, which `lucid` itself may not have loaded, and `check.rs` calls it, so a missing name is E0904 under `lucid check` too, and not a panic in Cranelift. `.pointer()` needs a name so that the list outlives the statement: `Stmt::Clear` releases it at the end of its block, not at its last use.

`lucid build` links `kernel32.lib`, and `msvcrt.lib`, which brings `ucrt.lib`, so what `find_symbol` finds also links. On Linux, `cc` links libc and the libraries from `--print native-static-libs`, which hold what the compiler itself has loaded.

## `std`

`resolve.rs` holds each file of `std/` through `include_str!`, and loads one as a module with the key `@std` and its name, from the path `@std/name.lcd`, so everything after that is the path of any module. No written name starts with `@`, so nothing clashes. Messages, traces, and `--costs` show the name as `std.name`. A `std` module not in the list leaves its `use` in the tree, which `check.rs` reports as E0901.

An `extern` task in `std` may start with `lucid_` and may pass counted values. It gets a borrowed value for each parameter, and gives a new value, so `ownership.rs` counts `ExprKind::Foreign` as new. `native.rs` lists these symbols with the rest of the runtime. So the system calls in `std` are Rust's, in `lucid_rt`, and M8 gets Linux and macOS with no new `std` code. `--costs` and `--unsafe` leave out sites in `@std/`.

A file task in `lucid_rt` keeps its failure in a thread local, and gives an empty value, so a call needs no result type of its own. `std.fs` asks for that failure next, and builds a `FileError`.

## `std.json`

`std/json.lcd` holds all that does not depend on a type: the parser, which builds a `Value` tree that `json.tree` makes public, by decision 238, reads of plain values, and generic reads and writes of lists, sets, and maps. `Json` is a built-in trait with no tasks, and `satisfied` in `check.rs` walks a type for it. `decode[T]` and `encode[T]` in that file have the body `...`, and they never compile: `mono.rs` turns a call to one into a call for its type, through `json_reader` and `json_writer` in `derive.rs`. Those pick a task of `std/json.lcd` by name, or `Derived::JsonRead` and `Derived::JsonWrite` for a `type`, `choice`, or union, which need its fields or members. A `Shape` keeps each field's default for this, and whether it is a `choice`.

A union picks its member from the tag of `Value`, through `json_tags` in `mir.rs`. So a read never goes back, and its time stays linear in the input. An error adds its step as it returns, so a read that works builds no path. Measured on a release build: 3 MB in 0.05 s to write, and 0.27 s to read into a `list` of a `type`.

## `test`

`check.rs` lowers a `test` to a task of its own, placed after the written tasks, only if the build keeps tests. `Frame::in_test` sends a failed `try` to `Stmt::Exit`, as at top level. `Program::tests` lists them.

A trap ends the process, and Cranelift frames have no unwind tables. So `lucid test` checks the file, then starts itself with `LUCID_TEST_FROM` set to the place of the first test to run. That child prints a NUL line before each test and after the last, and runs them in one process. The parent splits stdout at each NUL. With no final NUL, or an exit code that is not 0, the last test that started failed, and the parent starts a new child after it. A compile error that only code generation finds, E0902, comes from the child, in the form `--json` asks for. Passing tests cost one compile in total, and each failure costs one more.

## `expect`

`expect a == b` lowers to two hidden locals, an `if` on them, and `Stmt::Fail`, whose text is built only on the failing path. Any other condition uses `Stmt::Trap`.

## Early exits

`resolve.rs` puts a small Lucid prelude before each file. It declares `trait Error`, so nothing in the checker treats that trait as special, and an error type is one that satisfies it. It also holds `sort` on lists, a generic task in Lucid, since the runtime cannot call a hand-written `compare`. The prelude is a source file of its own, `@std/prelude.lcd`, so a trap site in it has a place. The text tasks `split`, `upper`, `lower`, and `to_int` lower in `check.rs` to `ExprKind::Foreign` calls into the runtime, so they need no MIR node or backend code of their own.

`try x` lowers to a `Seq`: a hidden local takes the value, an `if` tests `Holds`, and its branch is `Stmt::Return`, or `Stmt::Exit` at top level. `x else y` uses the same hidden local with `ExprKind::If`. `Frame::held` lists these locals, and `Checker::stmt` clears them after the statement, as does a `break` or `continue` inside it, since a count left there would make the next write copy. So a `Return` can sit inside an expression, and a statement can stop half done, which needs care in three places:

- **Values on their way to a keeper.** A record, list, map, or `take` argument holds a count outside any local until it is done. `Settle` in `ownership.rs` runs first. If a part can return, it runs that part and the ones before it into locals, in order. The exit of the task releases those, or a `Clear` after the statement.
- **Slots that are taken out.** `Stmt::Update` with `Take` must meet its `Put`. A `change` path already reads every other value before `take`, and `PutBack` in `check.rs` walks expressions, so a `for change` body puts its element back on this `return` too.
- **Blocks.** `ends` looks only at statements, so a `try` never counts as the end of a path, and a backend always has a block to go on with.

## Runtime

`lucid_rt` is small and has a C ABI. Showing, comparing, and hashing a compound value use no runtime type information: each is a derived task. Today it provides printing, trap reporting, allocation, reference counting, threads, channels, locks, and text, list, map, and record primitives. Sockets are built for `std.net`, see "Threads" below.

A task with counted locals has one exit block, which releases them, so code size does not grow with returns times locals. Every other task returns directly. Compiled code changes a count inline, and calls `lucid_release` only for the last one. A debug build calls the runtime for every count, so that its checks run, which means that `cargo test --release` is what tests the inline form.

Every heap value starts with a count and a kind, so one `lucid_retain` and `lucid_release` serve all of them, and the kind tells `lucid_release` what to release inside. A text is one allocation: this header, a byte length, a capacity, then its UTF-8 bytes. The kind also says whether a text is all ASCII, found on first need, which makes a character position a byte position. A text literal and a variant with no fields are static data, with a count of 0, which `lucid_retain` and `lucid_release` leave alone and which is never unique, so nothing writes to them or frees them, and `ownership.rs` gives them no temporary. `lucid_rt` builds those bytes, so the layout stays in one crate. In `s = "{s}x"`, where the join is the last use of `s`, `ExprKind::Join` starts with a `Move`, also when `s` is a `change` parameter or, through `Update::Take`, a field, by decision 222, and `lucid_text_append` grows `s` in place, doubling its room, one part at a time, and `lucid_text_append_int` writes an `int` part with no text of its own. A text slice, a list slice, or a loop character is a new value with its contents copied. `check.rs` lowers a `for` over text or a collection to a `while` over a hidden position.

Every integer lives in a 64-bit register, sign- or zero-extended from its own width. So a comparison, slot, or key needs no case per width, and only `u64` needs unsigned instructions. Math on a narrow type runs in 64 bits, where the result is exact, and traps if extending its low bits changes it. An `f32` is a real 32-bit float in registers, and its bits sit zero-extended in a slot.

A list is one allocation: the header, a length, a capacity, then 8-byte slots, or one-byte slots for a `list[byte]`, which the list knows, and which compiled code knows from the type. A map is one allocation too, with the layout of a compact dict: entries in insertion order, then a table of `2 * cap` indexes into them. A removed entry stays as a gap until the next rebuild. A set is a map with unused values. Hashes come from `RandomState`, keyed once per process. A map with a compound key also holds the addresses of two compiled tasks, which the runtime calls to hash a key and to compare two. An entry keeps its hash, so a rebuild calls neither. A derived hash starts from a state, and adds each length, tag, and part through `lucid_hash_int` and `lucid_hash_text`, each one a SipHash step. Entries of a map or set start alone, and `lucid_hash_sum` adds them, so order has no effect. `find` keeps no reference into the map during a task, since a key may hold that same map. A derived task reads a value by its position, not by its key, so it runs no key task for it: that keeps a key that is not equal to itself from trapping, and keeps nested maps from taking exponential time.

Compiled code reads a list's length and elements inline. A list set, a list push, and a field set are inline too. Each first makes room in a branch of its own, which calls the runtime only if the value is shared or full, and then stores on the common path. That split matters: Cranelift places a pure value where it is used, so a write with one arm that copies and one that does not would run its whole value in each arm. A value keeps its float registers through a load and a store for the same reason of cost. Any other change goes through a runtime task that takes the local's count and returns the value to store. Those tasks are where copy-on-write happens: each writes in place if the count is one, and otherwise copies. So MIR changes a collection only through `Stmt::Update`, never in an expression. That is why `.pop` lowers to statements, like `match`, and can only be the whole value of one. `.get` changes nothing, so it stays an expression: `ExprKind::Seq` runs any setup, then an `If` tests and reads. A write through a path such as `x[i].push(1)` first reads every index and value, then takes `x[i]` out of its slot, which holds null until the write puts it back. The slot's own count moves to a hidden local and back, through `Update::Take` and `Update::Put`, so no count changes, and a map step, which has no slot to move, retains instead. So the element has one count, and needs no copy.

### Parameters

A read parameter of a top-level task is a borrowed value. A `take` parameter of a top-level task arrives with a count from the caller, listed in `Task::take`. A written `equal` cannot get one, so it takes its own count when it starts. Code reached through a task value owns every argument, by decision 183: a call through a value gives each counted argument a count, and the code of a lambda, a nested task, or the small task in front of a top-level task moves each counted parameter to a local of its own when it starts. That convention is fixed, so the naive stage keeps those `take` lists. The runtime calls a task value the same way: `lucid_dom_fire` gives its text to the step.

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

A trap site is a cold block that passes `lucid_trap` a message holding its source position, and its frame pointer. Every task keeps frame pointers (`preserve_frame_pointers`), so a trace walks saved pointers and return addresses. `native.rs` hands the runtime a table from each call's return address to its task, line, and column, built from Cranelift's call sites and source locations, and text is made only on a trap. A task with no name, which `derive.rs` wrote, has a row with no text, so the walk goes through it and prints nothing. An address that is not in the table is a Rust frame, whose saved pointer cannot be trusted, so the walk stops. Every task checks the stack pointer on entry, after its prologue, so its frame must fit in the 1 MiB margin. E0902 reads the compiled frame size, and rejects a task over half the margin. Every compiled task takes the limit of its thread as hidden parameter 0, by decision 175, and passes it to each task it calls, so a task value's record is parameter 1. `native::run_entry` and `main` in `exe.rs` pass the limit of the main thread. A runtime task that calls compiled code, such as a key's `hash`, reads the limit from the thread local that `lucid_rt::set_stack_limit` sets. WebAssembly has no such parameter, since the engine finds an overflow.

### Threads

Built in M7, by decisions 169 to 177.

- **Counts.** Built. A count's sign tells who holds the value. `Head.count` is an `AtomicI64`, and a `Relaxed` load or store of it is a plain one. Inline `retain` adds 1 only to a positive count. Inline `release` subtracts 1 only from a count above 1. Otherwise each calls the runtime, unless the count is 0, since a variant with no fields is common in hot code. `lucid_retain` and `lucid_release` do nothing for 0, the count of static data, and use atomic operations on a negative count, with a release ordering on each decrement and an acquire fence before the free, as Rust's `Arc` does. That atomic path is a `#[cold]` function, since the order of the plain path cost up to 17% on `trees`, see "Signed counts (M7)" in `research.md`. Inline `is_unique` stays `count == 1`, so a marked value takes the runtime path. There `unique` makes an acquire load, and turns -1 back to 1, which allows the write. A copy because of sharing gives up the old count through `lucid_release`, since another thread may give up its own meanwhile. `lucid_mark` walks a value and all it reaches with a work list, not recursion, and negates each positive count. `spawn` calls it, and `send` and a lock will. `marked_counts_hold_across_threads` in `lucid_rt` shares a marked list among 8 threads. The WebAssembly backend uses the same layout and never marks.
- **Stack limit.** Built, see the trap sites under "Task values" above. A spawned thread calls `set_stack_limit`, and passes its own limit to the task it runs.
- **`spawn`.** Built, by decision 182. `check.rs` lowers `spawn f(x)` to a lifted task with no parameters, whose captures are the arguments, so they run in the spawning thread. Its code calls `f` and returns a list of one item, the result, so the runtime calls one shape of code for every result type. `lucid_spawn` retains and marks that record, and starts `std::thread::Builder` with a 64 MiB stack. It gives a counted `Ty::Spawned` value, which holds an `Arc` of the job, where the thread leaves the list. `lucid_wait` blocks on the job's `Condvar`, and gives a count of the list, which `.wait()` indexes. It marks the result first if the handle is marked, and `lucid_mark` on a handle marks a result already there, so no unmarked value reaches two threads. Each task, lambda, test, or top level that spawns has a hidden local, set on entry by `lucid_spawn_scope`, that holds the join handles of its threads. The exit block releases it like any counted local, and its last count joins them. Once it holds 16 threads, and then twice what was kept, `lucid_spawn` first joins and drops each thread that has ended and that no handle reaches, so a server loop stays small. A handle shows its type, like a task value, and has no other trait. `spawn` that is not given a call of a task, or that is given a `change` argument, is E0341. The WebAssembly backend reports E0907 at the first runtime call for threads.
- **Channels and locks.** Built, by decision 184. `Ty::Channel` and `Ty::Lock` are counted handles to a `Box` of a `Mutex` and one `Condvar`, so a copy shares them. Each value inside is a list of one item, so one runtime type holds any `T`, and `send` and a lock mark that list as it goes in. `check.rs` lowers every call: `send`, `close`, and `update` are statements, and `receive` is the whole value of one, like `.pop`, with a hidden list local. `read` gives a count of the lock's list, and `.read()` indexes it, as `.wait()` does. `update(f)` runs `lucid_lock_take`, which marks the lock as held by this thread and gives its list with the count, reads the item and drops the list, so the item has one count, calls `f` through its task value, which owns the item by decision 183, and gives `lucid_lock_put` a new list of what `f` returned. So a write in `f` is in place. A `read` waits while another thread holds the lock, and a thread that takes a lock it holds fails.
- **Waiting.** Built. `wait`, `send`, `receive`, a lock, and the join at the end of a task wait through `std::sync::Mutex` and `Condvar`, in `block`. One `AtomicU64` counts threads in its high half and waiting threads in its low half, so each change sees both. A thread that starts to wait adds 1, and traps if every thread now waits. A thread that ends checks the same after it subtracts itself. `wake` subtracts the threads that wait on a `Condvar` before it notifies them, and an epoch tells a woken thread from one that woke by itself, so no thread counts as waiting once something can run it. The web runs one thread, so a wait there is always a deadlock, and never reaches `Condvar::wait`.
- **Failures.** A runtime task for threads that finds a deadlock, a lock that this thread holds, a closed channel, or a capacity below 1 records why in the thread local `FAILURE`, and gives a static value with a count of 0. The compiled code calls `lucid_thread_failed` right after the call, and traps there through `Stmt::Fail` with `lucid_thread_failure`, so the trap shows the place of the call on both targets. A deadlock found at the join at the end of a task, or when a thread ends, has no compiled code to trap in, so it shows no place.
- **Traps.** Built. `trap` takes a lock on stderr, prints the message and the trace of its own thread, and ends the process with 101. Frame pointers work the same in every thread. The walk stops at the Rust frame that starts the thread.
- **Sockets.** Built, by decision 185. `Ty::Socket` is a counted handle that only a file under `@std/` can name, as `socket`. `std/net.lcd` wraps it in `Listener` and `Connection`, and calls `lucid_net_*` tasks. The runtime's `Socket` holds a `Box` of a `std::net` listener or stream, and `lucid_release` drops it, which closes it. A failed call gives `failed()` and keeps why in the thread local `NET_FAILURE`, which `std.net` reads, as `std.fs` does. A blocking call does not count as waiting. `use std.net` or `use std.http` under `--target web` is E0907, found in `main.rs` from `Source::host_only`, which keeps the module's name.
- **TLS.** Built, by decisions 179 and 187. `lucid_net_tls` gives a new socket whose endpoint is a `Tls`: a `rustls` client session behind a `Mutex`, and a second handle to the same TCP stream for timeouts. One `ClientConfig` is built on first use. The handshake runs inside the call. A read that blocks holds the lock, so a write from another thread waits, which a `ponytail:` comment marks. Only reachable tasks compile, by decision 190, so a program that never calls `secure` leaves most of `rustls` out of its `.exe`.
- **HTTP and HTML.** Built, by decisions 186 and 187, in Lucid only: `std/http.lcd` and `std/html.lcd` call no runtime code but `lucid_bytes_valid` and `lucid_bytes_text`, which `std.fs` already uses. The client in `std/http.lcd` reuses the server's header and chunk readers. `serve` holds a slot of a `channel[bool](256)` for each connection thread. After a refused request, `linger` closes the connection in stages, by decision 207, for at most 16 reads, through `lucid_net_close_write`, a runtime task that only `std/http.lcd` declares. A refusal that is a bug in the program, such as a `script` element or a header with a line break, traps through `expect`, with the reason as the compared text.

- `lucid run` links it directly, since the JIT runs inside the compiler process.
- `lucid build` embeds a prebuilt `lucid_rt` static library in the compiler and passes it to the system linker.
- The web target compiles it to `wasm32-unknown-unknown`, see "WebAssembly".

## `lucid build`

`native::object` lowers the program through the same `compile` as `lucid run`, into a Cranelift object file instead of memory. The top level is `lucid_main`. The target is the machine that runs `lucid`: `x86_64-pc-windows-msvc` or `x86_64-unknown-linux-gnu`, a constant per `cfg` in `native.rs`, by decision 191. On Linux the object file is position-independent, since `cc` makes a PIE by default. `backend/build.rs` builds `lucid_rt` a second time, as a release static library with the `exe` feature, which adds `exe.rs` and its C `main`. It runs its own `cargo` with a target directory of its own, so it waits on no lock of the outer build. The compiler holds that library through `include_bytes!`, writes it and the object file to a staging directory, and runs the linker: on Windows the MSVC `link.exe`, which `find-msvc-tools` finds along with its library paths, and on Linux the system `cc`. `stage` in `main.rs` makes the staging directory with a random name under the system temporary directory, through `DirBuilder::create`, which fails if the name exists, with mode `0700` on Linux. So no other user can have planted files in it, by decision 191. The system libraries that the static library needs come from rustc's `--print native-static-libs`. The library's C code, SQLite and `ring`, builds with `/Gw` on Windows, and `cc` links with `--gc-sections` on Linux, so the linker drops the parts a program never reaches, by decision 209. On Linux `cc` also gets `-Wl,--strip-debug`, so a program carries no DWARF but keeps its symbols, by decision 228.

`main` in `exe.rs` works out the stack limit, reads the call site table from the object file, runs `lucid_main` with the limit, and ends through `std::process::exit`, which flushes stdout. The table is `lucid_call_sites`: a count, then rows of `SITE_WORDS` words, with names in `lucid_call_names`. The task address in a row is a relocation, so the linker fills it in. On Windows the linker gives the main thread `EXE_STACK_BYTES`, 256 MiB, which is reserved, not committed. Linux takes the main thread's stack from `ulimit`, so there `main` starts a thread of `EXE_STACK_BYTES` for the program, and waits on it. It first ignores SIGPIPE, as Rust's own `main` does, so a write to a closed socket fails with `Closed` and does not end the program. The top level may use half of the stack, as under `lucid run`.

The object file targets baseline x86-64, not the features of the machine that builds it, so the program runs on any x86-64 machine of the same system. It is named after the file, or the package directory, with `.exe` on Windows and no extension on Linux, and goes in the current directory. If a directory there has that name, as when `lucid build app` runs beside `app/` on Linux, `lucid` refuses, and does not link. A failure to find or run the linker is not an error in the program, so it has no code: `lucid` prints the reason and exits with 1. A built program has no leak check, since its runtime is a release build.

## WebAssembly

By decisions 156 to 164 and 166 to 168. Built: the layout, the runtime on `wasm32`, `wasm.rs`, the loader, `lucid build --target web`, and `std.dom`. Every run and trap test runs under Node, except the ones that need the host. The example page `examples/web` runs in Chrome.

**Layout.** The header fields that compiled code reads are `u64` on every target, so `COUNT_OFFSET`, `LEN_OFFSET`, `SLOTS_OFFSET`, `TAG_OFFSET`, and the images from `static_text` and `static_record` hold for `wasm32` too. `web_runtime_loads` reads a text's and a record's header in the WebAssembly runtime at those offsets. `wasm.rs` uses the host's `lucid_rt::static_text` for text literals.

**Two modules.** `backend/build.rs` also builds `lucid_rt` for `wasm32-unknown-unknown`, on every host, as a `cdylib` linked with `--export-table --growable-table`, and gives its path in `LUCID_RT_WASM`. It exports `memory`, `__indirect_function_table`, and every `lucid_` task. `wasm.rs` holds it through `include_bytes!`, and writes the program module with `wasm-encoder`. The program imports `rt.memory`, `rt.__indirect_function_table`, `env.memory_base`, `env.table_base`, and every task in `IMPORTS`. It puts its static data at the memory base, as one data segment, and every task at the table base, as one element segment. The loader starts the runtime, grows its memory by the pages that the data needs, and grows its table by the number of tasks. It then starts the program with the old ends as the bases. The runtime's allocator never hands out those pages, by decision 166. The program exports `main`, the top level. A key task's address, and the tag of a task value, is the task's index in the shared table. The runtime calls a key task through the table, as it calls a function pointer on native, and a call to a task value is a `call_indirect`. Index 0 of Rust's table is null, so no task has it.

**Values.** A local has the type that native gives the value, by decision 166: an integer or an address is an `i64`, a float an `f64` or `f32`, and a `bool` or `none` an `i32`. An address is zero-extended into an `i64`, and wrapped to `i32` for a load, a store, or a pointer parameter of a runtime task. `IMPORTS` in `wasm.rs` lists every runtime task with the form of each parameter and result, and the program imports all of them, so a wrong entry fails when any program loads. WebAssembly has no overflow flag, so a checked `+`, `-`, or `*` reads its operands and the wrapped result from locals, and a narrow type checks its result as native does. A text join appends each part to a new empty text, since native's array of parts on the stack would need memory of its own. A task with `change` parameters returns several values, by decision 167: its result, then the value of each `change` parameter as it ends. The caller sets each local from them. Counts change through `lucid_retain` and `lucid_release`, and a write tests the count inline before it copies, as native does. A foreign call names a runtime task, since `std` and the prelude call only those.

**Control flow.** MIR is structured, so each `if`, loop, `break`, and `continue` is a WebAssembly block and a branch to it. An `if` chain is one block with a branch out of each arm, so its length adds no depth. A task's body is one block. A `return` stores its value in a local and leaves that block, and the code after it releases the owned locals once, as native's exit block does.

**Runtime.** Built. `runtime/src/web.rs`, under `cfg(target_arch = "wasm32")`, does only what Rust's standard library cannot there: print, trap, and exit through JavaScript imports, `time.now` and `time.clock` from `Date.now` and `performance.now`, a trap from `time.sleep`, and hash keys from `crypto.getRandomValues` through `SipHasher::new_with_keys`. `std.fs` gives `Failed`, `env.get` gives none, and `args.all()` gives `[]`, with no new code. The loader must give these imports, all in module `js`:

| Import | Does |
| --- | --- |
| `write(stream, at, len)` | writes bytes to stdout if `stream` is 1, or stderr if 2. A line may come in more than one write |
| `trap(at, len)` | reports a trap: the message, then the trace. Never returns |
| `exit(code)` | ends the program. Never returns |
| `now()` | `Date.now()`, in milliseconds |
| `clock()` | `performance.now()`, in milliseconds |
| `random(at, len)` | fills bytes from `crypto.getRandomValues` |
| `dom_create(at, len)`, `dom_text(at, len)` | makes an element or a text node, and gives its number |
| `dom_set_text`, `dom_set`, `dom_unset` | changes a text node, or sets or removes an attribute. `value` also sets the property |
| `dom_on(node, at, len, handler)`, `dom_off` | sets or clears the handler number for an event of a node |
| `dom_append`, `dom_replace`, `dom_remove` | moves nodes. Parent 0 is `document.body` |
| `dom_value_len()`, `dom_value(at)` | the event's key or value, as UTF-8, for the handler running now |

**Pages.** `std/dom.lcd` holds the node tree and the diff in Lucid, and reaches the page through the `lucid_dom_` tasks of `web.rs`, which pass text to the `dom_` imports. A node is a number that the loader gives, kept in a `Map`. `dom.app` keeps a `task(int, text)` with `lucid_dom_step`. The loader's listener for an event calls the runtime export `lucid_dom_fire` with the handler's number. That runs the kept task through the table, which updates the model, patches the page, and keeps the next task. An event that starts during a handler waits in a queue, and then reads its handler's number from the page that the handler left, so Lucid code never runs inside itself. A trap in a handler prints as at top level, and the page takes no more events. `lucid_dom_create` and `lucid_dom_set` trap on what decision 168 forbids. `resolve.rs` puts the place of the first `use std.dom` in `Source::web_only`, and `main.rs` reports E0906 there before native code is made.

A Rust panic in the runtime is a WebAssembly `unreachable`, which the loader sees as a `RuntimeError`. The loader prints it as `trap: ` and the error, and exits with 101.

**Output.** Built. `lucid build --target web app.lcd` writes `app.js` to the current directory: `backend/src/loader.js`, with both modules inside as base64, and the size of the static data. The loader compiles both with `WebAssembly.instantiate`, which a browser allows for a large module on its main thread. A page loads it with `<script src="app.js">`, which also works from `file://`. Under `node app.js`, print goes to stdout through `fs.writeSync`, a trap exits with 101, and a failed top-level `try` with 1. In a browser, each whole line goes to `console.log`, or `console.error` for stderr.

**Traps.** A trap prints the message and trace that native prints. `wasm.rs` records each call to a task, by function index and offset in the module, with its file, line, column, and calling task, or an empty text for a derived task, which a trace leaves out. The offset counts from the start of the module, as V8 and Firefox give it in each `wasm-function[i]:0xOFF` frame of `Error.stack`. The loader sets `Error.stackTraceLimit` to at least 64. In `js.trap`, it reads the stack of a new `Error`, skips the runtime's frames, whose names hold `lucid_rt`, and the first program frame, which the message already places. It shows the calls under it, at most 16, and stops at a frame that is not a recorded call, as native stops at a frame of the runtime. An engine stack overflow is a `RangeError` in V8 and an `InternalError`, "too much recursion", in Firefox. The loader then prints the place of the task in its innermost program frame, `trap: stack overflow`, and the same trace.

**Errors.** A user `extern` and `.pointer()` are E0905 under `--target web`, at the first place found. `use std.dom` is E0906 under `lucid run`, `lucid test`, and a native `lucid build`.

**Tests.** `web_programs_match_run` builds every run and trap test with `--target web` and runs it under `node`, against the same golden files. A missing `node` fails. Only the tests in `HOST_ONLY` are left out, since they read files, arguments, or the environment, sleep, or call a user `extern`. On 2026-11-07, 96 tests ran. `dom_pages_match` builds each page in `tests/dom`, and runs it under `node tests/dom/page.js`, a small fake `document` that plays the events in `x.events` and prints the page after each, with the number of nodes made, so a test shows what a patch keeps. `web_examples_build` builds each page in `examples/web`.

## `lucid fmt`

Built, by decisions 144 and 145.

**Shape.** The formatter is a Lucid program, `tools/fmt.lcd`, held inside the compiler through `include_str!` at the path `@tools/fmt.lcd`, as `std` modules are. `lucid fmt [--check] [PATH]` finds the files as `lucid test` does: a file, or every file in a package's `src`. It lexes each file with `lexer.rs` first. A file with a lexer error gets its diagnostics, as under `lucid check`, and is left alone. The rest go to the formatter in a child process of `lucid`, marked by the `LUCID_FMT_CHILD` environment variable, which compiles it with the JIT and passes `--check` if given, then the paths, through `lucid_rt::set_args`. A trap or a failed `try` ends the child, not the parent, so the safety net below still runs.

The formatter reads each file as bytes and builds the output as a `list[byte]`. All syntax is ASCII, so a byte scan finds every token boundary. Text and comments are copied as bytes. It writes a file only if the output differs. With `--check` it writes nothing, prints each path that would change, and ends with a top-level `try` of an error, which exits with code 1.

**Safety net.** Before the run, `lucid` keeps the text of each file. After it, whatever the child's exit code, `lucid` checks each changed file: it must be UTF-8, lex to the same token kinds, spans aside, and hold the same characters other than whitespace and a byte order mark, which covers comments. Any other change is a formatter bug: `lucid` writes the old text back and names the file, for every such file, then exits with 101.

**Rules.** The formatter changes only space and line breaks. It never joins, splits, or reorders lines or tokens.

1. Line breaks are `\n`. A byte order mark is dropped.
2. Trailing spaces are removed, except inside `"""` text.
3. Blank lines at the start of the file go. A run of blank lines becomes one. A blank line goes if the next code line is indented deeper than the previous code line, which is the first line of a block. A comment line counts as a code line here.
4. A file that is not empty ends with one `\n`.
5. A line outside brackets keeps its indentation, which the lexer has already checked.
6. A line inside brackets is indented 4 spaces past the line that opened the innermost open bracket. A line that starts with closing brackets is indented as the line that opened the first of them. So all three statements below are canonical:

   ```lucid
   let point = Point(
       x: 1,
       y: 2,
   )
   print sum([
       1,
       2,
   ])
   call(a, [
       b,
   ], c)
   ```

7. A comment line keeps its indentation, cut to a multiple of 4. Inside brackets, it follows rule 6.
8. Between two tokens on a line there is one space, except:
   - none after `(`, `[`, `{`, `.`, `./`, or a prefix `-`;
   - none before `)`, `]`, `}`, `,`, `:`, `.`, or `?`;
   - none on either side of `..` or `..=`;
   - none before `(` or `[` after a name that is not a keyword, `self`, `task`, a text, `)`, `]`, or `}`. So `f(x)`, `items[0]`, `task(int)`, `f[int](x)`, and `{"a": 1}["a"]`, but `if (a)` and `in [1]`.

   A `-` is a prefix when it starts the line, or follows an operator other than `?`, an opening bracket, `,`, `:`, or a keyword other than `true`, `false`, `none`, and `self`.
9. A comment after code has two spaces before `#`.
10. Text, numbers, and comment text are copied unchanged. A text that spans lines, in `"""` form or through a `{}` with a line break, is copied whole, line breaks and indentation included.

Line length has no limit, and no line is wrapped. Trailing commas, `use` order, and alignment are left as written.

**Tests.** `tests/fmt/name.lcd` with `name.out`, the expected output. Every `.lcd` file in `std/`, `tools/`, and `examples/` must already be formatted, which `lucid fmt --check` tests. A second run over each `.out` file must change nothing. Line endings and a byte order mark are tested from generated files, since git may change them. The safety net's check is unit-tested in `main.rs`, and the whole net was tried by hand with a formatter that drops all spaces.

## Ecosystem

Built in M8, by decisions 188 to 213.

- **Reachable tasks.** Built, by decision 190, see "Pipeline" above. The web loader calls only `main` and the runtime's `lucid_dom_fire`, which calls task values that `main` reached, so the top level and tests are the only roots. A task that nothing reaches is not compiled, so it cannot fail as E0902, or as E0907 for `spawn`. `examples/website.lcd` built to 2,067,968 bytes before, and 933,888 after.
- **Systems.** Built, by decision 191, see "`lucid build`". `TARGET` is a constant per `cfg`, not read through `target-lexicon`, since only two targets exist and that crate is not a direct dependency. Only Windows and Linux on x86-64 are supported, by decision 205.
- **`std.crypto` and `std.sql`.** Built. Runtime tasks `lucid_crypto_*`, in `runtime/src/crypto.rs`, and `lucid_sql_*`, in `runtime/src/sql.rs`, in `lucid_rt`, on native targets only, behind Lucid modules in `std/`. `Ty::Sql` is a type that a text literal with no interpolation takes from its context, as a number literal does, in `check.rs` `lower`, and `sql + sql` lowers to `ExprKind::Join`. Every backend treats it as `text`. It has only `Show` and `Equal`. `Ty::Database`, which only `std` can name as `database`, is a counted handle, `Kind::Sql` in the runtime, for a connection or a statement: the last count calls `sqlite3_close_v2` or `sqlite3_finalize`. The handle reaches its release task through a field, so the release code of every program does not name SQLite. Each call holds the connection's mutex, so its failure, kept in the thread local `SQL_FAILURE`, comes from that call. An authorizer refuses `ATTACH` of any name but the literal empty one that `VACUUM` uses, a null name included, and the pragmas that move SQLite's files. A second `sqlite3_prepare_v2` on the tail of a query must give no statement.
- **`std.process` and `std.console`.** Built in M13, by decisions 235 and 236. `lucid_console_*` in `runtime/src/lib.rs`, on every target: on `wasm32`, Rust's stdin is always at its end. `lucid_process_*` in `runtime/src/process.rs`, on native targets only. A started child is a `socket` handle whose endpoint is `Endpoint::Child`, so no compiler change was needed; `std.process` keeps it in a field of `Child`, so a program cannot pass it to `std.net`. Each pipe and the process have their own mutex. `run` drains stdout and stderr on threads, writes the input on a third, and with a timeout polls `try_wait` every 10 ms. A line read sets the thread local `INPUT_ENDED`, which `console.read_line` and `Child.read_line` both read. Tests: `console_reads_lines`, and `processes_run`, which drives `tests/process/child.lcd` from `parent.lcd`.
- **Orphan rule.** Built, by decisions 196 and 210. `resolve.rs` records the package of each file in `Source::packages`, and `Checker::check_orphan` compares the package of a task with `self` against the package of its receiver's `type`, `choice`, or `any` trait, and of each trait that names the task. Files of `std`, and of a program with no `lucid.toml`, have no package and are exempt.
- **Registry.** Built, by decisions 197 to 201 and 211. `manifest.rs` reads `version` dependencies and `[registry] url`. `resolve::load` reads the root's `lucid.lock` first, and points each registry dependency at its cache directory, `CACHE/NAME-VERSION-SHA256/`, when the lock holds a version in the floor's range, at or above it, and the directory holds a `lucid.toml`. The cache is `LUCID_CACHE` if set, or else `%LOCALAPPDATA%\lucid`, or `$XDG_CACHE_HOME/lucid` or `~/.cache/lucid`. A cached package is then a path dependency like any other, except that it may not have a `path` dependency, and must hold the locked version. If a dependency is not covered, `load` runs `lucid fetch CACHE REGISTRY LOCK NAME@VERSION...` in a child, with `LUCID_FETCH_CHILD` set, and loads again. The child compiles `tools/fetch.lcd`, held through `include_str!` like `tools/fmt.lcd`, which reads each index with `std.http` or `std.fs`, picks versions by minimal version selection, checks hashes, unpacks into a `.part-` directory and renames it, and writes the lock. It prints `note TEXT`, which goes to stderr, and `error CODE NAME TEXT`, which becomes a diagnostic on the manifest entry of `NAME`. `lucid package`, in `main.rs`, runs `lucid check`, writes the archive with `diag::json_string`, and hashes it with `lucid_rt::sha256_hex`. Test: `registry_fetches_and_locks`, with a directory registry.
- **`lucid lsp`.** Built, by decisions 202 and 212, in `lsp.rs`, with its own small JSON reader. It reads `Content-Length` frames from stdin, writes them to stdout, and keeps the text of each open document in `resolve::OPEN`, by canonical path, which `Loader::read` takes in place of the file. After each open, change, or close it checks every open document as the entry of `lucid check`, through `mono::run`, and sends each file's diagnostics, with an empty list for a file that had some before. An error in a `std` file shows at the start of the document. It never runs the registry client, which would write `lucid.lock` and wait on the network at each keystroke: a dependency that the lock and the cache do not cover is E0343, with a note to run `lucid check`. Go to definition and hover read `Program::refs`: through `check::check_with` with `index`, which only the server sets, `check.rs` records each written name it resolves, a binding or parameter, a task, a method, or a type or constructor, with the span of its declaration, and a binding's type. Hover shows a binding as `name: type`, and a task or type as its declaration's line with the `##` lines above it. Formatting writes the text to a file in a new private directory, as `lucid build` stages, runs `lucid fmt` on it in a child, and gives one edit for the whole text. A panic while checking is caught, and that document gets no results. Positions count UTF-16 units. Test: `language_server_answers_an_editor`. Checked in Neovim 0.12.4.

## Self-hosting

Built in M9, by decisions 214 to 226, in the order of `roadmap.md`: the dumps, and the lexer, parser, resolver with `lucid.toml` reading, type checker, monomorphizer with the derived tasks, and ownership pass in `compiler/lucid/`, whose `check` with `LUCID_DUMP=tokens`, `ast`, `resolved`, `checked`, `mir`, or `owned`, with or without `LUCID_NAIVE`, and whose `check --costs` and `check --unsafe`, match the Rust output, diagnostics included, on every repository file and package. Its `run` and `build` pass the golden tests of those commands. The Lucid resolver reads `std` files from the directory in the internal variable `LUCID_STD`, or else `std/` under the current directory, until Lucid can make text from the bytes of the files held inside the backend, by decision 226, and reports a registry dependency that the lock and the cache do not cover as E0343, since only the Rust `lucid` fetches, by decision 214.

- **Shape.** `compiler/lucid/` is a Lucid package that holds the front end, one module per Rust file it ports, except `mono.lcd`, which holds both `mono.rs` and `derive.rs`, since each calls the other and modules cannot import in a cycle, and a driver for `check`, `run`, and `build`. The Rust `lucid` stays the shipped program, with its own front end, and keeps every other command, by decision 214.
- **Dumps.** Built. `LUCID_DUMP=tokens`, `ast`, `resolved`, `checked`, `mir`, or `owned` makes `lucid check` print that stage on stdout, before any diagnostics, in a text form that both compilers write, by decision 215. `dump.rs` writes it. `tokens` and `ast` cover the named file alone, lexed and parsed with no imports, through `resolve::parse_file`, and print even when that file has errors. `resolved` is what `resolve.rs` gives for the whole program, by decision 223: the files read, in order, with where each starts, the grants, which files lack `unsafe`, which package holds each file, the first `use std.dom` and host-only `use`, and then the joined tree, or `-` if loading stopped before it. It prints even with errors. `checked` is the program that `check.rs` gives, before `mono.rs`, with type parameters still in it, by decision 225. The stage ends with the checker, so only its diagnostics print. `mir` is the program after `mono.rs`, and `owned` after `ownership.rs`, each printed, like `checked`, only if the program has no errors so far. `owned` starts with the `Source` the backend needs: its files, their text, and the grants. An empty value counts as unset. The variable is internal.
- **Dump form.** One node per line, indented two spaces per level. A line is the node's name, which is the Rust variant's name, then its atoms, separated by spaces, and its children follow one level deeper. An atom is a word, a number, `true`, `false`, `-` for a value that is absent, or a text in double quotes, where only `\`, `"`, line feed, carriage return, tab, and other control characters are escaped, the last as `\u{hex}`. A span is two atoms, start and end. A list of atoms is its length, then its items. A list of nodes puts its length on the parent's line, and the nodes follow as children, in field order. An absent child is a line `-`. A type is atoms in prefix form, such as `map text list int`, `data "main.User" 3 0`, or `task 1 int none`. A float is written as Lucid shows it. Indentation and line breaks only help the reader, so a reader can take the whole dump as one stream of atoms.
- **Dump test.** `stage_dumps_are_stable` runs each stage on every `.lcd` file in `tests/`, `examples/`, `std/`, `tools/`, and `compiler/lucid/`, twice, with `--json`, and fails on a panic or when the two runs differ. It first builds `compiler/lucid/` with the Rust `lucid`. It also runs `owned` with `LUCID_NAIVE` as `naive`, and `check --costs` and `check --unsafe` as `costs` and `unsafe`. The second run is that Lucid compiler, so its output must match the Rust one. Each `owned` and `naive` dump with no errors must also read back through `dump::read` and print the same text again.
- **Backend.** Built. The crate `backend/`, `lucid_backend`, holds `diag.rs`, `mir.rs`, the MIR half of `dump.rs`, `native.rs`, `wasm.rs`, `loader.js`, and the linking code, and `lucid` depends on it. `diag.rs` came along, since `Source` and its grants are what a backend reports against, and `BinaryOp` moved from `ast.rs` to `mir.rs`. `dump::read` reads the `owned` dump back into `Program` and `Source`, by decision 216, with leaked names and types, as `check.rs` makes them. Its C ABI is in `lib.rs`, by decisions 217 and 226. `backend_run` takes `JSON ARGS "ARG"...` and then the dump, and runs the program on a thread with the stack of `lucid run`. `backend_build` takes `JSON WEB "OUT"` and then the dump, and writes the program or the script. `backend_has_symbol` takes a name, for E0904. Each takes a `.pointer()` and a length, gives an `int`, and prints its own diagnostics. `compiler/lucid/src/backend.lcd` declares them, and `main.lcd` follows the Rust driver, and ends with C `exit(1)` when the backend reports a problem. `native::find_symbol` finds these tasks first, so the Lucid compiler runs under `lucid run`, and passes E0904. `native::object` tells whether a program calls one. Then `link` takes `LUCID_BACKEND_LIB`, a static library of `lucid_backend` with the runtime and `exe.rs`, which `build.rs` builds in release with a second cargo, marked by `LUCID_BACKEND_INNER` so that it does not start a build of itself. This adds about 3.5 minutes to the first `cargo build`. A library cannot hold its own bytes, so `link` reads it from the build directory, with its system libraries and their directories from `libs.txt` beside it, since `windows-targets` puts its own library in a directory that only cargo knows. The runtime's arguments, grants, and call sites can be set again, since the compiler runs a program in its own process, and the debug leak check counts only values made since the program started. Planned: the files held inside, by decision 226.
- **Fixed point.** Built. Stage 0 is the Rust `lucid`, and each later stage is built by the one before it. Stages 1, 2, and 3 are byte-identical. `lucid build` passes `/Brepro` to `link.exe`, since the PE timestamp was the one difference between two builds, by decision 218. `compiler_reaches_fixed_point` in `golden.rs` builds stages 2 and 3 from the stage 1 of the other tests, and compares them. It compares stage 1 too only under `cargo test --release`, since a debug `lucid` changes counts through runtime calls, so its stage 1 differs throughout. It passes on Windows and in WSL.
- **Bootstrap.** Until a release exists, stage 0 comes from `cargo build --release` in the same checkout, by decision 219. From the root of the checkout, with `LUCID_STD` naming its `std/` directory:

  ```sh
  cargo build --release
  mkdir -p stage1 stage2
  (cd stage1 && ../target/release/lucid build ../compiler/lucid)
  (cd stage2 && ../stage1/lucid build ../compiler/lucid)
  cmp stage1/lucid stage2/lucid
  ```

  On Windows the programs end in `.exe`. A built compiler reads its backend library from `target/`, by decision 226, so it runs only in the checkout that built it. The `compiler/lucid/stage0.txt` pin waits until the Rust front end is deleted, by decision 229.

## Allowed dependencies

| Crate | Purpose |
| --- | --- |
| `cranelift-codegen`, `cranelift-frontend`, `cranelift-module` | Native code generation, in `lucid_backend` |
| `cranelift-jit` | `lucid run` |
| `cranelift-object` | Object files for `lucid build` |
| `find-msvc-tools` | Finds the MSVC linker and its library paths for `lucid build`, on Windows only (decision 139). Linux runs the system `cc` |
| `target-lexicon` | Target descriptions (added when needed) |
| `wasm-encoder` | WebAssembly output for `--target web`, at 0.255 |
| `rustls` | TLS for the `std.http` client, in `lucid_rt` on native targets only, at 0.23, with default features off and `ring`, `std`, and `tls12` on (decision 179) |
| `webpki-roots` | The roots that TLS trusts, from Mozilla, at 1.0 (decision 179) |
| `libsqlite3-sys` | SQLite 3.54.1 for `std.sql`, in `lucid_rt` on native targets only, at 0.39, with `bundled`. Its bindings leave out `sqlite3_close_v2`, which `sql.rs` declares (decisions 194 and 209) |
| `argon2` | Argon2id for `std.crypto` passwords, in `lucid_rt` on native targets only, at 0.6, with only `alloc` and `password-hash`, and optimized in dev builds so tests stay fast (decisions 193 and 208) |
| `ring` | Digests, HMAC, and random bytes for `std.crypto`, named directly at 0.17 with default features off. `rustls` already builds it (decision 193) |
| `subtle` | Constant-time compare for `crypto.same`, at 2.6. `rustls` already builds it (decision 208) |

Cranelift crates are at 0.137, the latest stable release at M1. No parser generators and no CLI framework.

`ring`, which `rustls` uses, compiles C and assembly with the system C compiler when `lucid_rt` builds.

## Testing

- **Golden run tests:** `tests/run/name.lcd` with `name.out` for expected output.
- **Golden error tests:** `tests/fail/name.lcd` with `name.err` for expected diagnostics, under `lucid run`.
- **Golden cost tests:** `tests/costs/name.lcd` with `name.out` for what `lucid check --costs` reports.
- **Golden trap tests:** `tests/trap/name.lcd` with `name.err` for the trap message, and exit code 101.
- **Modules:** a file that a golden test imports sits in `modules_lib/` under the test directory, which the runner does not list.
- **Golden package tests:** `tests/package/name/` must print `expected.out` and exit 0, or `expected.err` and exit 1, under `lucid run name`. A directory with neither is a dependency of the others. E0338, E0339, and E0340 are tested here, since they need a manifest. `permissions_hold_at_run_time` in `golden.rs` checks the run-time lists under `lucid run` and in a built program.
- **Golden format tests:** `tests/fmt/name.lcd` with `name.out` for what `lucid fmt` makes of it. Each `.out` file must stay unchanged under `lucid fmt --check`, and so must every `.lcd` file in `std/`, `tools/`, and `examples/`.
- **Golden `lucid test` tests:** `tests/test/name.lcd` with `name.out` for the report. Exit code 1 if it holds a failure.
- **Error code check:** `compiler/tests/golden.rs` fails if a code in the source has no golden test or no entry in `docs/errors.md`, or if an example there does not give its own code.
- **Examples:** everything in `examples/` must parse. Later milestones raise this to compile.
- **Agent guide:** every `lucid` block in `docs/agent-guide.md` must run with exit code 0, and the guide must stay under the token budget, at about 4 bytes per token.
- **Lucid compiler:** the run, error, trap, and package tests run a third time with the compiler in Lucid, which `ported_exe` builds once, and `built_programs_match_run` builds with it too.
- **Ownership-off mode:** the run and trap tests run a second time with `LUCID_NAIVE` set, which turns ownership optimizations off. Output must match.
- **Memory checks:** the debug runtime counts live values, so every golden test also fails on a leak or double free.
- **Web programs:** `web_programs_match_run` runs the run and trap tests built for the web, under `node`. See "WebAssembly".
- **Web runtime:** `web_runtime_loads` loads the WebAssembly runtime under `node`, with the imports above, and checks its imports, exports, header offsets, and each import it uses.
- **Built programs:** `todo_example_works` and `built_programs_match_run` build with `lucid build`, and compare the program with `lucid run`. Every run and trap test gave the same output built, when checked by hand on 2026-11-07. `website_serves_under_load` builds `examples/website.lcd`, starts it on a free port, holds one connection with half a request head, and sends 20 requests from each of 64 threads at once, each thread on one kept-alive connection. Every answer must be 200 with the right body, the half request must then get its answer, and the server must not have stopped or written to stderr. It took about 1 second on 2026-11-08.
- **Stage dumps:** `stage_dumps_are_stable`, see "Self-hosting". It takes over a minute, so the `Stop` hook leaves it out, see `claude-code.md`.
- **Fuzzing:** `front_end_never_panics` in `fuzz.rs` mutates the `.lcd` files of `tests/`, `examples/`, `std/`, and `tools/`, and runs each result through `load`, `check`, `mono`, and `ownership`. None may panic. `cargo test` runs 300 inputs from a fixed seed. `LUCID_FUZZ=20000` runs more, and `LUCID_FUZZ_SEED` changes the seed. An input that panics is saved as `target/fuzz-crash-<seed>-<n>.lcd`. On 2026-11-07, 20,000 inputs gave no panic, and about 14% of them reached the checker. Code generation is not fuzzed (decision 150).

## Release

A release ships the Rust `lucid`, built in release mode, for Windows and for Linux x86-64, by decision 229. The binary holds `std`, the runtime, the WebAssembly runtime, and `docs/errors.md`, so it needs no other file. `lucid build` needs the MSVC linker on Windows and `cc` on Linux, as under "`lucid build`". The Linux binary needs a glibc no older than the one it was built against. The Lucid compiler in `compiler/lucid/` does not ship, since it reads its backend library from `target/`.

The steps, from a clean checkout of `main`, with `VERSION` the new version:

1. Set `version = "VERSION"` in `compiler/Cargo.toml`, `backend/Cargo.toml`, and `runtime/Cargo.toml`, by decision 227, and commit with the docs that name the version.
2. On Windows, run `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test --release`. Each must pass. The release run is the one that compares stage 1 of the fixed point. Then, in `website/`, run `npm run check:examples`, `npm run check:links`, and `npm run build`.
3. In WSL, copy the tree into the Linux file system, and run the same `cargo test` and `cargo test --release` there:

   ```sh
   git ls-files -co --exclude-standard | tar -cf /tmp/lucid.tar -T -   # from the checkout, in Git Bash
   mkdir ~/lucid-VERSION && tar -xf /mnt/c/.../lucid.tar -C ~/lucid-VERSION   # in WSL
   ```

4. Check that `target/release/lucid --version` prints `lucid VERSION` on each system.
5. Record the SHA-256 of each binary, with `sha256sum target/release/lucid.exe` on Windows and `sha256sum target/release/lucid` in WSL, and the host each was built on: the Rust version, and the glibc version from `ldd --version` on Linux.
6. Make an annotated tag `vVERSION` on the release commit, whose message holds the hashes and hosts of step 5.
7. Pack the archives, by decision 233: `lucid-VERSION-windows-x86_64.zip` and `lucid-VERSION-linux-x86_64.tar.gz`, each holding the binary, `README.md`, `LICENSE-MIT`, and `LICENSE-APACHE`, flat. Pack the Linux one in WSL, so the binary keeps mode 755. Write `SHA256SUMS` with `sha256sum` over both, and check it with `sha256sum -c`.
8. Find the Linux binary's real glibc floor, the newest version in `objdump -T target/release/lucid | grep -o 'GLIBC_[0-9.]*' | sort -V | tail -1`. If it moved, change the check in `website/public/install.sh` and the pages that name it.
9. Add a line per archive at the top of `website/public/releases.txt`: `VERSION PLATFORM URL SHA256`, with the URL of the release asset.
10. Push `main` and the tag, which deploys the site, then make the release: `gh release create vVERSION --verify-tag --title "Lucid VERSION" --notes-file NOTES --latest`, with both archives and `SHA256SUMS`. The notes say how to install, and give the hash and host of each binary.
11. Check that both installers, run as the downloads page shows them, install `lucid VERSION`.

The owner decides when to release. Each step that publishes, 10 above, needs the owner's word at the time.

## Development machine

Checked on 2026-11-04, on Windows 11 x86-64:

- Rust 1.97.1, target `x86_64-pc-windows-msvc`. A test binary linked and ran.
- Visual Studio Build Tools and Windows SDK 10.0.26100 are installed.
- `rust-lld` ships with the Rust toolchain.
- The `wasm32-unknown-unknown` target, added on 2026-11-07, which `backend/build.rs` needs: `rustup target add wasm32-unknown-unknown`.
- Node 26.13, which `cargo test` needs for the web tests.
- clang, gcc, and LLVM are not installed. The compiler does not need them.
- **Trap:** `/usr/bin/link.exe` in Git Bash is the GNU `link` tool, not the MSVC linker. `lucid build` finds the MSVC linker through `find-msvc-tools`, so it never runs this one.

Checked on 2026-11-08, for Linux: Arch Linux in WSL 2 on the same machine, x86-64, with gcc as `cc`, Node, and Rust 1.99.1 from `rustup`, with the `wasm32-unknown-unknown` target. `cargo test` runs from a copy of the tree in the Linux file system, since builds under `/mnt/c` are slow.
