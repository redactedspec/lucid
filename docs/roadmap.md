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

## M6: Browser (done)

Designed by decisions 156 to 164, see "WebAssembly" in `compiler.md`. Build in this order:

- [x] Prelude: text split, integer parsing, case change, list sort, by decisions 155 and 165
- [x] Runtime on `wasm32`, with the fixed-width layout of decision 156
- [x] WebAssembly backend from MIR: numbers, control flow, calls, `print`, the loader, `--target web`
- [x] WebAssembly backend: heap values, task values, `change`, `std`, traces. All run and trap tests pass under Node
- [x] `std.dom` and `dom.app`, by decisions 163 and 168
- [x] Example page, checked in a browser: `examples/web`

**Exit:** an interactive page written only in Lucid runs in a browser.

## M7: Concurrency and servers (done)

Designed by decisions 169 to 180, see "Threads" in `compiler.md`. OS threads, by decision 169. Build in this order:

- [x] Bit operators, by decisions 153 and 181
- [x] Counts safe across threads, by decision 170. The benchmarks of "M4 benchmarks" must not slow by more than 2%
- [x] Stack limit as a hidden parameter, by decision 175, with the same benchmark check
- [x] `spawn`, `spawned`, and waiting at the end of a task, by decisions 171, 172, and 182, and E0907, by decision 176
- [x] `channel`, `lock`, and the deadlock trap, by decisions 173, 174, and 184. A write through `update` to a value with one count is in place
- [x] `std.net`, by decisions 177 and 185
- [x] `std.html` and the `std.http` server, by decisions 178, 180, and 186, and `examples/website.lcd`
- [x] Load test: a Rust test that sends requests from 64 threads at once, and holds one slow connection open, to the built example. `website_serves_under_load` in `golden.rs`
- [x] `std.http` client with TLS, by decisions 179 and 187

**Exit:** `examples/website.lcd` serves pages under a load test.

## M8: Ecosystem (done)

Designed by decisions 188 to 205, see "Ecosystem" in `compiler.md`. Build in this order:

- [x] Prelude text tasks `starts_with`, `ends_with`, `contains`, `find`, and `trim`, by decision 189
- [x] Compile only reachable tasks, by decision 190
- [x] Linux x86-64: `run`, `test`, and `build`, with a fresh private staging directory, by decision 191. All tests pass in WSL
- [x] `std.crypto`, by decisions 193 and 208
- [x] `std.sql` and the `sql` type, by decisions 194, 195, and 209
- [x] Orphan rule, by decisions 196 and 210
- [x] Registry, `lucid package`, version selection, `lucid.lock`, and the fetch client in Lucid, by decisions 197 to 201 and 211. Tests use a local directory registry
- [x] `lucid lsp`, by decisions 202 and 212
- [x] Spec review, then the version 1.0 promise, by decisions 203 and 204. The `spec-reviewer` pass is decision 213, and a `/code-review ultra` pass found seven issues, of which six were fixed. The promise is in `vision.md`

**Exit:** a package that uses a registry package with `std.sql` and `std.crypto` builds and passes `lucid test` on Windows and Linux, and an editor shows its diagnostics through `lucid lsp`.

## M9: Self-hosted front end (done)

Started after the version 1.0 promise. Designed by decisions 214 to 220, see "Self-hosting" in `compiler.md`. Port one stage at a time, each checked against its Rust stage by dumps. The Rust front end and driver stay, and `lucid` stays the Rust program, by decision 214. Build in this order:

- [x] `LUCID_DUMP` in the Rust compiler, for `tokens`, `ast`, `mir`, and `owned`, and the golden test that compares dumps, by decision 215
- [x] Lexer and parser in Lucid, in `compiler/lucid/`. Token and tree dumps, and their diagnostics, match
- [x] Text append in place through a `change` parameter or a field, by decision 222
- [x] Resolver with `lucid.toml` reading in Lucid. `resolved` dumps and diagnostics match, by decision 223
- [x] Type checker in Lucid. `checked` dumps and diagnostics match, by decision 225
- [x] `derive` and the monomorphizer in Lucid. `mir` dumps and diagnostics match
- [x] Ownership pass, `--costs`, and `--unsafe` in Lucid. `owned` dumps match
- [x] Crate `lucid_backend`, the MIR reader, and the C ABI, by decisions 216, 217, and 226. The Lucid compiler runs `check`, `run`, and `build`, and the golden tests of those commands pass with it
- [x] Fixed point and bootstrap path, by decisions 218 and 219: `/Brepro`, the stage test on Windows and in WSL, and the steps in `compiler.md`

**Exit:** the Lucid-written compiler builds itself. A second build from that output is byte-identical.

## M10: Release readiness

Started after M9. Designed by decisions 227 to 230. No language change. Build in this order:

- [x] `/code-review ultra` over the M9 commits, run by the owner, then fixes. It covered the Rust side of M9, since the whole diff was over the size limit. Of nine findings, eight were fixed and one is answered by decision 231. The M5 commits held for review go to the 1.0 review
- [x] Version 1.0.0 in every crate, and `lucid --version`, by decision 227
- [x] Linux programs linked without debug sections, by decision 228
- [ ] The web loader checked in Firefox, by decision 230. Installing Firefox needs the owner's approval
- [ ] Release steps in `compiler.md`: build on Windows and in WSL, run every test, and record the SHA-256 of each binary, by decision 229
- [ ] License review, by the owner, before any public release
- [ ] Push to `origin`, which deploys the site, by the owner

**Exit:** `lucid --version` prints `lucid 1.0.0` on Windows and Linux, every test passes on both, and each finding of the ultra review is fixed or answered in `decisions.md`.

## Later, not planned

macOS, by decision 205. Text from a `list[byte]` in `std`, so the Lucid compiler takes the `std` files held inside the backend in place of `LUCID_STD`, by decision 226. The Lucid compiler for every command, then the `stage0.txt` pin, then deleting the Rust front end, by decisions 214, 219, and 229. Safari, by decision 230. `shared` and `weak`, by decision 213. Native backend in Lucid, LLVM release backend, plain types stored inline, literal types, compile-time evaluation.
