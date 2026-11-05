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
  "code": "E0204",
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
- Output order is deterministic.

## Agent onboarding

The repository ships `docs/agent-guide.md`, drafted at M3. It is one compact file with the full language, for pasting into any agent context. CI checks that it stays under the token budget and that all its code compiles.

Models have no Lucid training data, so this guide and the diagnostics carry the whole load. Evidence is in [research.md](research.md).

## Agent benchmark

A 10-task trial runs at M3, so that a syntax problem surfaces before the language freezes. The full benchmark arrives at M5. It is a fixed set of 50 programming tasks, each with tests. Agents get only `agent-guide.md`. The benchmark tracks first-attempt compile rate, first-attempt test pass rate, and repair turns. Compare against the same tasks in Python and TypeScript. Treat a repeated failure as a language or diagnostics defect.
