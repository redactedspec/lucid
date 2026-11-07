# Standard Library and Web

## Rule

The standard library covers what most programs need, so that small projects need no dependencies. It is written in Lucid. Only low-level parts use `unsafe` and the runtime.

Modules arrive in stages. See [roadmap.md](roadmap.md). The sources are in `std/`, and the compiler holds a copy of them, so a program needs no files beside it. `std` alone can call the runtime, through `extern` tasks that start with `lucid_`. All of the M5 modules are built.

| Module | Contents | Milestone |
| --- | --- | --- |
| prelude | `text`, `list`, `map`, `set`, math, core traits. Planned: text split, integer parsing, case change, list sort | M3, M6 |
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

## Built modules

| Task | Gives |
| --- | --- |
| `args.all()` | `list[text]`: what follows the program in its command |
| `args.get(index)` | `text?`: one of them |
| `env.get(name)` | `text?`: a variable, if set |

| `fs.read_text(path)` | `text or FileError` |
| `fs.write_text(path, content)` | `none or FileError`: replaces the file, or makes it |
| `fs.append_text(path, content)` | `none or FileError`: adds to its end, or makes it |
| `fs.read_bytes(path)` | `list[byte] or FileError` |
| `fs.write_bytes(path, data)` | `none or FileError` |
| `fs.exists(path)` | `bool` |
| `fs.list_dir(path)` | `list[text] or FileError`: sorted names |
| `fs.make_dir(path)` | `none or FileError`: with any parents it needs |
| `fs.remove(path)` | `none or FileError`: a file or an empty directory |
| `fs.rename(from, to)` | `none or FileError`: replaces what is at `to` |

`FileError` is a `choice`: `NotFound(path)`, `Denied(path)`, `NotText(path)`, or `Failed(path, detail)`. Paths are text, and `/` works on every system, so `"{dir}/{name}"` joins them.

In a package, `std.fs` needs the `files` permission, and `std.env` needs `env`. A path outside the listed paths gives `Failed`, and `exists` gives `false`. `env.get` of a name that is not listed traps. See "Package permissions" in [security.md](security.md).

| `time.now()` | `Time`: the wall clock, which can jump |
| `time.clock()` | `Instant`: a clock that only goes forward, for measuring |
| `later.since(earlier)` | `Duration`, for two `Time` or two `Instant` values |
| `time.seconds(n)`, `time.millis(n)` | `Duration` |
| `d.to_seconds()`, `d.to_millis()` | `float`, `int` |
| `time.sleep(length)` | waits. A negative length does not wait |
| `t.date()`, `date.time()` | `Date` and `Time`, each from the other, in UTC |

A `Duration` has `+`, `-`, and an order. A `Time` holds nanoseconds from 1970, in the `nanos` field, so it reaches the years 1678 to 2262. A `Date` has `year`, `month`, `day`, `hour`, `minute`, and `second`, where the last three default to 0, and it shows in ISO 8601. There are no time zones: local time needs zone tables. Parsing dates waits for a program that needs it.

Text from the system that is not valid Unicode has its bad parts replaced, except that `fs.read_text` gives `NotText`.

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

let raw = """
    {{"title": "Dune", "pages": 412}}
    """
let book = try json.parse[Book](raw)
print book.title
```

`Json` is derived by the compiler. No annotations or generators are needed. How each type maps to JSON is under "Traits" in [language.md](language.md).

`json.parse[T](raw)` gives `T or JsonError`, and `json.write(value)` gives `text`, with no spaces. A `JsonError` has `message` and `path`, as in `.books[2].title`. Input nested over 128 levels is an error, so it cannot overflow the stack. A `u64` reads only up to the highest `int`. A tree form for JSON of unknown shape waits for a program that needs it.
