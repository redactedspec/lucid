# Claude Code Workflow

How Claude Code is used to build Lucid. Based on the official docs, reviewed 2026-11-05. Sources are at the end.

## Skills Claude runs without being asked

| When | Skill |
| --- | --- |
| Cause of a failure is unclear | `investigate-first` |
| Bug fix | `surgical-patch` |
| Restructuring code | `safe-refactor` |
| New feature with overbuilding risk | `lean-build` |
| Before calling work done | `verify-and-stop` |
| After a non-trivial change | `code-review` (up to `xhigh`), `simplify` |
| Runtime, `unsafe`, or permission code | `security-review` |
| End of a milestone | `ponytail-audit`, `ponytail-debt` |
| Question needing many sources | `deep-research` (starts subagents, so big questions only) |

`code-review` at `max` level runs only on request.

## Commands only the owner can run

Claude cannot run built-in commands. It asks when one is needed.

| Command | Use |
| --- | --- |
| `/clear` | Between unrelated tasks. Costs nothing, and stops old context being paid for |
| `/rename`, `/resume` | Name a session before leaving it, and return to it later |
| `/compact <focus>` | Shrink a long session. This is itself a large request |
| `/rewind` or Esc twice | Roll back code or conversation. Does not cover Bash edits, so git stays the real safety net |
| `/usage`, `/context` | See what is using tokens |
| `/effort`, `/model` | Change effort or model |
| `/goal <condition>` | Keep working until a milestone exit criterion is met |
| `/code-review ultra` | Deep multi-agent cloud review, billed. Run before each milestone closes |
| Shift+Tab | Plan mode, for changes to the language or architecture |

## Working rules

- **Pass-or-fail checks.** Every task ends with a command that passes or fails. Report the command and its output, not a claim.
- **Two corrections, then reset.** After two failed corrections, ask the owner to run `/clear`, and restart with a better prompt.
- **One session per task.** Unrelated tasks raise the cost of each later turn.
- **Spec, then build.** Design in one session, and build in a fresh one from the written spec.
- **Subagents only where they pay.** Use them for parallel independent work, long output such as full test runs, or a reviewer with no prior context. They do not fix a full context.
- **Enforce what must always happen.** Use a hook or deny rule. `CLAUDE.md` holds guidelines, and adding emphasis does not make them stick. Keep the enforced set small.
- **Keep `CLAUDE.md` and imports short.** Target under 200 lines. Imports still load every session.

## What is enforced now

`.githooks/commit-msg` checks every commit for the rules in `AGENTS.md`. It rejects bad types, long subjects, long body lines, and attribution lines. After a fresh clone, turn it on:

```sh
git config core.hooksPath .githooks
```

Self-check: `sh .githooks/test-commit-msg.sh`.

## Planned setup

These wait until the code they act on exists. They are tracked in `docs/roadmap.md` under M1.

| Feature | Plan |
| --- | --- |
| `Stop` hook | Run `cargo test -q`, and block the end of a turn on failure |
| `PostToolUse` hook | Run `rustfmt` on edited Rust files |
| Rust code intelligence plugin | Show type errors after edits, without a compile |
| `.claude/rules/` | Stage-specific rules, loaded only for matching paths |
| `.claude/agents/spec-reviewer.md` | Read-only reviewer that compares code with `docs/language.md` and `docs/grammar.md` |
| `permissions.allow` | Allow `cargo` commands |
| Doc check | Reject commits that change source with no doc change, unless marked as needing none |

## Not useful for Lucid now

Skipped on purpose: agent teams (about 7x the tokens in plan mode), cloud sessions and GitHub features (Lucid has no remote), MCP servers, and scheduled tasks. Reconsider if a real need appears.

## Sources

Read in part. Long pages were partly summarized by the fetch tool, so check the page before relying on an exact limit.

- [Best practices](https://code.claude.com/docs/en/best-practices)
- [Memory](https://code.claude.com/docs/en/memory)
- [Hooks guide](https://code.claude.com/docs/en/hooks-guide)
- [Skills](https://code.claude.com/docs/en/skills)
- [Subagents](https://code.claude.com/docs/en/sub-agents)
- [Common workflows](https://code.claude.com/docs/en/common-workflows)
- [Costs](https://code.claude.com/docs/en/costs)
- [Settings](https://code.claude.com/docs/en/settings)
- [Full index](https://code.claude.com/docs/llms.txt)
