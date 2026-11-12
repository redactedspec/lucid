#!/bin/sh
# Stop: block the end of a turn while tests fail. A turn already continued by
# this hook may end, so an unfixable failure cannot loop.
grep -q '"stop_hook_active": *true' && exit 0
cd "$CLAUDE_PROJECT_DIR" || exit 0
# Nothing that tests read has changed since the last commit, untracked files
# included, so the tests would give what they gave then.
code="compiler runtime std tests examples tools Cargo.toml Cargo.lock"
[ -z "$(git status --porcelain --untracked-files=all -- $code)" ] && exit 0
# stage_dumps_are_stable takes over a minute, so it runs by hand before a
# commit that touches compiler/lucid or the dumps. See docs/claude-code.md.
output=$(cargo test -q -- --skip stage_dumps_are_stable 2>&1) && exit 0
printf '%s\n' "$output" | tail -n 30 >&2
exit 2
