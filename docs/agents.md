# Designing for Agents

"Easy for agents to write" means nothing unless it can be checked. This
document turns it into rules a proposed feature must pass, lists the
tools and the shape of their output, and describes the benchmark that
measures whether any of it works.

## Language rules

1. **One way.** Each common job has one idiomatic form. An agent does
   not have to choose, and output stays consistent.
2. **Context-free parsing.** Any snippet parses without knowledge of
   other files.
3. **Local reasoning.** A task's signature says everything a caller
   needs: its inputs, what it changes (`change`), what it takes over
   (`take`), and how it fails (`or` an error type).
4. **No hidden control flow.** No exceptions, macros, implicit
   conversions, or inheritance. Operators can be overloaded only for
   arithmetic, equality, and ordering, through traits.
5. **Visible at the call.** `change` marks mutation, `try` an early exit,
   `spawn` a new thread.
6. **Familiar on purpose.** Where possible, use forms common in Python
   and TypeScript, which models have seen most. New syntax needs a
   reason.
7. **Small.** The whole language must fit in one context window; see the
   budget in [vision.md](vision.md#measures).
8. **No layout decisions.** `lucid fmt` has no options, so a diff shows
   only meaning.

## Tools

Every command has stable `--json` output.

| Command | What it gives |
| --- | --- |
| `lucid check --json` | Diagnostics, quickly, with no code generation |
| `lucid build --json` | A standalone executable, or the diagnostics `lucid check --json` would give |
| `lucid build --target web` | `NAME.js`, which runs the program in a browser or under `node`. A user `extern` or `.pointer()` there is E0905. `lucid run` of a program that uses `std.dom` is E0906 |
| `lucid fix [--json] [PATH]` | Makes the fix of each error that has one, in place, and lists each as `file:line:column: fixed CODE: description`, or as `code`, `description`, `file`, `line`, and `column` with `--json`. If two fixes touch the same text only the first is made; a second run makes the next. A lexer error stops checking, so its fixes come one run before the checker's. `std` is never changed |
| `lucid explain [--json] CODE` | The cause, an example, and the fix for an error code, from [errors.md](errors.md). `--json` gives `code`, `title`, `cause`, `example`, and `fix` |
| `lucid test --json` | Results per test, for a file or for every file in a package's `src`: `name`, `passed`, `file`, `line`, `column`, and `output`, which holds what the test printed and, on failure, the trap message and trace |
| `lucid fmt [--check] [PATH]` | Puts a file, or every file of a package, in the one layout, changing only spaces and line breaks. `--check` changes nothing, lists each file that would change, and exits with 1 if any would. A file with a lexer error gets its diagnostics and is left alone |
| `lucid package [PATH]` | Checks a library, writes its archive `NAME-VERSION.lpk` to the current directory, and prints its line for the registry's index |
| `lucid context [--json] MODULE` | The public API of a file, a package directory, or `std.NAME`, compactly: each public header with its `##` doc comment, and the fields, variants, or tasks of a type, choice, or trait, with no bodies. `--json` gives `module`, `line`, `kind`, `name`, `signature`, and `doc` |
| `lucid doc --json` | The full docs, as data |
| `lucid lsp` | A language server on stdin and stdout: diagnostics, formatting, go to definition, and hover showing a name's type or declaration and its `##` comment. Positions count UTF-16 units. See "Editors" below |
| `lucid --version [--json]` | `lucid 1.0.0`, or `{"version":"1.0.0"}`, from the crate's version |
| `lucid check --unsafe` | Each `unsafe` block and `extern` task, as `file:line:column: unsafe: kind`, or as `kind`, `file`, `line`, and `column` with `--json`. `kind` is `block` or `extern`. `std` is left out |
| `lucid check --costs` | Each copy and reference-count site, as `file:line:column: kind: message`, or as `kind`, `type`, `file`, `line`, and `column` with `--json`. A `copy` is a value held twice that a later write changes, which costs O(n) if both holders stay. A `count` costs one count operation. Exits with 0. A program with an error or an unbuilt feature reports that instead and exits with 1 |

## Editors

Point any editor with a language server client at `lucid lsp`, for files
ending in `.lcd`. It syncs whole documents. No editor extension ships yet
(decision 202), so VS Code, which needs one, is not supported. In Neovim
0.11 or later:

```lua
vim.filetype.add({ extension = { lcd = 'lucid' } })
vim.lsp.config('lucid', { cmd = { 'lucid', 'lsp' }, filetypes = { 'lucid' }, root_markers = { 'lucid.toml' } })
vim.lsp.enable('lucid')
```

## Diagnostics

Every diagnostic has this shape:

```json
{
  "code": "E0301",
  "severity": "error",
  "message": "name `totl` is not defined",
  "file": "src/cart.lcd",
  "line": 12,
  "column": 9,
  "end_line": 12,
  "end_column": 13,
  "notes": [],
  "fixes": [
    {
      "description": "rename to `total`",
      "edits": [{"line": 12, "column": 9, "end_line": 12, "end_column": 13, "text": "total"}]
    }
  ]
}
```

A code never changes meaning once released. The compiler reports every
error it can find in one run, not only the first, in order of position
in the source. `column` counts Unicode characters from 1, and
`end_column` is exclusive.

Where a likely fix exists, the message says what to do. A fix that is
nearly certain also goes in `fixes`, where `lucid fix` can make it, and
text output shows it as `  fix: description`. Fixes exist today for
E0101, E0106, and E0301 when there is one close spelling: at most one
edit per three characters, as in rustc, with no other name at the same
distance.

A parser error on a line that already has a lexer error is dropped,
since it is usually a side effect. Type checking runs only on a file with
no syntax errors. If a file uses a feature that is not built, `lucid run`
reports only E0901, because the missing feature could supply names and
types that other errors depend on; `lucid check` then reports nothing
beyond syntax errors, for the same reason.

Every code has a golden test in `tests/fail/` or a test in
`compiler/tests/golden.rs`. [errors.md](errors.md) lists each code with
its cause, an example, and a fix, and `lucid explain CODE` prints one
entry.

## The agent guide

Models have no Lucid in their training data, so the
[agent guide](agent-guide.md) and the diagnostics carry the whole load.
The guide is one compact file with everything that runs today, meant to
be pasted into an agent's context. It lists only what works, and
features move into it as they are built. A golden test checks that it
stays under its token budget and that all of its code runs.

## The benchmark

A trial of 10 tasks ran at M3, so that a problem with the syntax would
show up before the language froze. It passed all 10 on the first
attempt. Its tasks are in `benchmarks/agent-trial/`.

The full benchmark, in `benchmarks/agent-bench/`, is a fixed set of 50
tasks, each checked against the output of a Python reference; tasks 01
to 10 are the trial's. A Lucid agent gets only `agent-guide.md`. The
same tasks run in Python and TypeScript for comparison. A second Lucid
arm gets a guide with its warnings removed by `strip_guide.py`, which
shows which rules go against what models expect, and how far the
diagnostics alone can carry an agent. `score.py` counts first-attempt
compiles, first-attempt passes, and repair turns. `prompt.md` holds the
prompt and the arms, and decision 148 the method.

The results, and their limits, are in
[research.md](research.md#m5-agent-benchmark). A failure that repeats is
treated as a defect in the language or in the diagnostics.

Both the trial and the benchmark run as ordinary Claude Code sessions,
with no API key, split over several sessions to fit plan limits; see
[claude-code.md](claude-code.md).
