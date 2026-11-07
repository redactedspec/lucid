# Roadmap

Each milestone ends with a runnable result. Do not start one until the last one passes its exit criteria.

## M0: Design (done)

Documents and example programs.

## M1: Hello world (done)

Build a thin end-to-end path first. Depth comes later.

- [x] Cargo workspace with `compiler/` and `runtime/`
- [x] Diagnostics with codes and `--json`
- [x] Lexer with layout tokens, for the full grammar
- [x] Parser for the full grammar, with error recovery
- [x] Cranelift JIT backend
- [x] `print` of text, `lucid run`, `lucid check`
- [x] Golden test runner
- [x] Claude Code setup, see "Project setup" in `docs/claude-code.md`

**Exit:** `lucid run` prints `Hello, world!`. All files in `examples/` parse, even though they do not yet run.

## M2: Core language (done)

- [x] `int`, `float`, `bool`, arithmetic with overflow traps
- [x] `let`, `var`, `if`, `while`, `for` over ranges
- [x] Tasks, recursion, parameter annotations
- [x] Type checking for all of the above
- [x] First benchmarks, with and without overflow checks

**Exit:** FizzBuzz and recursive Fibonacci run. `fib(40)` is within 2x of Rust release.

**Result:** both programs run. `fib(40)` measured 2.02x to 2.09x, and the owner accepted this exit in decision 29.

## M3: Data (done)

- [x] `text` values, interpolation, comparison, `.len()`, `.is_empty()`
- [x] `text` slicing, iteration
- [x] `list`, `map`, `set`, and `text.bytes()`
- [x] `type`, `choice`, `match` with exhaustiveness
- [x] Unions, `T?`, narrowing, `is`, and `.get`, `.pop`
- [x] Generics with monomorphization, list slices
- [x] Derived `Show`, `Equal`, and `Order`, showing and comparing collections
- [x] Traits, bounds, hand-written overrides, `self` and dot calls
- [x] `Hash` on compound keys
- [x] `any` values
- [x] Sized numbers, wrapping math tasks
- [x] Nested tasks, lambdas, task values
- [x] Parameter modes, exclusivity checks, `for change` over lists
- [x] Naive reference counting, leak-checking debug runtime
- [x] Draft `docs/agent-guide.md`, and run a 10-task agent trial

**Exit:** `examples/hello.lcd` runs. Golden tests cover every feature above, with no leaks under the debug runtime. Trial failures are recorded in `docs/research.md`.

**Result:** `examples/hello.lcd` runs, and the golden tests pass with no leaks. The trial passed 10 of 10 tasks on the first attempt, so it had no failures to record. Its limits are in `docs/research.md`.

## M4: Ownership performance (done)

- [x] Last-use moves, and block-level release of locals
- [x] Static text literals, with no allocation
- [x] Borrow elision of count operations
- [x] Reuse analysis, if measurements justify it: they did not, see decision 100
- [x] Shifts for division and remainder by a constant power of two
- [x] `lucid check --costs`
- [x] Benchmark suite: n-body, binary trees, JSON parse, hash map, string building

**Exit:** benchmark suite meets the speed and memory targets in `vision.md`. If not, decide in `decisions.md`: change the model or add an LLVM backend.

**Result:** all five programs meet the memory target, and four meet the speed target. `nbody` runs at 3.38x. Decision 101 keeps the model, adds no LLVM backend for now, and keeps the target. Figures are in `docs/research.md`.

## M5: A usable language (done)

- [x] `try`, `else`, the `Error` trait, unused-error check
- [x] `test`, `expect`, `unsafe`, and stack traces on a trap
- [x] Modules, `use`, `public`
- [x] Packages, `lucid.toml`, with path dependencies. The registry is at M8
- [x] `std.fs`, `std.json`, `std.env`, `std.args`, `std.time`
- [x] `lucid build`, `lucid test`
- [x] `lucid explain`
- [x] `lucid context`
- [x] `lucid fix`
- [x] `lucid fmt`, written in Lucid as first dogfooding test: `tools/fmt.lcd`
- [x] Permission checks, specified in `security.md` by decision 146
- [x] Full agent benchmark in `benchmarks/agent-bench/`, results in `research.md`
- [x] Fuzzing, in `compiler/src/fuzz.rs`

**Exit:** `examples/todo.lcd` builds to a standalone `.exe` and works.

## M6: Browser

Designed by decisions 156 to 164, see "WebAssembly" in `compiler.md`. Build in this order:

- [x] Prelude: text split, integer parsing, case change, list sort, by decisions 155 and 165
- [x] Runtime on `wasm32`, with the fixed-width layout of decision 156
- [x] WebAssembly backend from MIR: numbers, control flow, calls, `print`, the loader, `--target web`
- [x] WebAssembly backend: heap values, task values, `change`, `std`, traces. All run and trap tests pass under Node
- [x] `std.dom` and `dom.app`, by decisions 163 and 168
- [ ] Example page, checked in a browser

**Exit:** an interactive page written only in Lucid runs in a browser.

## M7: Concurrency and servers

- [ ] `spawn`, structured waiting, `channel`, `lock`
- [ ] Decide scheduler: OS threads or lightweight tasks
- [ ] `std.net`, `std.http`, TLS, `std.html`
- [ ] Bit operators, by decision 153

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

Native backend in Lucid, LLVM release backend, plain types stored inline, literal types, compile-time evaluation.
