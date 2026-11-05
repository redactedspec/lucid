# Standard Library and Web

## Rule

The standard library covers what most programs need, so that small projects need no dependencies. It is written in Lucid. Only low-level parts use `unsafe` and the runtime.

Modules arrive in stages. See [roadmap.md](roadmap.md).

| Module | Contents | Milestone |
| --- | --- | --- |
| prelude | `text`, `list`, `map`, `set`, math, core traits | M3 |
| `std.fs` | Files and paths | M5 |
| `std.json` | Parse and write, typed through the `Json` trait | M5 |
| `std.env`, `std.args` | Environment and command-line input | M5 |
| `std.time` | Clocks, durations, dates | M5 |
| `std.dom` | Browser access through JavaScript calls | M6 |
| `std.task` | `channel`, `lock` | M7 |
| `std.net`, `std.http` | Sockets, HTTP server and client, TLS | M7 |
| `std.html` | Escaped HTML building | M7 |
| `std.sql` | SQLite first, parameterized queries only | M8 |
| `std.crypto` | Hashes, random, and vetted primitives | M8 |

Never hand-roll cryptography primitives. Wrap a vetted implementation.

## Websites

Lucid covers both sides of a website.

**Server side** compiles to a native binary. See [examples/website.lcd](../examples/website.lcd).

**Browser side** compiles to WebAssembly with `lucid build --target web`. The output is a `.wasm` file with a small JavaScript loader. Browser access goes through `std.dom`, because WebAssembly cannot touch the DOM directly.

The browser framework is deliberately not designed yet. Decide it at M6, based on real programs.

## Example: JSON

```lucid
use std.json

type Book
    title: text
    pages: int

let raw = """{{"title": "Dune", "pages": 412}}"""
let book = try json.parse[Book](raw)
print book.title
```

`Json` is derived by the compiler. No annotations or generators are needed.
