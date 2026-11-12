# Lucid

A general-purpose, compiled programming language that reads like plain English, runs close to Rust and C++ speed, and is safe by default. It is designed to be written equally well by humans and AI agents.

```lucid
task greet(name: text) -> text
    return "Hello, {name}!"

let people = ["Ada", "Linus"]

for person in people
    print greet(person)
```

**Status:** early. The lexer and parser cover the full grammar. `lucid run` runs numbers of every size, booleans, text, lists, maps, sets, `type`, `choice`, `match`, unions with narrowing, generics, traits, `any` values, errors with `try` and `else`, `test` blocks with `expect`, foreign calls in `unsafe` blocks, modules, packages with path dependencies and permissions, the first `std` modules, lambdas, nested tasks, task values, `change` and `take` parameters, control flow, and recursive tasks, bit operators, and threads with `spawn`, `channel`, and `lock`, with type checking, overflow traps with stack traces, and reference counting. `lucid build` makes a standalone program on Windows and Linux, and `lucid build --target web` makes a `.js` file that runs the same programs in a browser or Node, with `std.dom` for interactive pages. `lucid fmt`, itself written in Lucid, gives code one layout. M6, the browser, is done: [examples/web](examples/web) is a to-do page written only in Lucid. M7, threads and servers, is done: bit operators, counts safe across threads, a stack limit per thread, `spawn`, channels and locks with a deadlock trap, TCP sockets in `std.net`, an HTTP/1.1 server and a client with TLS in `std.http`, and escaped pages from `std.html`. [examples/website.lcd](examples/website.lcd) serves a page and JSON to 64 clients at once in a load test. M8, the ecosystem, added text search and `trim` in the prelude, only reachable tasks compiled, which more than halves the size of a built server, Linux x86-64, where `run`, `test`, and `build` pass every test, `std.crypto`, with SHA-2, HMAC, random bytes, and Argon2id passwords, and `std.sql`, SQLite with a `sql` type that only text literals make, so a value goes in only through a `?` mark, the orphan rule, so a package adds tasks with `self` only to its own types, and a registry: `lucid package` publishes a library as static files, and `lucid run` fetches dependencies by minimal version selection and pins them in `lucid.lock`, through a client written in Lucid, and `lucid lsp`, a language server with diagnostics, formatting, go to definition, and hover. M8 is done, and the [version 1.0 promise](docs/vision.md#version-10-promise) holds from here. M9, a front end written in Lucid, is done: `LUCID_DUMP` prints each compiler stage as text, so each Lucid stage can be compared with the Rust one, and the lexer, parser, resolver, type checker, monomorphizer, and ownership pass in Lucid match the Rust ones on every file in the repository. The Lucid compiler hands its program to the Rust code generator through a small C ABI, so it runs `check`, `run`, and `build`, and passes the golden tests of those commands. It builds itself, and a second build from that output is byte-identical. Progress is in [docs/roadmap.md](docs/roadmap.md).

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
| [errors.md](docs/errors.md) | Every error code, with cause, example, and fix |
| [agent-guide.md](docs/agent-guide.md) | Everything that runs today, in one file for agents |
| [stdlib-and-web.md](docs/stdlib-and-web.md) | Standard library, servers, and browser output |
| [compiler.md](docs/compiler.md) | Compiler architecture and toolchain |
| [roadmap.md](docs/roadmap.md) | Milestones with exit criteria |
| [decisions.md](docs/decisions.md) | Decision log and open questions |
| [research.md](docs/research.md) | Evidence behind each decision, and its gaps |
| [claude-code.md](docs/claude-code.md) | How Claude Code is used to build Lucid |

[website/](website/) is the public site, lucidlang.dev: a static Astro site with the user docs. It is live at https://lucidlang.dev, deployed by Cloudflare Pages from `main`. See [website/README.md](website/README.md).

[examples/](examples/) holds target programs. They become the first compiler tests.

## License

Lucid is licensed under the GNU General Public License, version 3 or later: see [LICENSE](LICENSE). The runtime and standard library, which every built program contains, add the Classpath exception in [LICENSE-EXCEPTION](LICENSE-EXCEPTION), so a program built with Lucid may use any license.
