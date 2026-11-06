# Lucid

A general-purpose, compiled programming language that reads like plain English, runs close to Rust and C++ speed, and is safe by default. It is designed to be written equally well by humans and AI agents.

```lucid
task greet(name: text) -> text
    return "Hello, {name}!"

let people = ["Ada", "Linus"]

for person in people
    print greet(person)
```

**Status:** early. The lexer and parser cover the full grammar. `lucid run` runs numbers of every size, booleans, text, lists, maps, sets, `type`, `choice`, `match`, unions with narrowing, generics, traits, `any` values, lambdas, nested tasks, task values, `change` and `take` parameters, control flow, and recursive tasks, with type checking, overflow traps, and reference counting. M3 is done, and M4 is next. Progress is in [docs/roadmap.md](docs/roadmap.md).

## Docs

Read in this order:

| Doc | Purpose |
| --- | --- |
| [vision.md](docs/vision.md) | Goals, non-goals, and how to resolve trade-offs |
| [language.md](docs/language.md) | The language reference (normative) |
| [grammar.md](docs/grammar.md) | Formal grammar (normative) |
| [memory-model.md](docs/memory-model.md) | Inferred ownership: the core technical bet |
| [security.md](docs/security.md) | Safety guarantees and package permissions |
| [agents.md](docs/agents.md) | What makes Lucid agent-friendly, as checkable rules |
| [agent-guide.md](docs/agent-guide.md) | Everything that runs today, in one file for agents |
| [stdlib-and-web.md](docs/stdlib-and-web.md) | Standard library, servers, and browser output |
| [compiler.md](docs/compiler.md) | Compiler architecture and toolchain |
| [roadmap.md](docs/roadmap.md) | Milestones with exit criteria |
| [decisions.md](docs/decisions.md) | Decision log and open questions |
| [research.md](docs/research.md) | Evidence behind each decision, and its gaps |
| [claude-code.md](docs/claude-code.md) | How Claude Code is used to build Lucid |

[examples/](examples/) holds target programs. They become the first compiler tests.
