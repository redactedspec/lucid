---
paths:
  - "compiler/src/lexer.rs"
  - "compiler/src/parser.rs"
  - "compiler/src/ast.rs"
---

- Parser methods are named after grammar rules. A change here that changes accepted syntax also changes `docs/grammar.md`.
- Never panic on any input. New recursion goes through `Parser::nested`.
- A new error needs a new code, a `tests/fail/` case, and a row in `docs/agents.md`. Codes never change meaning.
