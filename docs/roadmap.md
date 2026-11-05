# Roadmap

Each milestone ends with a runnable result. Do not start one until the last one passes its exit criteria.

## M0: Design (done)

Documents and example programs.

## M1: Hello world

Build a thin end-to-end path first. Depth comes later.

- [ ] Cargo workspace with `compiler/` and `runtime/`
- [ ] Diagnostics with codes and `--json`
- [ ] Lexer with layout tokens, for the full grammar
- [ ] Parser for the full grammar, with error recovery
- [ ] Cranelift JIT backend
- [ ] `print` of text, `lucid run`, `lucid check`
- [ ] Golden test runner

**Exit:** `lucid run` prints `Hello, world!`. All files in `examples/` parse, even though they do not yet run.

## M2: Core language

- [ ] `int`, `float`, `bool`, arithmetic with overflow traps
- [ ] `let`, `var`, `if`, `while`, `for` over ranges
- [ ] Tasks, recursion, parameter annotations
- [ ] Type checking for all of the above
- [ ] First benchmarks

**Exit:** FizzBuzz and recursive Fibonacci run. `fib(40)` is within 2x of Rust release.

## M3: Data

- [ ] `text`, interpolation, `list`, `map`, `set`
- [ ] `type`, `choice`, `match` with exhaustiveness
- [ ] Unions, `T?`, narrowing, `is`
- [ ] Generics with monomorphization, traits, derived traits
- [ ] Parameter modes and exclusivity checks
- [ ] Naive reference counting, leak-checking debug runtime

**Exit:** `examples/hello.lcd` runs. Golden tests cover every feature above, with no leaks under the debug runtime.

## M4: Ownership performance

- [ ] Last-use moves
- [ ] Borrow elision of count operations
- [ ] Reuse analysis, if measurements justify it
- [ ] `lucid check --costs`
- [ ] Benchmark suite: n-body, binary trees, JSON parse, hash map, string building

**Exit:** benchmark suite meets the speed and memory targets in `vision.md`. If not, decide in `decisions.md`: change the model or add an LLVM backend.

## M5: A usable language

- [ ] `try`, `else`, the `Error` trait, unused-error check
- [ ] Modules, `use`, `public`, packages, `lucid.toml`
- [ ] `std.fs`, `std.json`, `std.env`, `std.args`, `std.time`
- [ ] `lucid build`, `test`, `fix`, `explain`, `context`
- [ ] `lucid fmt`, written in Lucid as first dogfooding test
- [ ] Permission checks
- [ ] Agent guide and agent benchmark
- [ ] Fuzzing

**Exit:** `examples/todo.lcd` builds to a standalone `.exe` and works.

## M6: Browser

- [ ] WebAssembly backend from MIR
- [ ] Runtime on `wasm32`
- [ ] `std.dom`, JavaScript loader

**Exit:** an interactive page written only in Lucid runs in a browser.

## M7: Concurrency and servers

- [ ] `spawn`, structured waiting, `channel`, `lock`
- [ ] Decide scheduler: OS threads or lightweight tasks
- [ ] `std.net`, `std.http`, TLS, `std.html`

**Exit:** `examples/website.lcd` serves pages under a load test.

## M8: Ecosystem

- [ ] Package registry and `lucid.lock`
- [ ] LSP server for editors
- [ ] `std.sql`, `std.crypto`
- [ ] Linux and macOS support
- [ ] Spec review, then version 1.0 stability promise

## M9: Self-hosted front end

Start only after the version 1.0 stability promise. Port one stage at a time. The Rust version of each stage stays until its Lucid version passes all tests.

- [ ] Lexer and parser in Lucid
- [ ] Resolver, type checker, and monomorphizer in Lucid
- [ ] MIR and ownership pass in Lucid
- [ ] Cranelift called through a small C ABI wrapper in Rust
- [ ] Documented bootstrap path from a pinned release binary

**Exit:** the Lucid-written compiler builds itself. A second build from that output is byte-identical.

## Later, not planned

Native backend in Lucid, LLVM release backend, literal types, compile-time evaluation.
