# Decisions

Add new entries at the end of the list. To reverse a decision, add a new entry that supersedes it. Evidence and its strength are in [research.md](research.md).

## Settled

| # | Date | Decision | Reason |
| --- | --- | --- | --- |
| 1 | 2026-11-04 | Name: Lucid, extension `.lcd`, CLI `lucid` | Owner's choice. A 1970s dataflow language was also called Lucid, but it is not in common use. |
| 2 | 2026-11-04 | Memory: inferred ownership with reference-counting fallback | No GC and no lifetime annotations. Best fit for ease plus speed. Hardest compiler work. |
| 3 | 2026-11-04 | Syntax: clean hybrid with indentation and English keywords | Owner's choice from three samples. Short, and unambiguous for agents. |
| 4 | 2026-11-04 | Compiler: Rust host, Cranelift native, WebAssembly output | Fast route to real programs. All front-end work is hand-written. |
| 5 | 2026-11-04 | `task` is the function keyword, and `print` is a statement | Taken from the chosen syntax sample. Concurrency uses `spawn`, so `task` carries no async meaning. |
| 6 | 2026-11-04 | Values are independent, with no stored references | Removes lifetimes from the language. See `memory-model.md`. |
| 7 | 2026-11-04 | Errors are union return values with `try` and `else` | No hidden control flow, and failure stays visible in signatures. |
| 8 | 2026-11-04 | One union form, `A or B`, with `T?` as shorthand | Brings in the TypeScript feel, and needs no `Option` or `Result`. |
| 9 | 2026-11-04 | Structural traits, no classes or inheritance | Fewer concepts, and code stays local. |
| 10 | 2026-11-04 | No `async`/`await` | Removes function coloring. The scheduler stays a runtime detail. |
| 11 | 2026-11-04 | MIR is structured | One input fits both Cranelift and WebAssembly. |
| 12 | 2026-11-04 | Integer overflow traps in release builds | Safety ranks above speed. Revisit if profiling shows a heavy cost. |
| 13 | 2026-11-04 | Return types are never inferred. No `->` means `none` | A signature must be complete by itself, for local reasoning. |
| 14 | 2026-11-04 | `try` and `else` prefer error types over `none` | Otherwise success in a `none or Error` task would count as failure. |
| 15 | 2026-11-04 | The `Error` trait requires `error_message`, not `message` | Traits are structural, so a common name would turn unrelated types into errors. |
| 16 | 2026-11-04 | Self-host the compiler in stages after 1.0, starting with `lucid fmt` at M5. Keep Cranelift in Rust behind a C ABI | Tests the language on a large program. Waiting avoids rewrites while the language changes, and the backend rewrite has no user value yet. |
| 17 | 2026-11-04 | Tasks cannot use a top-level `var` | Swift needs run-time checks for globals. Banning this access keeps exclusivity static. |
| 18 | 2026-11-04 | Permissions are transitive, and `process` counts as full trust like `unsafe` | Without this, permissions could be bypassed. Deno and Node.js document these escapes. |
| 19 | 2026-11-04 | Run an agent trial at M3, not M5 | Models have no Lucid training data, so agent-friendliness must be tested before syntax settles. |
| 20 | 2026-11-05 | `print` takes one value | One way to do it. Interpolation already joins values. Closes the M1 open question with its default. |
| 21 | 2026-11-05 | Names are ASCII for now | Identifier confusables are a known attack (see `research.md`). Widening later breaks nothing, and narrowing later would. |
| 22 | 2026-11-05 | Text escapes are `\n`, `\t`, `\r`, `\\`, and `\"` only. `"""` text drops a blank closing line | The spec left both undefined. These escapes are common to Python and TypeScript. Swift has the closing-line rule, so the value has no stray line break. |
| 23 | 2026-11-05 | `/` yields `float` for `int` operands, and follows IEEE 754 | Closes the M2 open question with its default. It matches Python, and `//` is one character away. The agent trial at M3 tests the surprise. |
| 24 | 2026-11-05 | `//` rounds down, `%` takes the sign of the divisor, both are `int` only, and a zero divisor traps | The spec named floor division, and this pairing keeps `(a // b) * b + a % b == a`, as in Python. `float` operands stay out until a real need appears, since adding them later breaks nothing. |
| 25 | 2026-11-05 | Stack overflow is a trap, checked on entry to each task | Recursion arrived at M2, and hitting the guard page ends the process with an operating system error, not the documented exit code 101. A software check gives the source position too. |
| 26 | 2026-11-05 | Named arguments keep parameter order, defaults come last, and only a parameter with a default can be named | Evaluation order is then the order on the page, with no hidden reordering. Swift has the same order rule. One form for each call. |
| 27 | 2026-11-05 | A task reads a top-level `let` only if its value is a literal | Top-level statements run in order, so a task called earlier could read an uninitialized value. That would break a guarantee in `security.md`. |
| 28 | 2026-11-05 | A trap message holds its source position, and each site's text is static data | One runtime entry point, and no cost on the hot path. Research favored a shared cold path. A stack trace comes later. |
| 29 | 2026-11-05 | Accept the M2 exit with `fib(40)` at 2.02x to 2.09x of Rust release. The 2x target in `vision.md` stays, and M4 judges it on the benchmark suite | Owner's choice. Evidence is in `research.md`: Rust itself pays 1.56x to 1.92x for the same checks, and Lucid is 1.20x to 1.26x of checked Rust on `fib`. This accepts one exit, not a lower target. `collatz` at 2.47x of checked Rust stays an M4 item. |
| 30 | 2026-11-05 | `-` joins a number with no postfix after it, in the parser. A default must be a literal (E0313) | The checker could not tell `-(x)` from `-x`, which let an out-of-range literal through. A literal default has no evaluation order to explain. Judgment calls with no outside evidence. |
| 31 | 2026-11-05 | Interpolation is the only way to join text. `+` does not work on `text` | Rules in `AGENTS.md`: a second way needs a strong reason. Adding `+` later breaks nothing. Judgment call, and the M3 agent trial tests whether agents reach for `+`. E0303 points to interpolation. |
| 32 | 2026-11-05 | `.len()` and `.is_empty()` are the names for text length and emptiness | The spec fixed `x.is_empty()` and left length unnamed. `len` is the short form that Python and Rust share. |
| 33 | 2026-11-05 | Naive counting uses task-level ownership: an expression borrows, a local or hidden local owns, and a task releases all its locals on return | No block tracking is needed for `break`, `continue`, or `return`, and hidden locals work inside `if` values and `and`. Trade-off: a value can outlive its block until M4. Judgment call. Decision 2 and `memory-model.md` already chose naive counting first. |
| 34 | 2026-11-05 | The leak and double-free checks are the debug profile of `lucid_rt` | Every golden test gets them with no flag, and a release build pays nothing. |
| 35 | 2026-11-05 | A text has no single index and no character type. Iteration gives one-character `text`, and `x[i..i + 1]` reads one character | One way to do it. `int` indexing on text would look O(1) and be O(n). Adding a character type or `x[i]` later breaks nothing. Judgment call, no source read. The M3 agent trial tests whether agents reach for `x[0]`, and E0303 points to the slice. |
| 36 | 2026-11-05 | A character is one code point. Text slices use character positions, and a bad range traps | `docs/language.md` already counts characters for `.len()`. Grapheme rules need Unicode tables in the runtime, and can arrive as a library. A trap matches list indexing and makes a bug visible, where clamping hides it. Judgment call, no source read. |
| 37 | 2026-11-05 | `.bytes()` waits for `list`, since its result is `list[byte]` | A stand-in type would be a breaking change once `list` arrives. Scope moved inside M3. |
| 38 | 2026-11-05 | Lists, maps, and sets start with a small built-in set of tasks. Anything needing `T?`, lambdas, or derived traits moves to that roadmap item | A stand-in for `.get` would be a breaking change once `T?` arrives, the reason in decision 37. The prelude moves to Lucid once generics exist. Scope moved inside M3. Judgment call. |
| 39 | 2026-11-05 | `key in m` is the membership test, and `for` over a map gives keys. There is no `.has` | The grammar already has `in`, and rule 1 of `agents.md` allows one way. Python uses both forms, so agents have strong priors. This settles the open question on `T?` in a map: `.get` flattens, and `in` tells the cases apart. Judgment call. |
| 40 | 2026-11-05 | A map uses the compact dict layout, hashed with SipHash through `RandomState` | Insertion order is the spec, and the layout gives it with one allocation. Python moved to SipHash to stop hash flooding (PEP 456), and random keys cannot change the output because order ignores the hash. M4 may change the hash if the benchmarks ask for it. Evidence is in `research.md`. |
| 41 | 2026-11-05 | A list or map write is a statement that hands the local's count to the runtime, which copies only if the value is shared | A write in an expression could free an element that the same statement still borrows, which breaks a guarantee in `security.md`. |
| 42 | 2026-11-05 | Every collection slot is 8 bytes for now | One code path until monomorphization exists. `list[byte]` pays 8x memory. M4 measures it. |
| 43 | 2026-11-05 | `byte` arrives with no literals or arithmetic, until sized numbers | `.bytes()` needs a real type, per decision 37, and the rest of `byte` has a roadmap item. |
| 44 | 2026-11-05 | A key is `int`, `bool`, `byte`, or `text`. `float` never qualifies, and collections wait for `Hash`. `in` on a list takes the same types | NaN does not equal itself, so a `float` key could never be found again. Judgment call. |
| 45 | 2026-11-05 | A negative list index traps, a missing map key traps on read, and `.remove` of a missing key does nothing | `language.md` already traps out of bounds, and Python's negative index hides off-by-one bugs. After `.remove` the key is gone either way, so there is no bug to report. Judgment call, and the M3 agent trial tests whether agents reach for `x[-1]`. |
| 46 | 2026-11-05 | `x[i] = f()` runs the index first, then the value, and each index in a path runs once | Decision 26 keeps evaluation in the order on the page. Python runs the value first, so agents may expect that. Judgment call. |
| 47 | 2026-11-05 | An empty `[]` or `{}` with no type from its context is E0314. Only earlier siblings give a type, so `[[], [1]]` is an error | Inference from later uses would break local reasoning, and types flow one way through the checker. Judgment call, and the M3 agent trial tests the surprise. |
| 48 | 2026-11-05 | A write through a path takes the element out of its slot, which holds null until the write ends | Review measured `g[k].push(v)` as quadratic, since the slot and a hidden local both held a count. Grouping into a map of lists is too common to wait for M4. Every index and value is read first, so no safe code sees the null. |

## Open questions

Decide each one by the milestone shown.

| Question | Default if undecided | Decide by |
| --- | --- | --- |
| How are generic arguments that are not expressions written? `f[int](x)` parses as an index, but `f[int?](x)` is a syntax error. | Parse index contents as a type when an expression fails. | M3 |
| Should `change` receivers be marked at dot calls? | No marker, and require `var`. | M3 |
| Can a task read a top-level `let` that is not a literal? It needs a defined initialization order. | No: keep decision 27. | M5 |
| Add an LLVM backend for release builds? | Only if benchmarks demand it. | M4 |
| Literal types such as `"open" or "closed"` | Leave out of version 1. | M5 |
| License | Choose before first public release. | M5 |
| Browser UI framework shape | Decide from real programs. | M6 |
| Can traps be contained so one failing server request does not end the process? | Let a process supervisor restart it. | M7 |
| Lightweight tasks or OS threads? JSPI allows blocking calls in browsers. Foreign calls are the unstudied cost. | Start with OS threads. | M7 |
