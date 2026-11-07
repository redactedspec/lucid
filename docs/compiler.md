# Compiler Architecture

The compiler is written in Rust until M9, when its front end moves to Lucid (decision 16). The lexer, parser, type checker, and ownership analysis are hand-written. Machine code comes from Cranelift, and browser code comes from a WebAssembly backend.

## Status

M4 is done, and M5 is in progress. `lucid run` runs a file or a package, and `lucid build` makes it a standalone `.exe`. Both compile `int`, `float`, the sized numbers and wrapping math, `bool`, `none`, `text`, `list`, `map`, `set`, `type`, `choice`, `match`, unions, `is` with narrowing, interpolation, text slices, `let`, `var`, `if`, `while`, `for` over ranges, text, and collections, tasks with recursion and defaults, `change` and `take` parameters, `for change` over lists, lambdas, nested tasks, and task values, generic tasks, types, and aliases, traits with bounds, `any` values, `self` with dot calls, `try` and `else`, and `expect`, `test`, `unsafe`, `print`, modules with `use ./` and `public`, packages with path dependencies, and the M5 modules of `std`. It reports E0901 for anything else. `check.rs` binds names, checks types, and lowers to MIR in one pass, since `resolve.rs` leaves it one tree. A generic body is checked once, with each parameter as an opaque `Ty::Param`. Flattening means that only the real type arguments show what is a union and what is counted, so `check.rs` records intent as `Wrap`, `View`, `Member`, and `Holds`, and `mono.rs` picks the code for each, starting from every task with no type parameters. `Ty` keeps compound types behind leaked references, so it stays `Copy`. `Ty::Data` holds a name, an index into the checker's table, and type arguments, so a recursive type needs no cycle in `Ty`. A `match` lowers to an `if` chain on a hidden local, and the last arm has no test, since the exhaustiveness check proves what remains. `Ty::Union` holds flat members in a fixed order, so two spellings are equal. Narrowing is a stack of facts in the checker, cut back at the end of each block. An assignment resets entries in place, and a loop first resets each name its body assigns. List, map, and set tasks stay built into `check.rs`. They can move to Lucid now that `change` parameters exist. `mono.rs` also replaces `==`, `<`, showing, and `in` on a compound value with a call: to a hand-written task if the type has one, or else to a task from `derive.rs`. It also gives each new map or set with a compound key its hash and equal tasks. `mono.rs` lists the types that reach each `any`. A task on an `any` tests its tag against that list, so `run` builds these tasks last, and again if building them boxed a new type. A task of a trait has a signature in `check.rs` and no body, and a call to one is `ExprKind::Method` until `mono.rs` knows the receiver. Trait checks are structural, so `Checker::satisfied` runs at each use, and walks a type's fields for a derived trait. `ownership.rs` gives each new heap value an owner, makes every count explicit with `Move` and `Retain`, and finds moves with a backward walk of what is live. First it turns each local that can only hold a path into a read parameter into `Stmt::Bind`, which holds no count, so `Task::borrowed` lists what no exit releases. The walk repeats until each loop head settles, which takes a pass per level of nesting. "Counting" in `mir.rs` states the rules that a backend follows. `check.rs` lets go of the names in a block with `Stmt::Clear`, and `ownership.rs` does the same for temporaries after their statement.

## `unsafe`

An `extern` task has a signature and a place in `Program::tasks`, like any task, so calls need no new path. `check.rs` writes its body: one `return` of `ExprKind::Foreign`, which `native.rs` lowers to a call in the C ABI. Parameters stay 64-bit, since C reads the low part, and a result widens from its declared width. `find_symbol` looks in `ucrtbase` and `kernel32`, and `check.rs` calls it, so a missing name is E0904 under `lucid check` too, and not a panic in Cranelift. `.pointer()` needs a name so that the list outlives the statement: `Stmt::Clear` releases it at the end of its block, not at its last use.

`lucid build` links `kernel32.lib`, and `msvcrt.lib`, which brings `ucrt.lib`, so what `find_symbol` finds also links. Linux and macOS need their own answer at M8.

## `std`

