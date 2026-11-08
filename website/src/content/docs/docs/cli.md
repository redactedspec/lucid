---
title: CLI reference
description: Every lucid command, its flags, and its exit codes.
---

```text
lucid run [PATH [ARGUMENTS]]
lucid build [--target web] [--json] [PATH]
lucid check [--json] [--costs | --unsafe] [PATH]
lucid test [--json] [PATH]
lucid fix [--json] [PATH]
lucid explain [--json] CODE
lucid context [--json] MODULE
lucid fmt [--check] [PATH]
```

`PATH` is a `.lcd` file or a package directory. A directory means the nearest package at or above it, and no `PATH` means the current directory.

## Commands

| Command | What it does |
| --- | --- |
| `lucid run` | Compiles with a JIT and runs. Words after the path go to the program |
| `lucid check` | Reports errors only, with no code generation |
| `lucid check --costs` | Lists each place a value is copied or counted, from the ownership pass |
| `lucid check --unsafe` | Lists every `unsafe` block and `extern` task, in the program and all it imports |
| `lucid test` | Runs the `test` blocks of a file, or of every file in a package's `src` |
| `lucid build` | Writes a standalone program to the current directory: `NAME.exe` on Windows, `NAME` on Linux. Windows and Linux x86-64 |
| `lucid build --target web` | Writes `NAME.js`, which holds the program as WebAssembly and loads it in a browser or Node |
| `lucid fmt` | Puts files in the one canonical layout. `--check` changes nothing, and lists files that would change |
| `lucid fix` | Applies the fixes that diagnostics show as `fix:` |
| `lucid explain CODE` | Prints the cause, an example, and a fix for an error code |
| `lucid context MODULE` | Prints the public API of a file, a package, or a `std` module, with its doc comments |

`lucid lsp`, a language server, and `lucid package`, for the registry, are planned for M8.

## JSON output

`--json` gives stable, machine-readable output. A diagnostic looks like this:

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

`column` counts Unicode characters from 1, and `end_column` is exclusive. Output is sorted by position, and one run reports every error it can find.

`lucid test --json` gives each test's `name`, `passed`, `file`, `line`, `column`, and `output`, which holds what the test printed and, on failure, the trap message and trace.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | A compile error, a failed test, or a failed `try` at top level |
| 2 | Bad usage |
| 101 | A trap at run time, or a compiler bug |

## Examples

```sh
lucid run examples/hello.lcd
lucid check --json src/main.lcd
lucid check --costs .
lucid test --json .
lucid context std.fs
lucid explain E0326
lucid fmt --check .
```
