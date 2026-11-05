#!/bin/sh
# Stop: block the end of a turn while tests fail. A turn already continued by
# this hook may end, so an unfixable failure cannot loop.
grep -q '"stop_hook_active": *true' && exit 0
cd "$CLAUDE_PROJECT_DIR" || exit 0
output=$(cargo test -q 2>&1) && exit 0
printf '%s\n' "$output" | tail -n 30 >&2
exit 2