`resolve.rs` holds each file of `std/` through `include_str!`, and loads one as a module with the key `@std` and its name, from the path `@std/name.lcd`, so everything after that is the path of any module. No written name starts with `@`, so nothing clashes. Messages, traces, and `--costs` show the name as `std.name`. A `std` module not in the list leaves its `use` in the tree, which `check.rs` reports as E0901.

An `extern` task in `std` may start with `lucid_` and may pass counted values. It gets a borrowed value for each parameter, and gives a new value, so `ownership.rs` counts `ExprKind::Foreign` as new. `native.rs` lists these symbols with the rest of the runtime. So the system calls in `std` are Rust's, in `lucid_rt`, and M8 gets Linux and macOS with no new `std` code. `--costs` and `--unsafe` leave out sites in `@std/`.

A file task in `lucid_rt` keeps its failure in a thread local, and gives an empty value, so a call needs no result type of its own. `std.fs` asks for that failure next, and builds a `FileError`.

## `std.json`

`std/json.lcd` holds all that does not depend on a type: the parser, which builds a private `Value` tree, reads of plain values, and generic reads and writes of lists, sets, and maps. `Json` is a built-in trait with no tasks, and `satisfied` in `check.rs` walks a type for it. `decode[T]` and `encode[T]` in that file have the body `...`, and they never compile: `mono.rs` turns a call to one into a call for its type, through `json_reader` and `json_writer` in `derive.rs`. Those pick a task of `std/json.lcd` by name, or `Derived::JsonRead` and `Derived::JsonWrite` for a `type`, `choice`, or union, which need its fields or members. A `Shape` keeps each field's default for this, and whether it is a `choice`.

A union picks its member from the tag of `Value`, through `json_tags` in `mir.rs`. So a read never goes back, and its time stays linear in the input. An error adds its step as it returns, so a read that works builds no path. Measured on a release build: 3 MB in 0.05 s to write, and 0.27 s to read into a `list` of a `type`.


## `test`

`check.rs` lowers a `test` to a task of its own, placed after the written tasks, only if the build keeps tests. `Frame::in_test` sends a failed `try` to `Stmt::Exit`, as at top level. `Program::tests` lists them.

A trap ends the process, and Cranelift frames have no unwind tables. So `lucid test` checks the file, then starts itself with `LUCID_TEST_FROM` set to the place of the first test to run. That child prints a NUL line before each test and after the last, and runs them in one process. The parent splits stdout at each NUL. With no final NUL, or an exit code that is not 0, the last test that started failed, and the parent starts a new child after it. A compile error that only code generation finds, E0902, comes from the child, in the form `--json` asks for. Passing tests cost one compile in total, and each failure costs one more.

## `expect`

`expect a == b` lowers to two hidden locals, an `if` on them, and `Stmt::Fail`, whose text is built only on the failing path. Any other condition uses `Stmt::Trap`.

## `lucid build`

`native::object` lowers the program through the same `compile` as `lucid run`, into a Cranelift object file instead of memory. The top level is `lucid_main`. `compiler/build.rs` builds `lucid_rt` a second time, as a release static library with the `exe` feature, which adds `exe.rs` and its C `main`. It runs its own `cargo` with a target directory of its own, so it waits on no lock of the outer build. The compiler holds that library through `include_bytes!`, writes it and the object file to a temporary directory, and runs the MSVC `link.exe`, which `find-msvc-tools` finds along with its library paths. The system libraries that the static library needs come from rustc's `--print native-static-libs`.

`main` in `exe.rs` sets `lucid_stack_limit`, reads the call site table from the object file, runs `lucid_main`, and ends through `std::process::exit`, which flushes stdout. Each task loads the limit on entry, where the JIT has a constant. The table is `lucid_call_sites`: a count, then rows of `SITE_WORDS` words, with names in `lucid_call_names`. The task address in a row is a relocation, so the linker fills it in. The linker gives the main thread `EXE_STACK_BYTES`, 256 MiB, which is reserved, not committed, and the top level may use half of it, as under `lucid run`.

