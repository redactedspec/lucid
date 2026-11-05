# Research

Evidence behind Lucid's design. Reviewed on 2026-11-04. Add to this file whenever a decision is made or challenged.

**Evidence levels**

- **Read:** the primary source text was read.
- **Snippet:** seen only through search excerpts or a summary. Read the source before relying on the exact figure.
- **None:** no direct evidence found. The decision is a judgment call, to be tested.

## Summary

| Decision | Verdict | Level | Action |
| --- | --- | --- | --- |
| Reference counting with reuse | Supported, but in functional languages only | Read | Benchmark at M4 |
| Mutable value semantics | Supported as research, little production proof | Snippet | Prototype at M3 |
| Exclusivity checks | Supported, static checks alone are not enough | Snippet | Rule added: no top-level `var` in tasks |
| Cranelift for speed | Partly supported. 2x target is at risk | Read | Keep LLVM option open |
| Overflow traps in release | Measured: 1.17x to 1.78x in Lucid, 1.56x to 1.92x in Rust | Read | Keep. Revisit with M4 benchmarks |
| Permissions | Supported, but first design had holes | Snippet | `security.md` fixed |
| Agent-friendliness | Weak. Biggest gap | Snippet | Test with agents from M3 |
| Indentation syntax helps agents | No evidence | None | Owner preference. Measure |
| No `async`, lightweight tasks in browser | Now feasible through JSPI | Snippet | Decide at M7 |
| ASCII names, text escapes | Judgment call from existing practice | None | Revisit before 1.0 |
| Division, argument order | Judgment call from existing practice | None | Agent trial at M3 |
| Text slices, no character type | Judgment call from existing practice | None | Agent trial at M3 |
| Compact ordered map, SipHash with random keys | Supported by Python and Rust practice | Snippet | Benchmark at M4 |

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

Sources: [Google Research listing](https://research.google/pubs/mutable-value-semantics/), [preprint](https://daveabrahams.com/wp-content/uploads/2022/02/implementation_strategies_for_mutable_value_semantics-preprint.pdf), [Hylo docs](https://docs.hylo-lang.org).

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

**For Lucid:** the first design had two holes. `process` could also bypass all limits, and a package could reach the network indirectly through a permitted dependency. Both are fixed in `security.md`. Permissions limit careless and casual misuse of dependencies. They are not a sandbox.

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

Sources: [PEP 456](https://peps.python.org/pep-0456/), [Rust HashMap docs change](https://github.com/rust-lang/rust/pull/35371), [CPython dictobject.c](https://chromium.googlesource.com/external/github.com/python/cpython/+/e92ac0a741b125f1cffe8c07b054d1dea7b0a05a/Objects/dictobject.c).

## Open research

| Topic | Needed by |
| --- | --- |
| Read Racordon et al. and the Hylo language tour in full | M3 |
| Agent trial with a new language and only a guide | M3 |
| Reference counting cost in imperative code | M4 |
| Foreign calls with lightweight tasks | M7 |
| Status of WebAssembly stack switching | M7 |
