# Research

Evidence behind Lucid's design. Reviewed on 2026-11-04. Add to this file whenever a decision is made or challenged.

**Evidence levels**

- **Read:** the primary source text was read.
- **Snippet:** seen only through search excerpts or a summary. Read the source before relying on the exact figure.
- **None:** no direct evidence found. The decision is a judgment call, to be tested.

## Summary

| Decision | Verdict | Level | Action |
| --- | --- | --- | --- |
| Reference counting with reuse | Supported. Lucid passes without reuse on allocation-heavy `trees` at 0.62x of Rust | Read, own measurement | Reuse not built, decision 100 |
| Mutable value semantics | Supported as research, and by Lucid's M4 suite on four of five programs. Tight float loops miss | Read in part, own measurement | Keep, decision 101 |
| Exclusivity checks | Supported, static checks alone are not enough | Snippet | Rule added: no top-level `var` in tasks |
| Cranelift for speed | Partly supported. 2x target is at risk | Read | Keep LLVM option open |
| Overflow traps in release | Measured: 1.17x to 1.78x in Lucid, 1.56x to 1.92x in Rust | Read | Keep. Revisit with M4 benchmarks |
| Permissions | Supported, but first design had holes | Snippet | `security.md` fixed |
| Agent-friendliness | Moderate. The M5 benchmark passed 44 of 50 on the first attempt, against 49 for Python and 46 for TypeScript, and 50 of 50 in the end, with limits stated below | Snippet, trial, benchmark | Fix the diagnostics and guide gaps it found |
| Indentation syntax helps agents | No evidence | None | Owner preference. Measure |
| No `async`, lightweight tasks in browser | Now feasible through JSPI | Snippet | Decide at M7 |
| ASCII names, text escapes | Judgment call from existing practice | None | Revisit before 1.0 |
| Division, argument order | Judgment call from existing practice | None | No failure in the M3 trial or the M5 benchmark |
| Type arguments in brackets, read in one pass | Follows Go. Form also proposed for Python | Read, parser Snippet | No failure in the M3 trial or the M5 benchmark |
| Generic body checked once, then substituted in MIR | Follows Rust and Go, not C++ templates | Snippet | No failure in the M3 trial or the M5 benchmark |
| Dot calls by receiver type, bounds required for every use | Judgment call from Rust and Python practice | None | One M5 benchmark failure, a missing `Equal` bound, repaired in one turn |
| Union narrowing by control flow | Follows TypeScript and Kotlin, tightened for memory safety | Read | No failure in the M3 trial or the M5 benchmark |
| Text slices, no character type | Judgment call from existing practice | None | No failure in the M3 trial or the M5 benchmark |
| Compact ordered map, SipHash with random keys | Supported by Python and Rust practice. `hashmap` is 1.74x of Rust with the same hash | Snippet, own measurement | Keep |
| A written `equal` removes derived `Hash` | Python and Clippy both guard this rule | Snippet | None |
| `any` by tag tests over the whole program, not a vtable | Differs from Rust and Swift, and no benchmark yet | Snippet | Measure at M4 |
| `change` as copy-in copy-out, with static exclusivity | Follows Swift `inout`, and stricter than Rust or Swift inside a statement | Read | No failure in the M3 trial or the M5 benchmark |

## Memory

### Perceus (Reinking, Xie, de Moura, Leijen, PLDI 2021)

Level: **Read** (technical report, by text extraction, partial).

- Compared Koka with OCaml, Haskell, Swift, and Java on allocation-heavy benchmarks, and with C++ on some.
- On tree insertion (`rbtree`, 42 million insertions), functional Koka is reported within 10% of C++ `std::map`.
- The benchmarks were chosen to stress memory. They are not general evidence of C++ parity.
- Reference counting cannot release cycles. In Koka, explicit mutable references are the only source of cycles.
- The authors supply no cycle collector, and expect mutable references to be a last resort.

**For Lucid:** this supports the `shared` plus `weak` design, because cycles can arise only through `shared`. It also warns that Perceus was built for a mostly functional language. Lucid looks imperative, so these results do not transfer automatically.

