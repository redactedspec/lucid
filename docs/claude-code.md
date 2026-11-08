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
| After a large diff | `ponytail-review`, to find what to delete |
| Small review, to save usage | `caveman-review` |
| Broad search in unfamiliar code | `caveman-explore` |
| End of a milestone | `ponytail-audit`, `ponytail-debt`, `caveman-stats` |
| Question needing many sources | Own searches first. `deep-research` starts subagents, so it needs owner approval |

`code-review` at `max` level runs only on request.

## Plan limits

The owner has the Claude Pro plan and no API credit. All work draws on Pro limits.

- **No API-billed tooling.** Nothing in Lucid may need an API key, including the agent trial and benchmark. Run those as normal Claude Code sessions.
- **`/code-review ultra`:** 3 free runs per account, one time only. They do not refresh. A stopped or failed run still counts. After that it needs usage credits, typically $5 to $25 per review. Save the free runs for the ownership pass at M4, the runtime and permissions at M5, and the 1.0 review.
- **Local `/code-review`:** counts toward normal usage, so it is the default review. Use `low` or `medium` on small changes.
- **Fable and 1M context:** may need usage credits. Do not select them without asking.
- **Subagents:** use few. Each one spends plan usage and starts with no context. Prefer direct tools for small jobs.
- **Workflows and agent teams:** not used. Their limits on Pro are not documented in the pages read.
- **Limit reached:** commit finished work, then stop and report. Do not switch to anything that bills credits.

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
| `/code-review ultra` | Deep multi-agent cloud review. Only 3 free runs ever. See "Plan limits" |
| Shift+Tab | Plan mode, for changes to the language or architecture |

## Working rules

- **Decide, then report.** Claude settles design and workflow choices itself, and does not ask the owner to pick. Spending limits under "Plan limits" and owner approval for `deep-research` still hold.
- **Pass-or-fail checks.** Every task ends with a command that passes or fails. Report the command and its output, not a claim.
- **Two corrections, then reset.** After two failed corrections, ask the owner to run `/clear`, and restart with a better prompt.
- **One session per task.** Unrelated tasks raise the cost of each later turn.
- **Spec, then build.** Design in one session, and build in a fresh one from the written spec.
- **Subagents only where they pay.** Use them for parallel independent work, long output such as full test runs, or a reviewer with no prior context. They do not fix a full context.
- **Enforce what must always happen.** Use a hook or deny rule. `CLAUDE.md` holds guidelines, and adding emphasis does not make them stick. Keep the enforced set small.
- **Keep `CLAUDE.md` and imports short.** Target under 200 lines. Imports still load every session.

## What is enforced now

`.githooks/commit-msg` checks every commit for the rules in `AGENTS.md`. It rejects bad types, long subjects, long body lines, and attribution lines. It also rejects a commit that changes `.rs` files under `compiler/` or `runtime/` with no `.md` file, unless the message has a `Docs: none` footer. After a fresh clone, turn it on:

```sh
git config core.hooksPath .githooks
```

Self-check: `sh .githooks/test-commit-msg.sh`.

## Project setup

Hooks load when a session starts, so a change to them applies from the next session.

| Feature | What it does |
| --- | --- |
| `Stop` hook | `.claude/hooks/test-on-stop.sh` runs `cargo test -q`, and blocks the end of a turn on failure, once per stop |
| `PostToolUse` hook | `.claude/hooks/format-rust.sh` runs `rustfmt` on edited Rust files |
| Rust code intelligence | `rust-analyzer` shows errors after edits. It comes from the owner's setup, not this repository |
| `.claude/rules/front-end.md` | Rules for the lexer and parser, loaded only for those files |
| `.claude/agents/spec-reviewer.md` | Read-only reviewer that compares code with `docs/language.md` and `docs/grammar.md` |
| `permissions.allow` | Allows `cargo` commands |

## Not useful for Lucid now

Skipped on purpose: agent teams (about 7x the tokens in plan mode), cloud sessions and GitHub features (the repository is a private GitHub repository, used only for push), MCP servers, and scheduled tasks. One exception is planned for M8: a GitHub Actions job on macOS, since no Mac is at hand, under a $0 spending limit, by decision 192. Reconsider if a real need appears.

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
- [Ultrareview](https://code.claude.com/docs/en/ultrareview.md)
- [Feature availability](https://code.claude.com/docs/en/feature-availability.md)
- [Model configuration](https://code.claude.com/docs/en/model-config.md)
- [Full index](https://code.claude.com/docs/llms.txt)
