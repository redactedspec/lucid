# Lucid

A general-purpose, compiled programming language that reads like plain English, runs close to Rust and C++ speed, and is safe by default. It is designed to be written equally well by humans and AI agents.

```lucid
task greet(name: text) -> text
    return "Hello, {name}!"

let people = ["Ada", "Linus"]

for person in people
    print greet(person)
```

**Status:** design stage. No compiler exists yet. Next step: milestone M1 in [docs/roadmap.md](docs/roadmap.md).

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
| [stdlib-and-web.md](docs/stdlib-and-web.md) | Standard library, servers, and browser output |
| [compiler.md](docs/compiler.md) | Compiler architecture and toolchain |
| [roadmap.md](docs/roadmap.md) | Milestones with exit criteria |
| [decisions.md](docs/decisions.md) | Decision log and open questions |
| [research.md](docs/research.md) | Evidence behind each decision, and its gaps |

[examples/](examples/) holds target programs. They become the first compiler tests.
