# The Lucid Programming Language

Lucid is a compiled language that reads like plain English, needs no
garbage collector or lifetime annotations, and is memory safe outside
`unsafe` blocks. It was designed from the start to be written by AI agents
as well as by people.

[Agent guide](docs/agent-guide.md) · [Language](docs/language.md) ·
[Grammar](docs/grammar.md) · [Memory model](docs/memory-model.md) ·
[Security](docs/security.md) · [Standard library](docs/stdlib-and-web.md) ·
[Errors](docs/errors.md) · [Examples](examples/) ·
[lucidlang.dev](https://lucidlang.dev)

```lucid
task greet(name: text) -> text
    return "Hello, {name}!"

let people = ["Ada", "Linus"]

for person in people
    print greet(person)
```

A block is four spaces of indent. A `task` is a function. There are no
semicolons, no braces around blocks, no colons after `if`. `lucid run
hello.lcd` compiles that file to machine code and runs it, and `lucid build`
writes a standalone program for Windows or Linux, or a `.js` file for a
browser.

## Why Lucid

* **Values, not references.** `var b = a` gives `b` its own copy of `a`,
  as far as the program can tell. The compiler makes that cheap: a value
  used for the last time moves, a shared buffer is copied only when it is
  written, and a buffer held once is written in place. Nothing dangles,
  because nothing points, so there is no lifetime syntax to learn.
* **No null, no exceptions, no silent overflow.** A value that may be
  missing has type `T?` and must be tested before use. A task that can fail
  says so in its signature, and the caller must handle the error or pass
  it on with `try`. Integer overflow traps in every build.
* **One way to write most things.** There is one loop over a collection,
  one way to join text, one layout, and `lucid fmt` has no options. An
  agent has fewer choices to get wrong, and a diff shows only what changed.
* **Tools that speak JSON.** `check`, `build`, `test`, `fix`, `explain`,
  and `context` all take `--json`. Every compile error has a stable code,
  and `lucid explain E0326` prints its cause, an example, and the fix.
* **Packages ask for their powers.** A dependency cannot touch files, the
  network, or the environment unless `lucid.toml` grants it, and it cannot
  use `unsafe` unless that is granted too.
* **Threads without data races.** `spawn`, `channel`, and `lock` are the
  only ways to share work. A value sent to another thread is marked, and
  its reference counts turn atomic only then.
* **Native and web from one source.** Code generation uses Cranelift for
  machine code and emits WebAssembly for the browser. The
  [to-do page](examples/web) is written only in Lucid.

It is not free. Task signatures need types. Shared data pays for reference
counts that Rust would not have. On five benchmarks Lucid ran between 0.62x
and 3.38x the time of `rustc -O`, met the targets of
[vision.md](docs/vision.md) on four, and is slowest where LLVM vectorizes
and Cranelift does not ([numbers](docs/research.md#m4-benchmarks)). In a
50-task benchmark, an agent with the [guide](docs/agent-guide.md) passed
44 tasks on the first attempt in Lucid, against 49 in Python and 46 in
TypeScript, and all 50 after repair.

## A closer look

### Errors are values

```lucid
choice ParseError
    Empty
    Bad(line: int)

task error_message(self: ParseError) -> text
    return match self
        Empty -> "empty input"
        Bad(line) -> "bad line {line}"

task parse(source: text) -> int or ParseError
    if source is empty
        return Empty
    return source.len()

task twice(source: text) -> int or ParseError
    let size = try parse(source)    # hands the error to the caller
    return size * 2

print twice("") else 0              # or give a fallback
```

An error that nobody looks at does not compile. `try` is the only early
exit that is not `return`, and it is visible at the call.

### Changing a value is visible at the call

```lucid
task add_all(take items: list[int], extra: int) -> list[int]
    items.push(extra)
    return items

let numbers = [1, 2, 3]
let more = add_all(numbers, 4)   # last use of numbers: moved, no copy
```

A parameter is a read-only view unless it says `change` or `take`, and the
caller writes the same word. While a value is passed as `change`, nothing
else may read or write it; the compiler rejects `swap(change a, change a)`
and a loop that pushes onto the list it walks. `lucid check --costs` shows
each place where a copy-on-write can copy. The full rules are in the
[memory model](docs/memory-model.md).

### Threads

```lucid
task worker(jobs: channel[int], total: lock[int])
    while true
        let job = jobs.receive()
        if job is none
            break
        total.update(n => n + job * job)
```

A thread that takes a lock it already holds traps, and so does a program
whose threads are all waiting, instead of hanging. `std.net` and `std.http` build on these
for servers; [website.lcd](examples/website.lcd) serves a page to 64
clients at once.

## Status

Version 1.0.0. The compiler, written in Rust, implements the whole
language as [language.md](docs/language.md) defines it, and passes every
test on Windows and on Linux x86-64. A second front end, written in Lucid
under `compiler/lucid/`, matches the Rust one stage by stage and compiles
itself to a byte-identical result. macOS is not supported yet.

From 1.0 on, a program that builds keeps building and behaving the same
under every 1.x release. [vision.md](docs/vision.md#version-10-promise)
says exactly what that covers. `shared` and `weak` references are planned
and not part of the promise.

## Building from source

You need Rust with the `wasm32-unknown-unknown` target, and, for `lucid
build`, the MSVC linker on Windows or `cc` on Linux. No binaries are
published yet.

```sh
git clone https://github.com/redactedspec/lucid
cd lucid
rustup target add wasm32-unknown-unknown
cargo build --release
target/release/lucid run examples/hello.lcd
```

The binary carries the standard library and the runtime, so it can be
copied anywhere. `cargo test` runs the unit and golden tests. How the
compiler is put together, and how a release is made, is in
[compiler.md](docs/compiler.md).

## Documentation

For people writing Lucid:

* [Agent guide](docs/agent-guide.md): everything that runs today, by
  example, in one file. Despite the name it is the quickest way in for
  anyone.
* [Language](docs/language.md) and [grammar](docs/grammar.md): the exact
  rules. Where the compiler and these disagree, one of them has a bug.
* [Memory model](docs/memory-model.md): values, `change`, `take`, and what
  the compiler does with them.
* [Security](docs/security.md): what safe code cannot do, `unsafe`, and
  package permissions.
* [Standard library and the web](docs/stdlib-and-web.md).
* [Errors](docs/errors.md): every error code, with an example and a fix.
* [Tools for agents](docs/agents.md): the rules a feature must pass, and
  every command's JSON.

For people working on Lucid:

* [Vision](docs/vision.md): what Lucid is for, and how to settle a
  trade-off.
* [Compiler](docs/compiler.md): the pipeline, each pass, and the release
  steps.
* [Roadmap](docs/roadmap.md): milestones and what is left.
* [Decisions](docs/decisions.md) and [research](docs/research.md): why
  each choice was made, and the evidence, kept as written at the time.
* [Claude Code](docs/claude-code.md): how this repository is worked on.
  Lucid is designed and built by Claude, with its owner directing.

The public site, [lucidlang.dev](https://lucidlang.dev), is built from
[website/](website/).

## License

Lucid is licensed under either of the [MIT license](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option. A program
built with Lucid may use any license.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in Lucid by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
