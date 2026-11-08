---
title: Writing Lucid with AI agents
description: The agent guide, JSON diagnostics, and a check-fix-test loop for coding agents that write Lucid.
---

Lucid is designed to be written by AI coding agents as reliably as by people. Models have no Lucid training data, so two things carry the load: a compact guide, and diagnostics that a program can read.

## Give the agent the guide

[`/agent-guide.md`](/agent-guide.md) holds everything that runs today, in one plain-text file sized to fit in a context window. Every code block in it runs, and the compiler's test suite checks that it stays true and stays under its size budget.

Put it in the agent's context with the task, or tell the agent to fetch it:

```text
Write this program in Lucid. Lucid has no training data, so do not assume
features from Python, TypeScript, or Rust. Read https://lucidlang.dev/agent-guide.md
first, and use only what it shows.
```

## The loop

Every command has stable `--json` output, so an agent can run a tight loop:

```sh
lucid check --json main.lcd     # 1. read the diagnostics
lucid fix --json main.lcd       # 2. apply the fixes the compiler is sure of
lucid explain --json E0326      # 3. look up any code it does not know
lucid test --json main.lcd      # 4. run the tests, with values on failure
lucid fmt main.lcd              # 5. put the file in the one layout
```

A diagnostic gives a stable code, an exact position, and a fix when one is near certain:

```json
{
  "code": "E0301",
  "severity": "error",
  "message": "name `totl` is not defined",
  "file": "typo.lcd",
  "line": 2,
  "column": 7,
  "end_line": 2,
  "end_column": 11,
  "notes": [],
  "fixes": [
    {
      "description": "rename to `total`",
      "edits": [{ "line": 2, "column": 7, "end_line": 2, "end_column": 11, "text": "total" }]
    }
  ]
}
```

Codes never change meaning once released, so an agent's knowledge of them stays valid. Every code is listed in [Compiler diagnostics](/docs/diagnostics/).

## Reading an API

`lucid context` prints the public tasks and types of a module, a file, or a package, with their `##` doc comments and no bodies. It works on a file that has errors.

```sh
lucid context std.fs
lucid context --json ./models.lcd
```

## Why the language helps

These rules make Lucid easier for an agent to get right, and each one is a test for new features:

- **One way.** Each common job has one form, so the agent has nothing to choose between.
- **Local reasoning.** A task's signature shows its inputs, what it changes (`change`), what it owns (`take`), and how it fails (`or` an error type).
- **Explicit at call sites.** `change` marks mutation, `try` marks an early exit, and `spawn` marks a thread.
- **No hidden control flow.** No exceptions, macros, implicit conversions, or inheritance.
- **Context-free parsing.** Any snippet parses without knowing other files.
- **No formatting decisions.** `lucid fmt` has no options, so diffs show only meaning.

## Common errors

| Code | Usual cause |
| --- | --- |
| E0201 | Syntax, often a `:` after `if`, or a missing indent |
| E0301 | A name is not defined, often a built-in that does not exist |
| E0303 | Types do not fit, often `int` with `float`, or `text + text` |
| E0314 | An empty `[]` or `{}` with no type: add an annotation |
| E0316 | A `match` does not cover every case |
| E0324 | A lambda has no context for its parameter types |
| E0326 | A value is used while something changes it: split the statement |
| E0328 | An error is not used: add `try`, `else`, or `match` |

## Limits

Agent-friendliness is a design goal, not a proven result. The guide and the diagnostics have to make up for the training data that Python and TypeScript have.
