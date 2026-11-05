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
| `lucid fix` | Apply machine-suggested fixes |
| `lucid explain E0123` | Cause, example, and fix for an error code |
| `lucid test --json` | Per-test results with values on failure |
| `lucid context <module>` | Compact public API of a module or package: signatures and doc comments only |
| `lucid doc --json` | Full docs as data |
| `lucid check --costs` | Copy and reference count sites |

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
  "notes": ["a variable named `total` exists in this scope"],
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
- Each error says what to do next, where a likely fix exists.
- Output order is deterministic: by source position.
- `column` counts Unicode characters, from 1. `end_column` is exclusive.

### Error codes

A parser error on a line that already has a lexer error is dropped, since it is usually a side effect.

Type checking runs only on a file with no syntax errors. If a file uses a feature that is not built, `lucid run` reports only E0901, because that feature could supply the names and types that other errors depend on. For the same reason, `lucid check` then reports nothing beyond syntax errors.

The ranges are: `E00xx` files, `E01xx` lexer, `E02xx` parser, `E03xx` names and types, `E09xx` compiler limits and features not built yet. Every code has a golden test in `tests/fail/`, or a test in `compiler/tests/golden.rs`.

| Code | Meaning |
| --- | --- |
| E0001 | Cannot read the file |
| E0002 | File is not valid UTF-8 |
| E0101 | Tab in indentation |
| E0102 | Indentation width is wrong |
| E0103 | Unexpected character |
| E0104 | Text is not closed |
| E0105 | Unknown escape |
| E0106 | Single `}` in text |
| E0107 | Malformed number |
| E0108 | Text after an opening `"""` |
| E0201 | Unexpected token |
| E0202 | Invalid assignment target |
| E0203 | `match` used as a value has a block arm |
| E0204 | Chained comparison or range |
| E0205 | Nesting over 128 levels, or over 1024 operators in one expression |
| E0206 | Declaration only allowed at top level |
| E0301 | Name is not defined |
| E0302 | Type is not defined |
| E0303 | Types do not fit |
| E0304 | Name is already defined in this scope |
| E0305 | Name cannot change |
| E0306 | Task uses a top-level `var` |
| E0307 | Arguments or parameters are in the wrong form |
| E0308 | Called value is not a task |
| E0309 | Task may end without returning |
| E0310 | `return` outside a task |
| E0311 | `break` or `continue` outside a loop |
| E0312 | Number does not fit its type |
| E0313 | Default is not a literal |
| E0314 | Empty list or map has no type to take |
| E0315 | Alias is defined by itself, or a type holds itself with no way to end |
| E0316 | `match` does not cover every case |
| E0317 | `match` arm can never run |
| E0318 | Pattern does not fit its value: unknown variant, wrong number of names, a bound type or variant name, `{}` in text, or a type that the value never holds |
| E0901 | Parses, but cannot run at this milestone |
| E0902 | Task frame is too large for the stack check: over 512 KiB, or half the stack for top-level code |

## Agent onboarding

The repository ships `docs/agent-guide.md`, drafted at M3. It is one compact file with the full language, for pasting into any agent context. CI checks that it stays under the token budget and that all its code compiles.

Models have no Lucid training data, so this guide and the diagnostics carry the whole load. Evidence is in [research.md](research.md).

## Agent benchmark

A 10-task trial runs at M3, so that a syntax problem surfaces before the language freezes. The full benchmark arrives at M5. It is a fixed set of 50 programming tasks, each with tests. Agents get only `agent-guide.md`. The benchmark tracks first-attempt compile rate, first-attempt test pass rate, and repair turns. Compare against the same tasks in Python and TypeScript. Treat a repeated failure as a language or diagnostics defect.

Both run as normal Claude Code sessions, with no API key, and are split over several sessions to fit plan limits. See `docs/claude-code.md`.
