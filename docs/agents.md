# Agent-Friendly Design

"Friendly to agents" must be checkable, or it is only a slogan. Each rule here is a test for proposed features.

## Language rules

1. **One way.** Each common job has one idiomatic form. Agents do not have to choose, and output stays consistent.
2. **Context-free parsing.** Any snippet parses without knowledge of other files.
3. **Local reasoning.** A task signature tells everything: inputs, mutation (`change`), ownership (`take`), and failure (`or` an error type).
4. **No hidden control flow.** No exceptions, macros, implicit conversions, or inheritance. Operator overloading is limited to arithmetic, equality, and ordering traits.
5. **Explicit at call sites.** `change` shows mutation. `try` shows a possible early exit. `spawn` shows concurrency.
6. **Familiar on purpose.** Prefer forms common in Python and TypeScript, so models have strong priors. New syntax needs a reason.
7. **Small.** The whole language must fit in one context window. See the token budget in `vision.md`.
8. **No formatting decisions.** `lucid fmt` has zero options. Diffs show only meaning.

## Tool rules

Every tool has a stable `--json` output.

| Command | Purpose for agents |
| --- | --- |
| `lucid check --json` | Fast diagnostics, no code generation |
| `lucid build --json` | A standalone executable, or the diagnostics that `lucid check --json` gives |
| `lucid build --target web` | `NAME.js`, which runs the program in a browser or under `node`. A user `extern` or `.pointer()` there is E0905. `lucid run` of a program with `std.dom` is E0906 |
| `lucid fix [--json] [PATH]` | Make the fix of each error that has one, in place, and list each as `file:line:column: fixed CODE: description`, or `code`, `description`, `file`, `line`, and `column` with `--json`. If two fixes touch the same text, only the first is made, and a second run makes the next. A lexer error stops checking, so its fixes come a run before those of the checker. `std` is never changed |
| `lucid explain [--json] CODE` | Cause, example, and fix for an error code, from `errors.md`. `--json` gives `code`, `title`, `cause`, `example`, and `fix` |
| `lucid test --json` | Per-test results of a file, or of every file in a package's `src`, with values on failure: `name`, `passed`, `file`, `line`, `column`, and `output`, which holds what the test printed and, on failure, the trap message and trace |
| `lucid fmt [--check] [PATH]` | Put a file, or every file of a package, in the one canonical layout, changing only space and line breaks. `--check` changes nothing, lists each file that would change, and exits with 1 if any would. A file with a lexer error gets its diagnostics and stays as it is |
| `lucid package [PATH]` | Check a library, write its archive `NAME-VERSION.lpk` to the current directory, and print its line of the registry's index, for upload to a registry |
| `lucid context [--json] MODULE` | Compact public API of a file, a package directory, or `std.NAME`: each public header with its `##` doc comments, and the fields, variants, or tasks of a type, choice, or trait, with no bodies. `--json` gives `module`, `line`, `kind`, `name`, `signature`, and `doc` |
| `lucid doc --json` | Full docs as data |
| `lucid lsp` | A language server on stdin and stdout, for an editor: diagnostics, formatting, go to definition, and hover with a name's type or declaration and `##` comment. Positions count UTF-16 units. See "Editors" below |
| `lucid --version [--json]` | `lucid 1.0.0`, or `{"version":"1.0.0"}` with `--json`, from the crate's version, by decision 227 |
| `lucid check --unsafe` | `unsafe` blocks and `extern` tasks: `file:line:column: unsafe: kind` on stdout, or `kind`, `file`, `line`, and `column` with `--json`. `kind` is `block` or `extern`. `std` stays out |
| `lucid check --costs` | Copy and reference count sites: `file:line:column: kind: message` on stdout, or `kind`, `type`, `file`, `line`, and `column` with `--json`. A `copy` is a value held twice that a later write changes, which costs O(n) if both holders stay. A `count` costs one count operation. Exit code 0. A program with an error or an unbuilt feature reports that instead, with exit code 1 |

## Editors

Point any editor with a language server client at the command `lucid lsp`, for files ending in `.lcd`. It syncs whole documents. No editor extension ships yet, by decision 202, so VS Code, which needs one, has no support. In Neovim 0.11 or later:

```lua
vim.filetype.add({ extension = { lcd = 'lucid' } })
vim.lsp.config('lucid', { cmd = { 'lucid', 'lsp' }, filetypes = { 'lucid' }, root_markers = { 'lucid.toml' } })
vim.lsp.enable('lucid')
```

## Diagnostics format

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

Rules:

- Codes never change meaning once released.
- The compiler reports all errors it can find in one run, not just the first.
- Each error says what to do next, where a likely fix exists. A fix that is near certain also goes in `fixes`, which `lucid fix` makes, and text output shows it as `  fix: description`. Fixes exist now for E0101, E0106, and E0301 with one close spelling: one edit away per three characters, as in rustc, with no other name at the same distance.
- Output order is deterministic: by source position.
- `column` counts Unicode characters, from 1. `end_column` is exclusive.

### Error codes

A parser error on a line that already has a lexer error is dropped, since it is usually a side effect.

Type checking runs only on a file with no syntax errors. If a file uses a feature that is not built, `lucid run` reports only E0901, because that feature could supply the names and types that other errors depend on. For the same reason, `lucid check` then reports nothing beyond syntax errors.

Every code has a golden test in `tests/fail/`, or a test in `compiler/tests/golden.rs`. [errors.md](errors.md) lists every code, with its cause, an example, and a fix, and `lucid explain CODE` prints one entry.

## Agent onboarding

The repository ships [agent-guide.md](agent-guide.md). It is one compact file with everything that runs today, for pasting into any agent context. It lists only what works, so features move into it as they are built. A golden test checks that it stays under the token budget and that all its code runs.

Models have no Lucid training data, so this guide and the diagnostics carry the whole load. Evidence is in [research.md](research.md).

## Agent benchmark

A 10-task trial ran at M3, so that a syntax problem would surface before the language freezes. It passed 10 of 10 on the first attempt, with limits stated in [research.md](research.md). Its tasks are in `benchmarks/agent-trial/`. The full benchmark is built in `benchmarks/agent-bench/`. It has run in full, with results in [research.md](research.md). It is a fixed set of 50 programming tasks, each checked against expected output from a Python reference. Tasks 01 to 10 are the trial's. Lucid agents get only `agent-guide.md`. The benchmark tracks first-attempt compile rate, first-attempt pass rate, and repair turns, with `score.py`. It compares the same tasks in Python and TypeScript. A second Lucid arm uses a guide with its warnings removed by `strip_guide.py`, to show which rules go against what models expect, and to test the diagnostics. `prompt.md` holds the subject prompt and the arms, and decision 148 the method. Treat a repeated failure as a language or diagnostics defect.

Both run as normal Claude Code sessions or subagents, with no API key, and are split over several sessions to fit plan limits. See `docs/claude-code.md`.