The object file targets baseline x86-64, not the features of the machine that builds it, so the program runs on any x86-64 Windows. It is named after the file, or the package directory, and goes in the current directory. A failure to find or run the linker is not an error in the program, so it has no code: `lucid` prints the reason and exits with 1. A built program has no leak check, since its runtime is a release build.

## Early exits

`main.rs` puts a small Lucid prelude before each file, which today only declares `trait Error`. So nothing in the checker treats that trait as special, and an error type is one that satisfies it.

`try x` lowers to a `Seq`: a hidden local takes the value, an `if` tests `Holds`, and its branch is `Stmt::Return`, or `Stmt::Exit` at top level. `x else y` uses the same hidden local with `ExprKind::If`. `Frame::held` lists these locals, and `Checker::stmt` clears them after the statement, as does a `break` or `continue` inside it, since a count left there would make the next write copy. So a `Return` can sit inside an expression, and a statement can stop half done, which needs care in three places:

- **Values on their way to a keeper.** A record, list, map, or `take` argument holds a count outside any local until it is done. `Settle` in `ownership.rs` runs first. If a part can return, it runs that part and the ones before it into locals, in order. The exit of the task releases those, or a `Clear` after the statement.
- **Slots that are taken out.** `Stmt::Update` with `Take` must meet its `Put`. A `change` path already reads every other value before `take`, and `PutBack` in `check.rs` walks expressions, so a `for change` body puts its element back on this `return` too.
- **Blocks.** `ends` looks only at statements, so a `try` never counts as the end of a path, and a backend always has a block to go on with.

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

## Modules

`resolve.rs` loads the entry file and each `use ./` in turn, then rewrites each module so that the later stages see one file. `module.name` is the new name of each top-level name, and no written name has a dot, so nothing clashes. The module's name is its path from the entry file. In a program with imports the entry file gets a prefix too, its file stem, so a module cannot see its names, and messages show that form: `main.User`. A file alone keeps its names. `alias.name` becomes one name. A task with `self` keeps its name, and `check.rs` tests its visibility, since only a call's type finds it. `ast::short_name` gives the written name back where one is shown at run time or passed to C.

A file with a `use` that has no `./` finds its package by walking up to the nearest `lucid.toml`, so a dependency's files need no record of where they came from. `Loader::package` reads each manifest once, by real directory, so two spellings of a path are one package, and a diamond of dependencies loads one copy. A module of a package has the key `@name` and then its path, `lib` for `src/lib.lcd`, and no written name starts with `@`, so it cannot meet a local module of the same name. Messages, traces, and `--costs` show that key as its user writes it, through `diag::plain`. `manifest.rs` reads a small part of TOML by hand, see decision 121.

Every entry of every manifest read is checked, used or not, after the modules load. A dependency's path, `lucid.toml`, and `src` must match their spelling on disk exactly, by the rule in `exact_file`. `lucid test` on a package checks all files before it runs any test. `is_library` compares real paths, so `src/lib.lcd` follows the library rules when it is named directly too.

Permissions are checked in three places, by decision 146. `Loader::import_std` checks a `std` module's permission, and `Loader::check_allow` checks each dependency entry against both manifests. `resolve.rs` records the files of each package without `unsafe` in `Source::no_unsafe`, and `check.rs` reports E0340 at each `unsafe` block and `extern` in them. The root package's permissions go in `Source::grants`. `native.rs` passes its `files` and `env` lists, as lines of text from `manifest::runtime_grants`, to `lucid_rt::set_grants` under the JIT, and as the data `lucid_grants` in a built program, which `exe.rs` reads. The runtime checks each `std.fs` path and `std.env` name against them.

`Source` holds all the files end to end, entry file first, or the manifest and then the entry file for a package, so a span alone gives its file. `Lines::place` turns an offset into a file, line, and column.

## `lucid fmt`

Built, by decisions 144 and 145.

**Shape.** The formatter is a Lucid program, `tools/fmt.lcd`, held inside the compiler through `include_str!` at the path `@tools/fmt.lcd`, as `std` modules are. `lucid fmt [--check] [PATH]` finds the files as `lucid test` does: a file, or every file in a package's `src`. It lexes each file with `lexer.rs` first. A file with a lexer error gets its diagnostics, as under `lucid check`, and is left alone. The rest go to the formatter in a child process of `lucid`, marked by the `LUCID_FMT_CHILD` environment variable, which compiles it with the JIT and passes `--check` if given, then the paths, through `lucid_rt::set_args`. A trap or a failed `try` ends the child, not the parent, so the safety net below still runs.

