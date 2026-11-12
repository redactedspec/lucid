#!/bin/sh
# Stop: block the end of a turn while tests fail. A turn already continued by
# this hook may end, so an unfixable failure cannot loop.
grep -q '"stop_hook_active": *true' && exit 0
cd "$CLAUDE_PROJECT_DIR" || exit 0
# Skip only when nothing that tests read differs from the last commit that
# passed here, untracked files included. A failed git call, a missing stamp,
# or a commit since the stamp all run the tests.
code="backend compiler runtime std tests examples tools docs Cargo.toml Cargo.lock"
stamp=$(git rev-parse --git-path lucid-tested 2>/dev/null) || stamp=
if [ -n "$stamp" ] && changes=$(git status --porcelain --untracked-files=all -- $code) &&
    [ -z "$changes" ] && tested=$(cat "$stamp" 2>/dev/null) &&
    git diff --quiet "$tested" HEAD -- $code 2>/dev/null; then
    exit 0
fi
# stage_dumps_are_stable takes over a minute, so it runs by hand before a
# commit that touches compiler/lucid or the dumps. See docs/claude-code.md.
if output=$(cargo test -q -- --skip stage_dumps_are_stable 2>&1); then
    # Stamp only a clean tree, so the stamp names exactly what passed.
    [ -n "$stamp" ] && [ -z "$(git status --porcelain --untracked-files=all -- $code)" ] &&
        git rev-parse HEAD >"$stamp" 2>/dev/null
    exit 0
fi
printf '%s\n' "$output" | tail -n 30 >&2
exit 2
