#!/bin/sh
# Self-check for the commit-msg hook.
hook=$(dirname "$0")/commit-msg
tmp=$(mktemp)
status=0

check() {
    expected=$1
    printf '%b' "$2" > "$tmp"
    sh "$hook" "$tmp" 2>/dev/null
    actual=$?
    if [ "$actual" -ne "$expected" ]; then
        echo "FAIL (exit $actual, wanted $expected): $2"
        status=1
    fi
}

long=$(printf 'x%.0s' $(seq 1 73))

check 0 'docs: add research log\n'
check 0 'feat(parser)!: drop old syntax\n\nBody text here.\n'
check 0 'docs: add guide\n# git comment\n\nBody.\n'
check 1 'Update docs\n'
check 1 'wip: add thing\n'
check 1 'docs: Add guide\n'
check 1 'docs: add guide.\n'
check 1 'docs(Parser): add guide\n'
check 1 'docs: this subject is much longer than the fifty character limit\n'
check 1 'docs: add guide\nno blank line\n'
check 1 "docs: add guide\n\n$long\n"
check 1 'docs: add guide\n\nCo-authored-by: A <a@b.c>\n'
check 1 'docs: add guide\n\nClaude-Session: abc\n'

rm -f "$tmp"
[ "$status" -eq 0 ] && echo "commit-msg checks passed"
exit "$status"
