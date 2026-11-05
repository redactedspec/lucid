# Working on Lucid

Instructions for any agent or human changing this repository.

## Sources of truth

- `docs/language.md` and `docs/grammar.md` define the language. If code and spec disagree, one of them has a bug: decide which, then fix it. Never leave them out of sync.
- `docs/decisions.md` records why. Check it before reopening a settled choice.
- `docs/roadmap.md` says what to build next. Work on the current milestone only.

## Rules

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
