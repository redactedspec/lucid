# Vision

Lucid should be easier to write than Python, as safe as Rust, and fast
enough to replace C++ or Rust in most programs. It should also be the
language AI agents write most reliably. Those aims pull against each
other, and this document says how to choose when they do.

## Goals

Lucid is meant to be:

1. **Readable.** Someone who does not know the language can guess what a
   program does.
2. **General.** Scripts, command-line tools, servers, websites, and native
   applications, in one language.
3. **Fast.** Compiled to machine code and to WebAssembly, with no garbage
   collector and no interpreter.
4. **Safe.** Safe code has no memory errors, data races, null errors, or
   unchecked failures.
5. **Easy for agents.** The grammar is unambiguous, there is one obvious
   way to do most things, a task can be understood from its signature, and
   every tool has machine-readable output.

## When goals conflict

Settle it in this order:

1. **Safety.** No guarantee is given up for convenience or for speed.
2. **Clarity.** Code is read more often than it is written, and an agent
   has to read intent exactly.
3. **Simplicity.** Fewer concepts beat more power.
4. **Speed.** The default should be fast. Control is offered where
   profiling shows it is needed.
5. **Brevity.** Short code is good, unless it costs any of the above.

A feature that adds a second way to do something has to clear this list
with room to spare.

## What it costs

"Python ease with C++ speed" cannot be entirely true. Lucid pays for it
in these ways.

Task signatures need types, where Python needs none. Locals and lambdas
are inferred, so ordinary code stays light.

Ownership is hidden, so the compiler cannot prove everything, and some
programs pay for reference counts that hand-written Rust would not have.
Heavily shared data costs the most. `lucid check --costs` shows where.

Peak speed comes in stages. Cranelift compiles about ten times faster
than LLVM, but published measurements put its code from 14% to twice as
slow on some benchmarks. At M4 a tight floating-point loop ran at 3.4x
the time of Rust, mostly for want of vectorizing. An LLVM backend for
release builds remains possible.

Nobody has shown that agents write Lucid well. Models have years of
Python in their training data and no Lucid at all. The benchmark so far
is in [research.md](research.md#m5-agent-benchmark).

A cycle of `shared` references leaks unless one side is `weak`. A leak
is safe, but it is still a bug.

There are no stored references. Designs common in C++ or Rust, such as a
node that points at its parent, need indexes or `shared` instead.

## Non-goals

* Operating system kernels and hard real-time software, in version 1.
  Lucid has a small runtime.
* Inheritance, exceptions, macros, implicit conversions, and null. Each
  one breaks reasoning about a piece of code on its own.
* Source compatibility with any other language.
* Configurable syntax, or a formatter with settings.

## Version 1.0 promise

A program that builds under version 1.0 builds and behaves the same under
every 1.x release. This has held since the end of M8.

The promise covers the language as [language.md](language.md) and
[grammar.md](grammar.md) define it, the `std` modules and the prelude,
`lucid.toml` and `lucid.lock`, the commands and their flags, the meaning
of each error code, and the fields of `--json` output, to which new
fields may be added.

It does not cover the wording of messages, the runtime's memory layout or
the files a build writes, the speed or size of built programs, or
anything marked planned, such as `shared` and `weak`.

A fix for a safety hole may break a program that relied on the hole.
There are no editions: a 1.x compiler reads every program one way.

## Measures

| Measure | Target |
| --- | --- |
| Code size of simple tasks | Same number of lines as Python, or fewer, on the example suite |
| Speed | Within 2x of Rust release builds on the benchmark suite |
| Memory | At most Rust plus 20% on the benchmark suite |
| Compiling hello world | Under 100 ms |
| Checking | Over 100,000 lines a second |
| Agents | First-attempt pass rate at or above Python, on the benchmark in [agents.md](agents.md) |
| Size of the language | The whole reference fits in under 15,000 tokens |
