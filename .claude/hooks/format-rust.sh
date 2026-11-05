#!/bin/sh
# PostToolUse: format an edited Rust file. Never blocks the edit.
file=$(sed -n 's/.*"file_path": *"\([^"]*\)".*/\1/p' | head -n 1 | sed 's/\\\\/\//g')
case "$file" in
    *.rs) rustfmt --edition 2024 "$file" >/dev/null 2>&1 ;;
esac
exit 0
