# Working on Lucid

Instructions for any agent or human changing this repository.

## Sources of truth

- `docs/language.md` and `docs/grammar.md` define the language. If code and spec disagree, one of them has a bug: decide which, then fix it. Never leave them out of sync.
- `docs/decisions.md` records why. Check it before reopening a settled choice.
- `docs/roadmap.md` says what to build next. Work on the current milestone only.

## Documentation

Docs must never go stale. Every commit that changes behavior, code, plans, or decisions updates all affected docs in that same commit. Before each commit, check every file below against the change. A task is not finished while any doc is out of date.

| Change | Update |
| --- | --- |
| Syntax or semantics | `docs/language.md`, `docs/grammar.md`, `examples/` |
| Memory or ownership behavior | `docs/memory-model.md` |
| Safety guarantees or permissions | `docs/security.md` |
| Diagnostics, tools, or agent workflow | `docs/agents.md` |
| Standard library or web support | `docs/stdlib-and-web.md` |
| Compiler structure, commands, dependencies, or toolchain | `docs/compiler.md`, this file |
| Goals, targets, or trade-offs | `docs/vision.md` |
| Milestone progress or scope | `docs/roadmap.md` |
| A decision made or reversed | `docs/decisions.md` |
| New evidence or benchmark results | `docs/research.md` |
| Agent tooling, hooks, or Claude Code workflow | `docs/claude-code.md` |
| Project status, or a doc added or removed | `README.md` |

Docs describe what is true now. Mark planned items as planned, and remove claims that stop being true.

## Rules

- Claude builds Lucid. The owner directs and decides.
- Back every design decision with research. Cite primary sources in `docs/research.md`, and link the decision entry to them. If evidence is weak or missing, say so, and test it with a benchmark or experiment.
- Language change: update spec, grammar, examples, and tests in the same change. Add a decision entry when the change is not obvious.
- Before adding a feature, apply the priority order in `docs/vision.md` and the rules in `docs/agents.md`. A feature that adds a second way to do something needs a strong reason.
- Hand-write the lexer, parser, type checker, and ownership pass. Allowed compiler dependencies are listed in `docs/compiler.md`. Do not add others without a decision entry.
- Every compile error needs a stable code and a test. Every language feature needs a passing golden test.
- Any safety hole in user programs is a release blocker. Never trade away a guarantee in `docs/security.md` for speed.

## Commands

No compiler exists until M1. The planned commands are listed in `docs/compiler.md`.

## Commits

- Commit every finished change locally. No remote: never add one or push.
- No `Co-authored-by` or other attribution lines.
- Strict Conventional Commits. Allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`.
- Subject at most 50 characters, imperative mood, lowercase start, no trailing period.
- Body wrapped at 72, and explains what and why.
- A change that fits two types is two commits.
- `.githooks/commit-msg` enforces these rules. Never bypass it. Turn it on with `git config core.hooksPath .githooks`.
