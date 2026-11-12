---
title: Getting started
description: Write, run, check, test, and build a first Lucid program.
---

Lucid is a general-purpose, compiled language that reads like plain English. Programs compile to native code through Cranelift, or to WebAssembly for the browser. Ownership is inferred, so you write no lifetimes, and there is no garbage collector.

:::note[Version 1.0.0]
Lucid 1.0.0 builds from source; binaries are not published yet. See [Installation](/docs/installation/) for how to build it, and the [roadmap](/roadmap/).
:::

## A first program

Source files end in `.lcd`. Save this as `hello.lcd`:

```lucid title="hello.lcd"
task greet(name: text) -> text
    return "Hello, {name}"

print greet("world")
```

A `task` is a function. Its parameters and return type are always written out. Blocks are indented by four spaces, and `{name}` inside text inserts a value.

## Run it

```sh
lucid run hello.lcd
```

```text
Hello, world
```

`lucid run` compiles the file to machine code in memory and runs it at once. Words after the path go to the program:

```sh
lucid run todo.lcd add "buy milk"
```

## Check it

`lucid check` reports errors without running anything. It prints nothing when the program is correct.

```sh
lucid check hello.lcd
```

A mistake gets a stable code, a position, and often a fix:

```text
typo.lcd:2:7: error E0301: name `totl` is not defined
  fix: rename to `total`
```

`lucid fix typo.lcd` applies that fix, and `lucid explain E0301` explains the code. Add `--json` to any of these to get the same thing as data.

## Test it

Tests live next to the code:

```lucid title="math.lcd"
task add(a: int, b: int) -> int
    return a + b

test "adds numbers"
    expect add(2, 3) == 5
```

```sh
lucid test math.lcd
```

```text
ok adds numbers
1 tests, 0 failed
```

A failed `expect` on a comparison shows both sides, such as `expect failed: 4 == 5`. `lucid run` skips tests.

## Build it

```sh
lucid build hello.lcd               # writes hello.exe (hello on Linux), a standalone program
lucid build --target web hello.lcd  # writes hello.js, for a browser or Node
```

Native builds work on Windows and Linux x86-64.

## Format it

```sh
lucid fmt hello.lcd
```

There is one layout and no settings, so formatting never comes up in review.

## Stability

Lucid keeps a version 1.0 promise: a program that builds under 1.0 builds and behaves the same under every 1.x. It covers the language, the `std` modules, `lucid.toml` and `lucid.lock`, the commands and their flags, the meaning of each error code, and the fields of `--json` output, which may gain fields. It does not cover message text, built files, or speed. A fix to a safety hole may break a program that relied on it.

## Next

Read the language pages in order, starting with [Syntax](/docs/syntax/). The whole reference is small enough to read in one sitting.
