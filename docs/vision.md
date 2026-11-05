# Vision

## The promise

Lucid should be easier to write than Python, safe like Rust, and fast enough to replace C++ or Rust in most applications. It should also be the language that AI agents write most reliably.

## Goals

1. **Readable.** A non-expert can guess what a program does.
2. **General.** Scripts, command-line tools, servers, websites, and native applications, all in one language.
3. **Fast.** Compiled to native code and WebAssembly. No garbage collector and no interpreter.
4. **Safe.** Safe code has no memory errors, data races, null errors, or unchecked failures.
5. **Agent-friendly.** Unambiguous grammar, one obvious way to do things, local reasoning, and machine-readable tools.

## Priority order

Goals conflict. Resolve conflicts in this order:

1. **Safety.** Never give up a guarantee for convenience or speed.
2. **Clarity.** Code is read more often than written, and agents must parse intent exactly.
3. **Simplicity.** Fewer concepts beat more power.
4. **Speed of programs.** Make the default fast. Offer control where profiling demands it.
5. **Brevity.** Short code is good, unless it costs any of the above.

## The honest trade-offs

"Python ease with C++ speed" cannot be fully true. These are the prices Lucid pays:

- **Task signatures need types.** Python needs none. Locals and lambdas are inferred, so everyday code stays light.
- **Some programs pay for reference counting.** Lucid hides ownership, so the compiler cannot prove everything. Heavily shared data costs more than hand-tuned Rust. `lucid check --costs` shows where.
- **Peak speed is a staged target.** Cranelift compiles about 10x faster than LLVM, but published data shows code from 14% slower up to 2x slower on some benchmarks. An LLVM release backend stays possible later.
- **Agent-friendliness is unproven.** Models have training data for Python and none for Lucid. See [research.md](research.md).
- **Reference cycles leak unless `weak` is used.** A leak is safe but still a bug.
- **Stored references do not exist.** Some designs common in C++ or Rust need indexes or `shared` instead.

## Non-goals

- Operating system kernels and hard real-time software in version 1. Lucid has a small runtime.
- Inheritance, exceptions, macros, implicit conversions, and null. Each one breaks local reasoning.
- Source compatibility with any other language.
- Customizable syntax or formatter settings.

## Success measures

| Measure | Target |
| --- | --- |
| Simple task code size | Same or fewer lines than Python on the example suite |
| Runtime speed | Within 2x of Rust release builds on the benchmark suite |
| Memory | No more than Rust plus 20% on the benchmark suite |
| Hello-world compile | Under 100 ms |
| Check speed | Over 100,000 lines per second |
| Agent success | First-attempt test pass rate at or above Python, per the benchmark in `agents.md` |
| Language size | Whole language reference fits in under 15,000 tokens |