The formatter reads each file as bytes and builds the output as a `list[byte]`. All syntax is ASCII, so a byte scan finds every token boundary. Text and comments are copied as bytes. It writes a file only if the output differs. With `--check` it writes nothing, prints each path that would change, and ends with a top-level `try` of an error, which exits with code 1.

**Safety net.** Before the run, `lucid` keeps the text of each file. After it, whatever the child's exit code, `lucid` lexes each changed file again and compares the token kinds, spans aside, with the old ones. A difference is a formatter bug: `lucid` writes the old text back, names the file, and exits with 101. Comments are not tokens, so the net does not cover them. The formatter copies them unchanged.

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

**Tests.** `tests/fmt/name.lcd` with `name.out`, the expected output. Every `.lcd` file in `std/`, `tools/`, and `examples/` must already be formatted, which `lucid fmt --check` tests. A second run over each `.out` file must change nothing. Line endings and a byte order mark are tested from generated files, since git may change them. The safety net was tried by hand with a formatter that drops all spaces, and has no automatic test.

## Repository layout

Start with few files. Split a file only when it hurts.

```text
compiler/          Rust crate `lucid`: the CLI and compiler
    src/
        main.rs    CLI, and linking for `lucid build`
        context.rs public API for `lucid context`
        diag.rs    diagnostics
        fuzz.rs    test only: mutation fuzzing of the front end
        lexer.rs
        manifest.rs    reads `lucid.toml`
        ast.rs
        parser.rs
        resolve.rs joins modules into one tree
        check.rs   names, types, lowering to MIR
        mono.rs    one concrete task per use of a generic
        derive.rs  derived tasks, as MIR for one concrete type
        mir.rs     typed, structured operations
        ownership.rs   owners and counts of heap values, last-use moves
        native.rs  Cranelift backend
        wasm.rs        planned: WebAssembly backend
    build.rs       builds `lucid_rt` as a static library for `lucid build`
    tests/golden.rs    runs everything in `tests/` and `examples/`
runtime/           Rust crate `lucid_rt`: allocator, counting, text, traps
    src/exe.rs     start of a built program, only in the static library
std/               standard library in Lucid, held inside the compiler
tools/             `fmt.lcd`, the formatter in Lucid, held inside the compiler
tests/             golden tests
benchmarks/        Lucid and Rust versions of each program
    agent-trial/   tasks, reference answers, and runner of the M3 trial
    agent-bench/   tasks, reference, runner, and scorer of the M5 agent benchmark
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

A trap site is a cold block that passes `lucid_trap` a message holding its source position, and its frame pointer. Every task keeps frame pointers (`preserve_frame_pointers`), so a trace walks saved pointers and return addresses. `native.rs` hands the runtime a table from each call's return address to its task, line, and column, built from Cranelift's call sites and source locations, and text is made only on a trap. A task with no name, which `derive.rs` wrote, has a row with no text, so the walk goes through it and prints nothing. An address that is not in the table is a Rust frame, whose saved pointer cannot be trusted, so the walk stops. Every task checks the stack pointer on entry, after its prologue, so its frame must fit in the 1 MiB margin. E0902 reads the compiled frame size, and rejects a task over half the margin. Under `lucid run` the limit is a constant in the code, because it has one thread with a known stack. A built program reads it from `lucid_stack_limit`, which its start sets. `spawn` needs a limit per thread.

- `lucid run` links it directly, since the JIT runs inside the compiler process.
- `lucid build` embeds a prebuilt `lucid_rt` static library in the compiler and passes it to the system linker.
- The web target compiles it to `wasm32-unknown-unknown`.

## Allowed dependencies

| Crate | Purpose |
| --- | --- |
| `cranelift-codegen`, `cranelift-frontend`, `cranelift-module` | Native code generation |
| `cranelift-jit` | `lucid run` |
| `cranelift-object` | Object files for `lucid build` |
| `find-msvc-tools` | Finds the MSVC linker and its library paths for `lucid build`, on Windows only (decision 139) |
| `target-lexicon` | Target descriptions (added when needed) |
| `wasm-encoder` | WebAssembly output (added when needed) |

Cranelift crates are at 0.137, the latest stable release at M1. No parser generators and no CLI framework.

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
| `lucid build [--json] [PATH]` | Produce a native executable | Works on Windows, for the features under "Status". Writes `NAME.exe` to the current directory |
| `lucid build --target web` | Produce WebAssembly and a loader | Planned |
| `lucid test [--json] [PATH]` | Run `test` blocks | Works, for a file, or every file in a package's `src` |
| `lucid fmt [--check] [PATH]` | Format in place, with no style options. `--check` lists files that would change, and exits with 1 if any would | Works, for a file, or every file in a package's `src`. See "`lucid fmt`" |
| `lucid fix [--json] [PATH]` | Apply suggested fixes | Works. A `Diagnostic` holds `fixes`, each a set of edits. The command writes the edited text of each file back with its own line endings, from the text that `Source` holds |
| `lucid explain [--json] CODE` | Explain an error code | Works, from `docs/errors.md`, which the compiler holds through `include_str!` |
| `lucid context [--json] MODULE` | Print a compact API | Works for a file, a package directory, which means every file in its `src`, or `std.NAME`. `context.rs` reads tokens, not the tree, so a file with errors still shows its API |
| `lucid doc` | Generate docs | Planned |

`run.py` also reports peak memory, on Windows only, with the peak of an empty program taken off the Lucid figure, since `lucid run` holds the compiler. The `LUCID_NAIVE` environment variable turns ownership optimizations off, for tests and for measuring them.

The `bench-unchecked` cargo feature removes `+`, `-`, and `*` overflow checks, only so that `benchmarks/run.sh` can measure their cost. It must never ship: `docs/security.md` promises traps in all builds.

`PATH` is a file or a directory. A directory means the nearest package at or above it, and no `PATH` means the current directory.

Exit codes: 0 for success, 1 for compile errors or a failed test, 2 for bad usage, and 101 for a trap or a compiler bug. If the reader of compiler output has gone, the output is dropped and the exit code does not change.

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
- **Ownership-off mode:** the run and trap tests run a second time with `LUCID_NAIVE` set, which turns ownership optimizations off. Output must match.
- **Memory checks:** the debug runtime counts live values, so every golden test also fails on a leak or double free.
- **Built programs:** `todo_example_works` and `built_programs_match_run` build with `lucid build`, and compare the program with `lucid run`. Every run and trap test gave the same output built, when checked by hand on 2026-11-07.
- **Fuzzing:** `front_end_never_panics` in `fuzz.rs` mutates the `.lcd` files of `tests/`, `examples/`, `std/`, and `tools/`, and runs each result through `load`, `check`, `mono`, and `ownership`. None may panic. `cargo test` runs 300 inputs from a fixed seed. `LUCID_FUZZ=20000` runs more, and `LUCID_FUZZ_SEED` changes the seed. An input that panics is saved as `target/fuzz-crash-<seed>-<n>.lcd`. On 2026-11-07, 20,000 inputs gave no panic, and about 14% of them reached the checker. Code generation is not fuzzed (decision 150).

## Development machine

Checked on 2026-11-04, on Windows 11 x86-64:

- Rust 1.97.1, target `x86_64-pc-windows-msvc`. A test binary linked and ran.
- Visual Studio Build Tools and Windows SDK 10.0.26100 are installed.
- `rust-lld` ships with the Rust toolchain.
- The `wasm32-unknown-unknown` target is missing. Add it at M6: `rustup target add wasm32-unknown-unknown`.
- clang, gcc, and LLVM are not installed. The compiler does not need them.
- **Trap:** `/usr/bin/link.exe` in Git Bash is the GNU `link` tool, not the MSVC linker. `lucid build` finds the MSVC linker through `find-msvc-tools`, so it never runs this one.