Source: [Perceus technical report](https://www.microsoft.com/en-us/research/uploads/prod/2020/11/perceus-tr-v4.pdf).

### Counting Immutable Beans (Ullrich, de Moura, IFL 2019)

Level: **Read** for the method, **Snippet** for benchmarks.

- Lean tags values as single-threaded, multi-threaded, or persistent. Only multi-threaded values use atomic count updates.
- Borrow inference is a heuristic, and correctness does not depend on it.
- It is unsafe to update a borrowed reference in place, even when its count is 1.
- Lean cannot create cycles, which is not true of Lucid.

**For Lucid:** this supports thread tagging in `memory-model.md`, and the rule that correctness never depends on an optimization. The in-place rule must hold in the ownership pass.

Source: [arXiv 1908.05647](https://arxiv.org/abs/1908.05647).

### Mutable value semantics (Racordon et al., JOT 2022)

Level: **Snippet**.

This paper describes implementation strategies for mutable value semantics, with Swift as a case study. Hylo builds a language on the idea and was still active in April 2026. Its stability was not confirmed.

**For Lucid:** no production language has shipped this exact model, so Lucid carries real research risk.

Read on 2026-11-06, in part: the paper's text, by extraction, for sections 2, 6, and 7, and Hylo tour chapters on bindings, functions, and subscripts.

- **Measured.** Racordon et al. compare Swift, their Swiftlet, C++, and Scala Native on random programs and micro-benchmarks. They report Swift "slower than C++ only for programs with extremely large number of mutating operations (>90%)". This is one machine and mostly generated programs.
- **The cost is uniqueness checks.** Copy-on-write comes "at the cost of runtime checks preceding every mutation". In Swiftlet, "unnecessary reference counter checks account for most of the performance loss". This points M4 at "Borrow elision of count operations".
- **Nested `inout` copies.** With a pointer to the inside of a shared buffer, "the caller is compelled to copy non-unique storage defensively". A Lucid path takes its value out of its slot, so a shared outer list copies for the same reason, once.
- **What is lost.** "Traversing an arbitrary graph requires access to the whole graph at each step". `shared` is Lucid's answer, which the paper's strict form does not have.
- **Hylo.** `let`, `inout`, and `sink` match Lucid's default, `change`, and `take`. Hylo also has `set`, which fills a variable that has no value yet, and subscripts that `yield` a projection. Lucid has neither, and no trial task needed them, so this calls for no change now.

Sources: [Paper in JOT](https://www.jot.fm/issues/issue_2022_02/article2.pdf), [Hylo tour](https://docs.hylo-lang.org/language-tour), [Google Research listing](https://research.google/pubs/mutable-value-semantics/), [preprint](https://daveabrahams.com/wp-content/uploads/2022/02/implementation_strategies_for_mutable_value_semantics-preprint.pdf), [Hylo docs](https://docs.hylo-lang.org).

### Exclusivity and reference count cost (Swift)

Level: **Snippet**.

- Static checks were not enough for Swift. Escaping closures, class properties, and globals needed run-time checks, which are now on in release builds.
- Run-time checks cost most on heavily mutable class properties.
- Swift reports of retain and release cost range from almost none to over 70% of a profile. They are anecdotes, not controlled studies.

**For Lucid:** Lucid removes two of Swift's three cases. Only `shared` needs dynamic checks. Top-level `var` was an unhandled third case, so tasks now cannot use it.

Sources: [SE-0176](https://github.com/apple/swift-evolution/blob/master/proposals/0176-enforce-exclusive-access-to-memory.md), [Swift forum report](https://forums.swift.org/t/improving-linked-list-performance-swift-release-and-swift-retain-overhead/17205).

## Compiler

### Cranelift

Level: **Read**.

- The Cranelift README cites third-party benchmarks: about 14% slower than an LLVM-based Wasm engine, and about 2% slower than V8 TurboFan.
- LWN describes the same 2020 paper as showing code "approximately twice as slow on some benchmarks".
- Compile speed was about an order of magnitude faster than LLVM.
- The data is from WebAssembly workloads in 2020, not from a language like Lucid.
- The README's planned Rust use is a debug backend, not a release one.

**For Lucid:** Cranelift is a sound choice for M1 to M4. The "within 2x of Rust" target has no margin on bad workloads. If M4 misses it, an LLVM release backend is the expected fix. Structured MIR keeps that open.

Sources: [Cranelift README](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/README.md), [LWN](https://lwn.net/Articles/964735), [Rust RFC 560](https://rust-lang.github.io/rfcs/0560-integer-overflow.html).

### Integer overflow traps

Level: **Snippet**.

- Regehr measured 11.8% slowdown on SPEC CINT 2006, reduced to 8.7% with patches. The worst case was near 40%. Most of the cost came from blocked loop optimizations.
- Luu measured 28% on bzip2 compression with diagnostics, and almost nothing without them.
- Rust disables these checks in release builds by default.

**For Lucid:** expect a real cost. Keep traps on, because safety ranks first. Lucid may pay less than LLVM languages, since Cranelift does fewer loop optimizations, but that is a guess to measure at M2. Trap sites should share one cold reporting path, to avoid the diagnostic cost Luu saw.

Sources: [Regehr](https://blog.regehr.org/archives/1384), [Luu](https://danluu.com/integer-overflow/). Regehr's figures and cause were read in full for decision 29. Luu's update says the register allocation problem is gone from clang 3.8 and gcc 5.

### Counts and moves (M4)

Level: **Read** for Perceus, in the entry above. Decision 92 takes one idea from it, explicit `dup` and `drop` instructions, and departs from it on when a value is released, in decision 94. That departure is a judgment call with no measurement behind it. The first M4 benchmark table is under "M4 benchmarks".

### M4 benchmarks

Level: **Read** (own measurement). `sh benchmarks/run.sh`, 2026-11-06, on the development machine: 15 paired rounds. Lucid includes JIT compile time, about 7 ms. Rust is `rustc -O`. Lucid memory has the peak of an empty program taken off, since `lucid run` holds the compiler, so it stands in for what `lucid build` will use. Targets from `vision.md`: 2x time, 1.2x memory.

| Program | Rust best ms | Lucid best ms | Time, best | Time, median | Memory |
| --- | --- | --- | --- | --- | --- |
| `nbody`, 2 million steps | 67 | 228 | 3.38 | 3.64 | 0.36 |
| `trees`, depth 18 | 2954 | 1828 | 0.62 | 0.66 | 0.92 |
| `json`, 5 parses of 2.4 MB | 153 | 253 | 1.65 | 1.49 | 1.15 |
| `hashmap`, 1 million keys | 297 | 517 | 1.74 | 1.65 | 0.90 |
| `strings`, 5 million appends | 120 | 92 | 0.76 | 0.79 | 0.98 |

Compared with one run of each before M4: `nbody` 3558 ms, `trees` 5291 ms, `hashmap` 688 ms, `strings` 45 s at 300,000 appends, and `json` over 280 s. The M2 programs moved little, as they hold no counted values: `fib` 2.01, `sum` 1.66, and `collatz` 2.82, down from 4.74, or 1.46 of Rust with overflow checks.

Limits: five programs that I wrote, on one machine, with spread between runs of about 10% to 20%. `trees` and `strings` beat Rust, and I did not find out why in detail, so do not read them as general wins. Both are programs that M4 worked on. `trees` has no leaf allocation in either language.

What moved the numbers, in order of size:

- **`strings`, `json`:** in-place append and the ASCII flag took these from quadratic to linear. Moves are what make the append safe.
- **`nbody`:** three steps of about 2x each. Inline counts and writes went from 3.3 s to 0.9 s, moving counts through slots went to 0.47 s, and computing a value once went to 0.23 s.
- **`trees`:** static variants with no fields halved allocation. Borrow elision and inline counts took off the rest.
- **Last-use moves alone changed little.** `trees`, `hashmap`, and `nbody` stayed within noise. Moves remove count pairs, and the time was in the runtime calls around them. They matter because they make values unique, which the writes above depend on.

**Cranelift places pure values at their uses.** A value used in both arms of a branch runs in each arm, and again after the join if used there, so a write with a fast arm and a copying arm ran `sqrt` and `/` six times per pair in `nbody`. Read from the machine code of the flat version. A branch that holds no use of the value avoids it. Any later hot path with two arms has the same risk.

**Where `nbody` stands.** Best of 9 runs, in ms:

| Build | Time |
| --- | --- |
| Rust | 67 |
| Rust, values boxed one by one | 90 |
| Rust, no vectorizing | 99 |
| Rust, boxed, no vectorizing | 99 |
| Lucid | 226 |

Vectorizing is worth about 1.5x here, and Cranelift has none. Boxing costs at most 1.34x, even under LLVM. A version of the Lucid program with one flat `list[float]` per field took 309 ms in one run, so moving the records out of the heap would not help by itself. The gap to unvectorized Rust, 2.2x, is then bounds checks, count checks, and code quality. No LLVM build was tried, so what it would give Lucid is unmeasured.

### First Lucid benchmarks (M2)

Level: **Read** (own measurement). `sh benchmarks/run.sh`, 2026-11-05, on the development machine: 15 paired rounds, so drift hits every build alike. Lucid includes JIT compile time. Rust is `rustc -O`, and "Rust checked" adds `-C overflow-checks=on`.

| Program | Rust | Rust checked | Lucid | Lucid unchecked | Lucid / Rust | Lucid / Rust checked |
| --- | --- | --- | --- | --- | --- | --- |
| `fib(40)`, best ms | 222 | 366 | 463 | 396 | 2.09 | 1.26 |
| `fib(40)`, median ms | 259 | 434 | 523 | 475 | 2.02 | 1.20 |
| `collatz`, best ms | 335 | 642 | 1588 | 1767 | 4.74 | 2.47 |
| `collatz`, median ms | 352 | 656 | 1742 | 1909 | 4.95 | 2.65 |
| `sum`, best ms | 622 | 970 | 1045 | 588 | 1.68 | 1.08 |
| `sum`, median ms | 737 | 1122 | 1229 | 652 | 1.67 | 1.10 |

Limits: three small programs, one machine, and spread between runs of about 10% to 20%. Treat small differences as noise. In `collatz`, Lucid without checks measured slower than with them, and this is not explained.

Findings:

- **LLVM pays too.** Overflow checks cost Rust 1.56x to 1.92x on these programs. Lucid pays 1.17x on `fib` and 1.78x on `sum`. Regehr states that "most of the slowdown comes from integer overflow checks interfering with loop optimizations", and `fib` shows that effect: optimized Rust has one recursive call site in `fib`, because LLVM turns the other into a loop. With checks it keeps both. Rust release therefore sets a target that a checked language cannot reach through this route.
- **`collatz` is a division gap.** Rust with `black_box` on the divisor, so that it must divide, took 2044 ms best and 2269 ms median, and 2266 ms and 2365 ms with checks. Lucid took 1567 ms and 1861 ms in the same rounds. So Rust's lead here comes from turning `% 2` and `// 2` into shifts. Lucid can do the same, see M4.
- **The target was tight before overflow checks.** The Cranelift README puts generated code "~14% slower than WAVM (LLVM)", from a 2020 WebAssembly paper. Regehr reports 11.8% for checks on SPEC CINT 2006, with HMMER at about 40%. Rust RFC 560 keeps checks out of optimized builds so that "final code will not pay a performance penalty", and gives no measurement.
- **Calling conventions did not help.** `SystemV`, `Tail`, and `Fast` stayed at 457 to 473 ms on `fib`. Each check is `seto`, `test`, and `jnz`, because Cranelift has no signed overflow branch.
- `lucid run` of hello world took 6.7 ms, against the 100 ms target.

### WebAssembly stack switching

Level: **Snippet**.

JSPI reached phase 4. Caniuse lists support in Chrome 137, Firefox 153, and Safari 27. Verify versions against release notes before use. The status of the separate stack-switching proposal was not established.

**For Lucid:** blocking calls in the browser can use JSPI, so "no `async`" is viable there.

Sources: [V8 blog](https://v8.dev/blog/jspi), [caniuse](https://caniuse.com/wf-wasm-jspi).

## Concurrency

Level: **Snippet**.

Structured concurrency is widely adopted, for example Java `StructuredTaskScope`. Rust removed green threads before 1.0, partly because of per-thread stack memory. Java virtual threads need no function coloring, but pinning can block a carrier thread. Foreign calls are the main unresolved cost, and this search did not cover them. Study this before M7.

Sources: [Why async Rust?](https://without.boats/blog/why-async-rust/).

## Security

Level: **Snippet**.

- Deno states that a loaded native library ignores its `--allow-*` flags, and that subprocesses escape its permissions.
- Node.js states that it does not provide a sandbox, and that child processes do not inherit restrictions.
- Capslock analyzes Go packages by following transitive calls into privileged functions, and it is designed for review, not containment.

**For Lucid:** the first design had two holes. `process` could also bypass all limits, and a package could reach the network indirectly through a permitted dependency. Both are fixed in `security.md`. Deno checks `--allow-read` paths for the whole process, not per module, and decision 146 does the same at run time. Permissions limit careless and casual misuse of dependencies. They are not a sandbox.

Sources: [Deno security](https://docs.deno.com/runtime/fundamentals/security), [Node.js security model](https://github.com/nodejs/node/blob/master/doc/contributing/security-model-strategy.md), [Capslock](https://security.googleblog.com/2023/09/capslock-what-is-your-code-really.html).

## Agents

This is the weakest area, and it is a stated core goal.

### What the evidence says

- **No ranking exists.** No controlled study was found that ranks languages for agents.
- **Compile failures are common in strict languages.** Nogueira et al. counted 86,726 errors from seven LLMs across C, C++, Java, and Rust. Compile failures were 68% of Rust errors, and 46% of Java errors (calculated from their counts). A compile failure can still be better than a silent bug, and agents get to repair it. Level: **Read** (partial).
- **Unfamiliarity costs.** Wu, Anderson, and Guha found OCaml had a significant token penalty against Python across all models tested, while Java and Rust varied by model. They report agents burning tokens on an unfamiliar compiler. Level: **Snippet**.
- **Explicit types may help learning.** One scaling-law study found Rust's annotations and rigid syntax make it more learnable with fewer parameters. Level: **Snippet**.
- **Grammar helps generation.** Grammar Prompting improved DSL generation with few examples. Level: **Snippet**.
- **Syntax is token-relevant, but not proven for layout.** Studies compare languages, and none isolates indentation from braces.

### M3 agent trial

Run on 2026-11-06. Tasks, reference answers, and the runner are in `benchmarks/agent-trial/`.

**Method.** Two new Claude Code subagents on Opus 5.5 got `docs/agent-guide.md` (about 4,400 tokens), `tasks.md`, and the debug compiler, in a directory outside the repository. Each did 5 of the 10 tasks, and was told to read no other Lucid source. `try.sh` kept every attempt and compared its output with answers from a Python reference. The debug runtime also fails a run on a leak.

| Measure | Result |
| --- | --- |
| Compiled on the first attempt | 10 of 10 |
| Passed on the first attempt | 10 of 10 |
| Repair turns | 0 |
| Traps, leaks | 0 |
| Failures to record | None. Every attempt was rerun afterwards, and all passed |
| Lines of Lucid written | 544 |

The tasks covered text, maps, unions, `choice`, `change`, `for change`, recursive `Node?` trees, generics, closures, traits, `any`, and sized numbers.

**What it does not show.**

- It does not show that the syntax matches what models expect. The guide warns of each known trap, such as `+` on text, `x[0]`, and capturing a `var`, so no decision that names this trial was tested against unaided habit. None of those errors fired.
- It does not test diagnostics, since no attempt failed.
- It is not a comparison. There is no Python or TypeScript arm, and 10 tasks is a small sample.
- It is not independent. One author wrote the language, guide, and tasks, the subjects are the same model family as that author, and they saw the project's `AGENTS.md`, which has no syntax. An agent on 5 tasks could also learn from its own earlier ones, though no attempt failed.
- The credit cannot be split between the guide and the language, which is why decision 90 adds an arm to the M5 benchmark.

**Guide gaps.** The subjects listed what they had to guess. Each guess worked, and these went into the guide after the trial: `%` sign, unary `-`, trailing commas, expressions in slices, `.get` with a negative index, `is empty` on text, and `_` for every field of a variant. They called task 03 and task 10 hardest, for `.pop` with narrowing, and for hex with no bit operations. The second supports decision 153, on bit operators.

### M5 agent benchmark pilot

Run on 2026-11-07, before the full benchmark, to find bad tasks and runner faults. Method as in decision 148, with one block, tasks 11 to 20, per arm: four Claude Code subagents on Opus 5.5. Logs are in `benchmarks/agent-bench/results/`, and `score.py` reads them.

| Arm | Compiled on the first attempt | Passed on the first attempt | Passed in the end | Repair turns |
| --- | --- | --- | --- | --- |
| Lucid | 7 of 10 | 7 of 10 | 10 of 10 | 5, on 3 tasks |
| Lucid, bare guide | 7 of 10 | 7 of 10 | 10 of 10 | 3, on 3 tasks |
| Python | 10 of 10 | 10 of 10 | 10 of 10 | 0 |
| TypeScript | 10 of 10 | 10 of 10 | 10 of 10 | 0 |

Every Lucid failure was a compile error, and every program that compiled gave the right output. No task spec or runner fault showed.

**Failures, by cause.**

- **`empty` as a name**, both arms, task 20. `empty` is a keyword, and E0201 says only `expected a name, found empty`.
- **`{}` as a field default**, both arms, task 17. E0313 says `a default must be a literal`, and the guide calls `{}` an empty literal.
- **Variants matched on a union**, Lucid arm, task 11, 3 repair turns. `match` on `int or ParseError` cannot name `Empty`: it needs an arm `e: ParseError`, then a second `match`. E0318 adds a note on nesting that misled. A `match` as an arm's value, and a block after a typed arm, also cost turns.
- **`==` on `text?` and `text`**, bare arm, task 19. E0303 points to `is`, which does not compare values.

**Guide gaps.** Both arms guessed that `\n` and `\"` work in text, which the guide never shows.

**Fixed after the pilot.** Decision 149 allows an empty `[]` or `{}` as a default. E0318 on a union now names the choice to match first, E0201 says when a name is a keyword, and E0303 on a union suggests `else`. The guide now shows escapes, and that a union match names no variant. The bare guide drops that last line.

**What it does not show.** The bare arm did no worse than the full one, but 10 tasks cannot tell the arms apart, and none of the guide's stripped warnings fired. Limits of the M3 trial still hold: one author, one model family.

### M5 agent benchmark

Run on 2026-11-07, by the method of decision 148: all 50 tasks in each of the four arms, one Claude Code subagent on Opus 5.5 per block of 10. Block 11 is the pilot's, run before the fixes listed above. The other blocks ran after them. Plan usage stopped the run partway, so the two Lucid arms of block 31 were run again whole in new workspaces, and their partial logs are not counted. Logs are in `benchmarks/agent-bench/results/`, and `score.py` reads them.

| Arm | Compiled on the first attempt | Passed on the first attempt | Passed in the end | Repair turns |
| --- | --- | --- | --- | --- |
| Lucid | 44 of 50 | 44 of 50 | 50 of 50 | 10, on 6 tasks |
| Lucid, bare guide | 40 of 50 | 40 of 50 | 50 of 50 | 11, on 10 tasks |
| Python | 50 of 50 | 49 of 50 | 50 of 50 | 1, on 1 task |
| TypeScript | 50 of 50 | 46 of 50 | 50 of 50 | 4, on 4 tasks |

Without the pilot block, first-attempt passes are 37 of 40 for Lucid, 33 for the bare guide, 39 for Python, and 36 for TypeScript. With the full guide, blocks 01 and 21 had no failure. As in the pilot, every Lucid failure was a compile error, and every Lucid program that compiled gave the right output.

**Lucid failures after the pilot, by cause.**

- **A brace in text**, both arms, task 40. A `{` with no closing brace gives E0201 `expected '}', found ':'`, which does not say that `{` starts an interpolation. The guide shows `{{` but not `}}`. E0106, for a lone `}`, names the fix.
- **A type named `Order`**, both arms, task 40. E0304 says `40.Order` is already defined, and E0302 then says it is not defined. Neither names the built-in trait it clashes with.
- **A block as a match arm**, both arms, task 41. The guide allows an indented block as an arm but shows none. One subject left out `->`, and one put a `match` straight after it. Neither E0201 says that a block arm is `pattern ->` and then indented lines.
- **A keyword as a name**: `change` in the Lucid arm, task 39, and `empty` in the bare arm, task 09. The E0201 keyword note added after the pilot fixed each in one turn.
- **`+` on text**, bare arm, tasks 31 and 46. The bare guide drops this rule, and E0303 led to interpolation in one turn each.
- **`==` with only an `Order` bound**, bare arm, task 36. E0320 was clear, but no guide says that `Order` does not include `Equal`.
- **A field default that is not a literal**, bare arm, task 07. E0313 fixed it in one turn.

**Other arms.** Python failed task 04 once: `sum` over floats gave a different last digit from the reference's loop, since `sum` compensates for rounding from Python 3.12. The task now says to add in list order. TypeScript failed four tasks once each, by declaring fields in constructor parameters. `tsc --strict` accepts that form, and Node's type stripping rejects it at run time.

**Guide gaps.** Subjects guessed, correctly each time: methods on a generic type, `}}`, the form of `"""` text, that built-in trait names are taken, matching variants of `fs.FileError` from another module, that a literal in sized arithmetic takes the other side's type, the base of a `lucid.toml` dependency path and which sections may be left out, and that the `else` of `if x is none` narrows. Bare-guide subjects also wrote their own text split, number parsing, case change, and list sort. Those are missing from the standard library, not only from the guide.

**What it shows.** Lucid passes on the first attempt less often than Python or TypeScript, at 88% against 98% and 92%, but every task passed in the end, mostly after one repair turn. Most failures came from diagnostics that do not name the fix, or from rules the guide does not state. The bare guide cost 4 first-attempt passes, mostly on rules it removes, and the diagnostics repaired each one in one or two turns. That supports keeping the guide's warnings, and it shows that the diagnostics alone can carry a subject through. 50 tasks per arm cannot separate 44 from 46 with confidence. Limits of the M3 trial still hold: one run per arm, one model family, and one author for the language, the tasks, and the guide.

### The biggest risk

Models have seen billions of lines of Python and none of Lucid. "Syntax Without Semantics" (2026 preprint) describes this gap: models know the algorithm, but cannot express it in an unfamiliar language. No study was found on full languages taught through documentation alone.

So "agent-friendly" is a hypothesis. Clean design does not make up for missing training data. Lucid must close the gap through familiar forms, a compact guide, and strong diagnostics, and it must measure the result.

Sources: [Nogueira et al.](https://arxiv.org/abs/2608.00661), [Wu et al.](https://arxiv.org/pdf/2607.22807), [Scaling Laws for Code](https://arxiv.org/pdf/2512.13472), [Grammar Prompting](https://arxiv.org/pdf/2305.19234), [Syntax Without Semantics](https://arxiv.org/html/2605.15607v1).

## Lexical choices

Decisions 21 and 22. Evidence here is weak: these are small choices taken from existing practice, and none was tested. Level: **None**. The sources below are named from memory and were not read for this project.

- **ASCII names.** Unicode identifiers allow confusable names and direction tricks in source. "Trojan Source" (Boucher and Anderson, 2021) describes the attack, and Unicode UAX #31 and #39 describe mitigations. Rust stayed ASCII-only until it had confusable lints.
- **Multi-line text.** Swift multi-line strings use the closing delimiter to set indentation. Lucid removes common indentation as Python's `textwrap.dedent` does, and takes only the blank closing-line rule from Swift.

Sources: [Trojan Source](https://trojansource.codes/), [UAX #31](https://www.unicode.org/reports/tr31/), [Swift strings](https://docs.swift.org/swift-book/documentation/the-swift-programming-language/stringsandcharacters/).

## Division and calls

Decisions 23, 24, and 26. Evidence is weak: these follow existing practice and none was tested. Level: **None**. The sources below are named from memory and were not read for this project.

- **Division.** Python 3 has `/` yielding a float and `//` flooring (PEP 238), and its `%` takes the sign of the divisor. This pairing keeps the division identity, while C-style truncation does not suit a language with floor division.
- **Argument order.** Swift requires labeled arguments in declared order. Python allows any order, and it evaluates in the order written.

Sources: [PEP 238](https://peps.python.org/pep-0238/), [Python expressions](https://docs.python.org/3/reference/expressions.html), [Swift functions](https://docs.swift.org/swift-book/documentation/the-swift-programming-language/functions/).

## Collections

- **Hash flooding.** PEP 456 made SipHash the default string hash in Python, since a remote attacker could recover the secrets of its earlier FNV-based hash. It calls SipHash the best mix of speed and security it reviewed, and notes that setup cost dominates on very short strings, so it offers a cutoff of about 5 to 7 bytes. Level: Snippet.
- **Rust default.** The `HashMap` docs name SipHash 1-3 with random seeds, and say that the algorithm may change. A 2016 docs change said that DoS resistance is not guaranteed, because of how randomness is acquired. Check the current wording. Level: Snippet.
- **Compact layout.** CPython's `dictobject.c` describes a table of indices alongside an entries array that is mostly append only, so iterating the entries gives insertion order. Level: Snippet, from source comments.

- **Equal and hash.** The Python 3.3 data model says that a class that overrides `__eq__` and does not define `__hash__` has its `__hash__` set to `None`, so its instances raise `TypeError` when hashed. The snippet of the current page did not repeat that sentence, so check it before relying on the wording. Clippy's `derived_hash_with_manual_eq` is in the `correctness` group, states the rule as `k1 == k2` implies `hash(k1) == hash(k2)`, and accepts a manual `Hash` with a derived `PartialEq`. Lucid takes the same one-way rule. Level: Snippet, and the rule's wording came from a third-party copy of the lint docs.
- **Hash of an unordered value.** CPython's `frozenset_hash` shuffles each entry's hash and joins them with XOR, so order does not matter. A CPython report (issue 150134) says that its shuffle spreads changes only upward, which raises collisions. Lucid's entries are already keyed SipHash output, so it needs no shuffle. It adds them, by judgment: no benchmark yet. Level: Snippet.

Sources: [Python data model](https://docs.python.org/3.3/reference/datamodel.html), [Clippy derive lints](https://rust.googlesource.com/rust-clippy/+/refs/heads/master/clippy_lints/src/derive/mod.rs), [CPython issue 150134](https://github.com/python/cpython/issues/150134), [setobject.c](https://chromium.googlesource.com/external/github.com/python/cpython/+/04d497c284ac933488cc747b3f7082beab300848/Objects/setobject.c), [PEP 456](https://peps.python.org/pep-0456/), [Rust HashMap docs change](https://github.com/rust-lang/rust/pull/35371), [CPython dictobject.c](https://chromium.googlesource.com/external/github.com/python/cpython/+/e92ac0a741b125f1cffe8c07b054d1dea7b0a05a/Objects/dictobject.c).

## Boxed trait values

- **Rust.** The Reference says that a pointer to a trait object holds a pointer to the value and one to a vtable, with one vtable per trait implementation. The `std::dyn` page says that dynamic dispatch generally cannot be inlined. Only dyn-compatible traits can be trait objects, per E0038.
- **Swift.** SE-0335 makes existential types explicit with `any`, so that programmers opt into their cost. It was accepted with modifications. A newer proposal, SE-0521, may let `any P?` cover the whole existential: one secondary source, not verified.
- **Lucid.** A vtable pays for separate compilation, which Lucid does not have before modules, and may not need after, since monomorphization is whole-program. Tag tests need no backend work, and a table is a local change. The cost of a long chain is unmeasured. Level: Snippet for the sources, none for the trade-off.

Sources: [Rust trait objects](https://doc.rust-lang.org/reference/types/trait-object.html), [Rust `dyn`](https://doc.rust-lang.org/std/keyword.dyn.html), [E0038](https://doc.rust-lang.org/error_codes/E0038.html), [SE-0335](https://github.com/apple/swift-evolution/blob/main/proposals/0335-existential-any.md).

## Narrowing

Decisions 59 and 60, with `.get` in decision 62. Level: **Read**, for the TypeScript and Kotlin parts below, and **Snippet** for Python. The TypeScript page was read for its first 40,000 characters only, and the end of it, past narrowing by discriminated unions, was seen only as a summary.

- **TypeScript.** Early returns narrow: the handbook says the compiler sees the rest of the body is unreachable for one type, and so removes that type. Assignment narrows too, checked against the declared type. Property paths such as `container.value` narrow as well.
- **Kotlin.** A local `var` smart casts "if the variable is not modified between the check and its usage", and a mutable property never does, "because the variable can be modified at any time by other code". Lucid takes this rule, since a Kotlin smart cast and a Lucid narrowed read both depend on the same proof.
- **What differs.** A wrong type in TypeScript is erased and stays memory safe. A narrowed read in Lucid has no run-time test, so Lucid takes the stricter rule. Lucid's fields cannot change behind a reader, so a field could narrow in a later version with a proof, and allowing it then breaks nothing.
- **Python.** `dict.get` returns `None` for a missing key, and never raises `KeyError`. Summary text, so check the wording.

Sources: [TypeScript narrowing](https://www.typescriptlang.org/docs/handbook/2/narrowing.html), [Kotlin type checks and casts](https://kotlinlang.org/docs/typecasts.html), [Python built-in types](https://docs.python.org/3/library/stdtypes.html).

## Type arguments

Decision 65. PEP 718 and the Rust reference: **Read**. The Go spec and parser: **Snippet**, since the sections on index expressions and `parseIndexOrSliceOrInstance` came through a summary. Check the code before relying on the exact behavior.

- **Go.** `f[T]` uses brackets for both jobs. The summary of the parser quotes this comment: "We can't know if we have an index expression or a type instantiation; so even if we see a (named) type we are not going to be in type context." It reads the first element as an expression and decides from the next token. The summary reported no backtracking.
- **Rust.** The reference says `::` is required before `<` "to avoid ambiguity with the less-than operator". Lucid uses `[`, which is no operator, so it has no need for this form.
- **Python.** PEP 718 is a draft, targeted at 3.16. It proposes `func[int](1, 2)`, and says that type checkers "cannot always infer `T` from the arguments alone (e.g. empty containers, lambdas, or factory callbacks)". So explicit arguments stay necessary even with good inference. It rejects `f<int>(x)`, as that "is already valid syntax with a different meaning."
- **Swift.** Whether a call can specialize a generic task was not settled: the pages read did not say.

Sources: [Go spec](https://go.dev/ref/spec), [Go parser](https://github.com/golang/go/blob/master/src/go/parser/parser.go), [PEP 718](https://peps.python.org/pep-0718/), [Rust paths](https://doc.rust-lang.org/reference/paths.html).

## Generics

Decisions 67 and 68. Level: **Snippet**: all of this came through search summaries, so read the source before relying on the detail.

- **Rust.** The rustc collector finds every concrete instance, starting from non-generic roots and following uses through MIR. `mono.rs` does the same. It has a depth-based recursion limit, and open pull request 156993 says that this check can fire too late for polymorphic recursion, where each step makes the type arguments larger. The pull request adds a check on their length. Whether it merged is not known.
- **Go.** `go/types` infers in stages: given type arguments first, then typed function arguments, then untyped ones. Lucid has the same order, with no untyped literals to treat apart. The searches did not reach Go's rule on instantiation cycles.
- **C++.** Pre-C++20 templates caught type errors at instantiation, in the second phase of lookup. Concepts improve the message at the call site, and a forum thread says they do not move the check of the body to its definition. This is weak evidence, from forum posts and a blog.
- **Not found.** No source was read on which design agents handle best. The M3 agent trial tests E0319 and the rule that a call takes no type from its context.

Sources: [rustc monomorphization](https://rustc-dev-guide.rust-lang.org/backend/monomorph.html), [rustc collector](https://doc.rust-lang.org/nightly/nightly-rustc/rustc_monomorphize/collector/index.html), [Rust PR 156993](https://github.com/rust-lang/rust/pull/156993), [go/types infer.go](https://go.googlesource.com/go/+/refs/tags/go1.17/src/go/types/infer.go), [Go type inference post](https://go.googlesource.com/website/+show/HEAD/_content/blog/type-inference.md), [Lemire on concepts](https://isocpp.org/blog/2025/06/cpp20-concepts-for-nicer-compiler-errors-daniel-lemire).

## Sized numbers

Decisions 79 and 80. Level: **Snippet**: these came through search summaries and mirrors of the spec, so read the current pages before relying on exact wording.

- **Go.** The numeric types section lists `byte` as an alias for `uint8`, and all numeric types are distinct except `byte` and `rune`. The source for `builtin` says that `byte` is equivalent to `uint8` in all ways. An untyped constant can be assigned to any type that can represent it, and `var b byte = 300` is a compile error. Lucid gives the same result for a single literal, with no exact constant math.
- **Rust.** The standard library has `checked_add`, `wrapping_add`, and `overflowing_add` as methods. Lucid takes the wrapping names.
- **Zig.** The language reference makes overflow on `+` Illegal Behavior, and gives `+%`, `-%`, and `*%` for wrapping. So a wrapping operator is also a known design. It costs three new tokens.
- **Not found.** No source was read on which form agents write best. The M3 agent trial can test `x + 1` on a sized value, and the conversion tasks.

Sources: [Go spec](https://go.dev/ref/spec#Numeric_types), [Go builtin](https://go.googlesource.com/go/+/go1.9/src/builtin/builtin.go), [Rust u8](https://doc.rust-lang.org/std/primitive.u8.html), [Zig integer rules](https://zig.guide/language-basics/integer-rules).

## Captures

Decision 81. The Go post: **Snippet** of the primary source. Python: **Snippet**, through the CodeQL page and a tutorial, not the FAQ itself. Swift was not read.

- **Go.** "Fixing For Loops in Go 1.22" says that closures over a loop variable usually saw its final value, and that per-iteration scope is the fix. The team had to correct many tests that were not testing what they thought, and the post cites a Let's Encrypt incident. `go vet` opts for false negatives here, since it is hard to tell whether a reference outlives its iteration.
- **Python.** A closure reads its variable when called, not when made, so lambdas made in a loop all see the last value. The usual fix is `lambda i=i: i`, which binds the value at creation. That is Lucid's only form.
- **Not found.** No study was read on how often agents write closures that change a captured variable, which is what E0325 forbids. The M3 agent trial tests it.

Sources: [Go blog](https://go.dev/blog/loopvar-preview), [CodeQL on loop capture](https://codeql.github.com/codeql-query-help/python/py-loop-variable-capture/).

## Task values

Decisions 82 and 83. Level: **Snippet**: the spec itself was not read, so check "Comparison operators" before relying on the wording.

- **Calling convention, decision 82.** No source was read. It follows from decision 49 and from wanting one call form, so it is a judgment call. M4 benchmarks will show its cost.
- **Go.** A 2011 spec change allows a slice, map, or function value to be compared only to `nil`, and the type checker's test data expects "func can only be compared to nil". A mailing list thread says that function comparison was removed because it made closures surprising. That thread is a discussion, not the spec.

Sources: [spec change, in a Japanese write-up](https://kijimad.github.io/comemo/10370.html), [go/types test data](https://golang.google.cn/src/internal/types/testdata/spec/comparisons.go), [golang-nuts thread](https://groups.google.com/g/golang-nuts/c/b-WtVh3H_oY).

## Parameter modes

Decisions 84 to 88. Swift book and RFC 2025: **Read**, the sections cited. Java: **Read**. SE-0176 was not reloaded for this: its address has moved, so the Swift rule below comes from the book. No source was read on how often agents hit a rule like E0326.

- **Swift `inout`.** The reference defines the steps: the argument is copied at the call, the body changes the copy, and the copy is assigned back on return. It names this "copy-in copy-out" or "call by value result". It also says that reading the original during the call is "a simultaneous access of the value, which violates memory exclusivity". So Lucid's stack slot is the defined behavior, not an approximation. The reference also makes a closure that captures an in-out parameter nonescaping. Lucid has no nonescaping closures yet, so E0325 forbids the capture.
- **Rust two-phase borrows.** RFC 2025 opens with `vec.push(vec.len())` as what it must accept, and lets reads happen during a "reservation" before the mutable borrow starts. This is why decision 86 lets a plain value of the same variable go with a `change`. It covers method calls only, as "a conservative step", which supports starting narrow in Lucid too.
- **Changing a collection while looping.** Java detects this at run time, and its documentation says the check is "best-effort", so it "should be used only to detect bugs". Rust rejects it through borrows. Decision 87 makes it a compile error, with no run-time cost.
- **Not found.** No measurement compares copy-in copy-out with a pointer to the variable. Lucid's form loads and stores once per call, and M4 benchmarks will show the cost.

Sources: [Swift reference, In-Out Parameters](https://github.com/swiftlang/swift-book/blob/main/TSPL.docc/ReferenceManual/Declarations.md), [RFC 2025](https://rust-lang.github.io/rfcs/2025-nested-method-calls.html), [ConcurrentModificationException](https://docs.oracle.com/javase/8/docs/api/java/util/ConcurrentModificationException.html).

## Error handling

Read for decisions 103 and 105. The evidence is weak on both.

- **Rust `must_use`.** The reference ties the lint to an expression statement: "If the expression of an expression statement is a call expression or method call expression whose function operand is a function to which the attribute is applied, the use triggers the `unused_must_use` lint." It calls `let _ = f();` the idiomatic discard. So by its wording a never-read `let` falls to `unused_variables`, and is not a `must_use` case. The page does not state this outright, so this is an inference.
- **Zig.** The fetched language reference says that a `void` expression is the only kind whose value can be ignored. A summary of the page gave `try` as returning the error from the current function. The Errors section was only partly read, so treat this as a pointer, not as a confirmed rule for error unions.
- **Rust `main`.** The `Termination` page says only that failure returns `libc::EXIT_FAILURE`. It gives no stream or format, so it gives no evidence for decision 105. Printing to stderr with exit code 1 comes from the spec and ordinary command-line practice, and is not tested.

No user study or agent trial has tested these. The M5 agent benchmark never raised E0328, so it is still untested.

Sources: [Rust diagnostics attributes](https://doc.rust-lang.org/reference/attributes/diagnostics.html), [Rust Termination](https://doc.rust-lang.org/std/process/trait.Termination.html), [Zig language reference](https://ziglang.org/documentation/master/).

## Stack traces

Read for decision 108.

- **Frame pointers.** PEP 831 reports under 2% geometric mean cost for CPython, and a 64-frame walk in 116 ns on x86-64 Linux. Fedora discussions put the cost at 0% to 2%, read only through a Phoronix report. A 2017 kernel thread reports 5% to 10% on some benchmarks, so the cost depends on the workload.
- **Measured here.** Every Lucid task already calls a trap path, so Cranelift already gave it a frame, and the flag should change little. Three rounds each, with the flag and without: `fib` median 1.82x and 1.88x of Rust, `nbody` best 241 ms and 228 ms, median 248 ms and 266 ms. That is within noise, and three rounds cannot show a small cost.

Sources: [PEP 831](https://peps.python.org/pep-0831/), [Phoronix](https://www.phoronix.com/news/Red-Hat-eu-stacktrace), [kernel thread](https://lkml.iu.edu/1706.0/01381.html).

## `expect`

Read for decision 109. pytest documents that its rewritten `assert` shows "the values of the most common subexpressions including calls, attributes, comparisons, and binary and unary operators", and that re-evaluating an expression after a failure has a side-effect risk. Lucid has one `expect` form, so showing both sides needs no second `expect_eq`. Each side runs once, which avoids that risk. Rust's `assert_eq!` was not read for this. No study compares failure output for agents, and the M5 agent benchmark may show if it helps.

Sources: [pytest assertions](https://docs.pytest.org/en/7.1.x/assert.html).

## Test isolation

Read for decision 110.

- **nextest.** It runs each test in its own process. Its design page says processes are natural fault boundaries, and lets it time, watch, and kill each test with operating system tools. It also lists the cost: tests that share state in a process do not fit.
- **Cost here.** A Lucid process compiles from source, so one process per test would compile once per test. One child that goes on until a failure compiles once, plus once per failure.
- **Not confirmed.** A Zig commit message mentions test runners "respawned as needed". The search did not show what it does, so it is not evidence here. Windows has no `fork`, so a compiled child cannot be copied per test.

Sources: [nextest, why process per test](https://nexte.st/docs/design/why-process-per-test/), [nextest, how it works](https://nexte.st/book/how-it-works.html).

## Unsafe code

Read for decisions 111 and 112.

- **Clippy.** `undocumented_unsafe_blocks` checks that an `unsafe` block has a `// SAFETY:` comment. It sits in the `restriction` group, so it is off by default in Rust. Open issues show false positives around attributes. Lucid has no attributes, and its check looks only at the block's own line and the line above, so most of those cases cannot arise. Security.md already asked for this comment, so a compile error only enforces it. No evidence shows how often such comments are true.
- **Not read.** Rust's RFC 2585 on `unsafe` operations was not found in the search, so it is not evidence here.
- **Weak ground.** Decision 112 rests on the small size of the feature, and on `AGENTS.md` asking for no unneeded work. No source was read on pointer design.

Sources: [Clippy lint source](https://rust.googlesource.com/rust-clippy/+/refs/heads/beta/clippy_lints/src/undocumented_unsafe_blocks.rs).

## Modules

Decisions 116 to 120.

- **Go.** A name in another package needs a qualified form and must be exported. An import's name is scoped to the file that holds it. A blank import exists for initialization side effects, which Lucid avoids by having no module code run.
- **Rust.** The orphan rule limits who may implement a trait, as the Reference states. Lucid has no such limit today: any module may add a task to any type, and a clash is E0304. Decision 126 keeps that, for now, across packages.
- **Swift.** Evolution discussion describes inferring a case from the switched type, with a trap: `.none` on an optional means `Optional.none`. Lucid avoids that trap since `none` is a keyword, so no variant has that name.
- **Weak ground.** Decision 118 is a judgment call, with no study behind it. These were search summaries of the pages, not full reads, and the Swift source is a mailing list and forums, not the language reference.

Sources: [Go spec](https://go.dev/ref/spec), [Rust Reference, implementations](https://doc.rust-lang.org/reference/items/implementations.html), [Swift Evolution thread](https://lists.swift.org/pipermail/swift-evolution/Week-of-Mon-20160208/009940.html), [Swift Forums on `.none`](https://forums.swift.org/t/should-you-ever-have-a-none-case-in-an-enum/58760).

## Packages

Decisions 121 to 127.

- **Cargo.** A `path` dependency is resolved relative to the manifest that names it, and must reach a directory with its own manifest. crates.io rejects a package whose dependencies live outside it, and `path` and `version` can sit together. The default program is `src/main.rs`, and the default library is `src/lib.rs`. A package can hold both, which decision 123 does not copy.
- **Go.** A `replace` line can point at a local directory holding a `go.mod`, and it applies only in the main module. Lucid's `path` is simpler, and has the wider reach, since a dependency's own paths count too. That is safe while paths are local, and the open question on registry packages covers the rest.
- **TOML 1.0.** Repeating a key is invalid. An inline table cannot be extended outside its braces, and takes no comma after its last pair. An array can span lines, and can end with a comma. TOML 1.1 relaxes inline tables, so a manifest in the subset of decision 121 stays valid under both.
- **Weak ground.** These were search summaries of the pages, not full reads. Decision 123's limit to one entry and decision 126's lack of an orphan rule are judgment calls. The file layout has had no trial with agents. The full benchmark at M5 tests it.

Sources: [Cargo, specifying dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html), [Cargo, package layout](https://doc.rust-lang.org/cargo/guide/project-layout.html), [Go modules reference](https://go.dev/ref/mod), [go.mod reference](https://go.dev/doc/modules/gomod-ref), [TOML 1.0](https://toml.io/en/v1.0.0), [TOML 1.1](https://toml.io/en/v1.1.0).

## Standard library

Decisions 128 to 138.

- **Go and Rust.** Both ship their standard library with the toolchain, and both pin it to the compiler version. This is from memory of their install layouts, not read for this entry.
- **Whole-file reads.** Python has `Path.read_text`, Go has `os.ReadFile`, and Rust has `std::fs::read_to_string`. All are from memory, not read for this entry.
- **Clocks.** Rust splits `SystemTime` from `Instant`. Go's `time.Time` carries a wall and a monotonic reading, and `UnixNano` covers the years 1678 to 2262. Hinnant's paper gives the date algorithms, and four results were checked against Python's `datetime`. Rust and Go are from memory, not read for this entry.
- **JSON shape.** serde's page on enum representations states that the externally tagged form is its default, gives `{"Request": {...}}` as its shape, and shows a separate untagged form. That untagged tries variants in order was not confirmed by what was read: it is from memory.
- **JSON limits.** RFC 8259 says that object names should be unique, that many implementations keep only the last of a repeated name, and that an implementation may limit the range and precision of numbers. The wording on nesting limits was seen only through a CPython documentation change. serde_json's default of 128 levels comes from third-party reports that cite its source, not its own documentation.
- **Weak ground.** These are judgment calls with no study behind them, and the sources above were search summaries, not full reads. The test is M5's exit, a standalone `.exe`.

Sources: [serde enum representations](https://serde.rs/enum-representations.html), [RFC 8259](https://datatracker.ietf.org/doc/html/rfc8259), [serde_stacker](https://docs.rs/serde_stacker/latest/src/serde_stacker/lib.rs.html), [a report on the limit](https://github.com/microsoft/pg_durable/pull/328).

## Executables

Decisions 139 and 140.

Level: **Read** (own measurement), 2026-11-07, on the development machine.

- **Same output.** Built with `lucid build`, all 89 run tests and 53 trap tests gave the output and exit code of `lucid run`, traces and stack overflows included. A stack overflow traps with its trace, and does not crash, so the 256 MiB reserve and the limit agree.
- **Same speed.** A release compiler, best of 5, in seconds: `fib` 0.492 under `lucid run` and 0.474 built, `nbody` 0.258 and 0.241, `trees` 1.931 and 1.965. `lucid run` includes JIT compile time, about 7 ms. So loading the stack limit, and baseline x86-64 in place of the host's features, have no cost that this shows.
- **Size.** `hello.exe` is 157 KB, and `todo.exe` 366 KB. The embedded runtime library is 12 MB, and the linker keeps only what the program reaches.
- **Linker search.** The MSVC linker needs `LIB` set to the C runtime and Windows SDK library directories. Outside a developer prompt nothing sets it, and Git Bash puts GNU `link` first on `PATH`. `find-msvc-tools` 0.1.15 tries the environment of a developer prompt, then the setup API of Visual Studio 2017 and later, then the registry for 2015. It gives `link.exe` with `LIB` and `PATH` set, and has no dependencies. Read in its source, `find_tools.rs` and `Cargo.toml`.
- **Weak ground.** That `/STACK` reserves and does not commit, and that `std::process::exit` flushes stdout, are from memory of the Microsoft and Rust documentation. The tests above show both behaviors, but on one machine.

Sources: [find-msvc-tools](https://docs.rs/find-msvc-tools), [MSVC /STACK](https://learn.microsoft.com/en-us/cpp/build/reference/stack-stack-allocations), [rustc --print native-static-libs](https://doc.rust-lang.org/rustc/command-line-arguments.html#--print-print-compiler-information), [std::process::exit](https://doc.rust-lang.org/std/process/fn.exit.html).

## Error explanations

Decision 141.

- **rustc.** `rustc --explain E0308` prints a Markdown entry with a cause and code examples, and the entries live in the compiler source as one file per code. Its examples are compiled by its test suite. This is from memory of the Rust repository, not read for this entry.
- **Weak ground.** No study shows that agents fix errors faster with an explanation than with the message alone. The M5 agent benchmark did not record whether subjects ran `lucid explain`, so it is still open.

Sources: [Rust error codes index](https://doc.rust-lang.org/error_codes/error-index.html).

## Fixes

Decision 143.

- **rustc and cargo fix.** rustc tags each suggestion with an applicability, and `cargo fix` applies only those marked machine-applicable. Its name suggestions accept an edit distance of up to a third of the name's length. Both are from memory of the Rust compiler source, not read for this entry.
- **Weak ground.** The fixes chosen here are the obvious ones. No measurement yet shows how often agents hit them, and the M5 agent benchmark did not record it.

Sources: [cargo fix](https://doc.rust-lang.org/cargo/commands/cargo-fix.html), [rustc edit_distance](https://doc.rust-lang.org/nightly/nightly-rustc/rustc_span/edit_distance/index.html).

## Formatting

Decision 144.

- **One style, no options.** gofmt has no style options, and Effective Go says Go has no line length limit. The Go Proverbs put the result as "Gofmt's style is no one's favorite, yet gofmt is everyone's favorite." Black calls itself uncompromising for the same reason, though it keeps a line length option.
- **Two spaces before a trailing comment.** PEP 8 asks for at least two, and Black writes two.
- **Weak ground.** No measurement shows that a canonical layout helps agents. The case rests on smaller diffs and on Go's long use of gofmt. The M5 agent benchmark did not test it.

Sources: [Effective Go, Formatting](https://go.dev/doc/effective_go#formatting), [Go Proverbs](https://go-proverbs.github.io/), [Black code style](https://black.readthedocs.io/en/stable/the_black_code_style/current_style.html), [PEP 8, Comments](https://peps.python.org/pep-0008/#comments).

## M5 open questions

Decisions 151, 152, and 153. Evidence is the M5 agent benchmark logs, plus language references recalled, not read for this entry.

- **Top-level values.** The Go spec orders package-level variables by their dependencies, and reports a cycle. Swift initializes each global lazily, on first use, behind a one-time check. Rust requires a `static` to have a constant initializer, and adds lazy globals only through `LazyLock` in the library. In 100 Lucid solutions of the benchmark, no attempt hit E0901 for a top-level `let`. Two subjects listed "top-level let of text" as a guess, and it was a literal, which works. Level: **None** for the languages, own logs for the count.
- **Literal types.** In 50 TypeScript solutions, a search for a union of string literals found none. No Lucid subject tried to write one. Level: own logs.
- **Bit operations.** Python and TypeScript solutions used `& 0xFFFFFFFF`, `& 0xFF`, and `>>> 0` for hashes, checksums, and a random number generator. The M3 trial's subjects called the hex task hard for lack of bit operations, see "M3 agent trial". Python's reference puts `|`, `^`, `&`, and shifts above comparisons, unlike C. Level: own logs, and **None** for the precedence claim.
- **Weak ground.** The benchmark tasks did not target these features, so absence of use is weak evidence.

Sources: [Go spec, Package initialization](https://go.dev/ref/spec#Package_initialization), [Swift, Global and Local Variables](https://docs.swift.org/swift-book/documentation/the-swift-programming-language/properties/#Global-and-Local-Variables), [Rust reference, Static items](https://doc.rust-lang.org/reference/items/static-items.html), [Python, Operator precedence](https://docs.python.org/3/reference/expressions.html#operator-precedence).

## License

Decision 154.

- **The problem.** GCC's exception says GCC may combine its runtime libraries with a compiled program, and that the exception exists to let non-GPL programs use them. Lucid has the same shape: `lucid build` puts `lucid_rt`, `std`, and derived tasks into every program.
- **Why not GCC's text.** It defines "GCC" as the GNU Compiler Collection, and the document may be copied only verbatim. Using it would need a rewritten exception, which is new legal text.
- **The Classpath exception.** It permits linking the library with independent modules into an executable under any terms. It speaks of linking, so `LICENSE-EXCEPTION` states that compiling `std` into a program counts, and covers generated code too. That added notice is the owner's grant, and has not had legal review.
- **Weak ground.** No lawyer read this. Have it checked before the first public release.

Level: **Read** for both exception texts, through SPDX's copies.

Sources: [GCC Runtime Library Exception 3.1](https://www.gnu.org/licenses/gcc-exception-3.1.html), [SPDX GCC-exception-3.1](https://spdx.org/licenses/GCC-exception-3.1.html), [SPDX Classpath-exception-2.0](https://spdx.org/licenses/Classpath-exception-2.0.html), [GPL version 3](https://www.gnu.org/licenses/gpl-3.0.html).

## Open research

| Topic | Needed by |
| --- | --- |
| Reference counting cost in imperative code | Measured at M4, see "M4 benchmarks". Five programs only |
| Foreign calls with lightweight tasks | M7 |
| Status of WebAssembly stack switching | M7 |
