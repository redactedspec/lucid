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
| 22 | 2026-11-05 | Text escapes are `\n`, `\t`, `\r`, `\`, and `\"` only. `"""` text drops a blank closing line | The spec left both undefined. These escapes are common to Python and TypeScript. Swift has the closing-line rule, so the value has no stray line break. |

## Open questions

Decide each one by the milestone shown.

| Question | Default if undecided | Decide by |
| --- | --- | --- |
| Does `/` returning `float` for integers surprise agents? | Keep, matching Python. | M2 |
| Is `T?` inside a map a problem? `map[text, int?].get` flattens to `int?`, which loses "key missing" versus "value none". | Accept, and offer `.has(key)`. | M3 |
| Should `change` receivers be marked at dot calls? | No marker, and require `var`. | M3 |
| Add an LLVM backend for release builds? | Only if benchmarks demand it. | M4 |
| Literal types such as `"open" or "closed"` | Leave out of version 1. | M5 |
| License | Choose before first public release. | M5 |
| Browser UI framework shape | Decide from real programs. | M6 |
| Can traps be contained so one failing server request does not end the process? | Let a process supervisor restart it. | M7 |
| Lightweight tasks or OS threads? JSPI allows blocking calls in browsers. Foreign calls are the unstudied cost. | Start with OS threads. | M7 |
