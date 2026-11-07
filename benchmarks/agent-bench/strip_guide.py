# Usage: python strip_guide.py GUIDE OUT. Writes the guide for the bare arm:
# docs/agent-guide.md without the warnings that say what Lucid does not do,
# so the arm shows which habits agents bring and how the errors guide them.
# Fails if a warning is no longer in the guide, so this list cannot go stale.
import re
import sys

WARNINGS = [
    " Lucid has no training data: do not assume a feature from Python, TypeScript, or Rust unless it is shown here.",
    "- There are no colons after `if` or `task`, no semicolons, and no braces around blocks.\n",
    " There is no `**`, `&`, `|`, `^`, `<<`, or `>>`. A chain such as `a < b < c` is an error.",
    " An integer literal never becomes a `float`: write `2.0`.",
    " Interpolation is the only way to join text. `+` does not work on text: write `\"{a}{b}\"`.",
    "- There is no `x[0]` on text, and no character type. Write `x[0..1]`.\n",
    " There is no negative index.",
    " Write anything else yourself.",
    "- `.pop()` must be a whole statement, or all of the value of `let`, `var`, `return`, or `name =`. `f(x.pop())` is an error.\n",
    "- A `for` body cannot change what it visits. To remove or add while looping, loop over a copy: `let copy = items`, then `for x in copy`.\n",
    " There is no truthiness: write `items is empty` or `x != 0`.",
    "A task cannot read or write a top-level `var`: pass it. ",
    " Capturing a `var`, a `change` or `take` parameter, or a `for change` name is an error: write `let copy = count` first. A captured name cannot change, so a closure cannot keep a running count.",
    " `f(x)` is an error for such a task, and `x.f()` is an error for any other task.",
    "- Patterns do not nest, and there are no guards.\n",
    "- A `match` on a union names no variant. Match `e: ParseError`, then `match e` inside that arm.\n",
    "- Only a local name or parameter narrows, never a field or call. Write `let b = a.b`, test `b`, and use `b`.\n",
    ", and `[1, none]` alone is an error. A `list[int]` is not a `list[int?]`",
]
SECTION = re.compile(r"## What is not available\n.*?(?=\n## )", re.S)

guide = open(sys.argv[1], encoding="utf8").read()
for w in WARNINGS:
    if guide.count(w) != 1:
        sys.exit(f"warning not found once in the guide: {w!r}")
    guide = guide.replace(w, "")
guide, found = SECTION.subn("", guide)
if found != 1:
    sys.exit("section 'What is not available' not found")
open(sys.argv[2], "w", encoding="utf8", newline="\n").write(guide)
