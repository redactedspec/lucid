# Standard Library and Web

## Rule

The standard library covers what most programs need, so that small projects need no dependencies. It is written in Lucid. Only low-level parts use `unsafe` and the runtime.

Modules arrive in stages. See [roadmap.md](roadmap.md). The sources are in `std/`, and the compiler holds a copy of them, so a program needs no files beside it. `std` alone can call the runtime, through `extern` tasks that start with `lucid_`. All of the M5 modules are built, and `std.dom`.

| Module | Contents | Milestone |
| --- | --- | --- |
| prelude | `text`, `list`, `map`, `set`, math, core traits, text split, integer parsing, case change, list sort, `channel`, `lock`, `spawned` | M3, M6, M7 |
| `std.fs` | Files and paths | M5 |
| `std.json` | Parse and write, typed through the `Json` trait | M5 |
| `std.env`, `std.args` | Environment and command-line input | M5 |
| `std.time` | Clocks, durations, dates | M5 |
| `std.dom` | Browser pages in the Elm architecture | M6 |
| `std.net` | TCP sockets | M7, built |
| `std.http` | HTTP/1.1 server, built, and client with TLS | M7 |
| `std.html` | Escaped HTML building | M7, built |
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

**Server side** compiles to a native binary. See [examples/website.lcd](../examples/website.lcd). `std.net`, `std.html`, and the `std.http` server are built, by decisions 177, 178, 180, 185, and 186:

- `std.net`: `net.listen(port)` gives a `Listener`, whose `accept()` gives a `Connection`, as `net.connect(host, port)` does. Port 0 takes a free port, and `listener.port` tells which. It listens on `127.0.0.1` unless given `host: "0.0.0.0"`. A connection has `read(max)`, `write(bytes)`, and `set_timeout(length)`, which takes a `time.Duration`. `read` gives an empty list once the other end closes. Each fails with a `NetError`: `Refused`, `TimedOut`, `InUse`, `Closed`, or `Failed(detail)`. A socket closes when no value holds it.

- `std.http`: `http.App()`, with `app.get(path, handler)` and `app.post(path, handler)`, routes by method and exact path. `app.serve(port)` serves HTTP/1.1 with keep-alive, one thread per connection and at most 256 at once, on `127.0.0.1` unless given `host: "0.0.0.0"`, and gives a `NetError` only if it cannot listen. `app.answer(connection)` serves one connection that a program accepted itself. A handler is a `task(http.Request) -> http.Response`. A `Request` has `method`, `path`, `query` (what follows the `?`), `headers` with names in lowercase, and `body` bytes, with `request.header(name)` and `request.body_text()`. A `Response` has `status`, `headers`, and `body`, and `http.page(node)`, `http.json(value)`, and `http.text(content)` build one with status 200. `HEAD` runs the `GET` handler. An unknown path gives 404, and a known path with another method gives 405. Limits and what is refused are in [security.md](security.md). A handler that traps ends the server, by decision 172.
- `std.html`: `html.Element(tag, attrs, children)`, `html.Text(content)`, `html.Attribute(name, value)`, `html.document(title, children)`, `html.render(node)`, which escapes every text and attribute value, and `html.escape(content)`. Tags that hold text take it: `h1`, `h2`, `h3`, `p`, `li`, `span`, `strong`, `em`, `code`, `pre`, and `a(href, content)`. Tags that hold elements take a list: `div`, `ul`, `ol`, `main`, `section`. A void element such as `br` renders with no end tag. `std.html` works on the web too.

Planned, by decision 179: `http.get(url)` and `http.post(url, body)` as the client, with TLS for `https`.

Under `--target web`, `std.net` and `std.http` are error E0907.

**Browser side** compiles to WebAssembly with `lucid build --target web`. The output is one JavaScript file, `NAME.js`, which holds a small loader and both WebAssembly modules, so a page needs only `<script src="NAME.js">`. Every built feature works, except a user `extern` and `.pointer()`, which are E0905. On the web, `std.fs` gives `Failed`, `env.get` gives none, `args.all()` gives `[]`, and `time.sleep` traps. Browser access goes through `std.dom`, because WebAssembly cannot touch the DOM directly. `std.dom` works only with `--target web`, and elsewhere it is E0906.

A page is built in the Elm architecture, by decisions 163 and 168:

- a model: any value that holds the page's state;
- `view(model)`: gives a `dom.Node[Msg]`, a tree where each handler gives a message;
- `update(model, message)`: gives the new model;
- `dom.app(init, view, update)`: shows `view(init)` in the page's `<body>`, and after each event runs `update`, then `view`, and changes only the nodes that differ. A node is kept when the new one has the same tag in the same place, so an `<input>` keeps its focus.

| Constructor | Is |
| --- | --- |
| `dom.Element(tag:, attrs: [], children: [])` | an element |
| `dom.Text(content:)` | text, never read as HTML |
| `dom.Attribute(name:, value:)` | an attribute. `value` also sets an `<input>`'s value, and `checked` whether it is checked, so a reused checkbox shows the model |
| `dom.On(event:, message:)` | a handler. `message` is a `task(text) -> Msg`, given the event's key for a key event, else the element's value, as an `<input>` has, else `""` |

Events run one at a time. An event that a change to the page starts, such as `blur` when a focused node goes, waits for the handler that runs now, and is dropped if its node has left the page. A handler cannot hold state itself, since a capture cannot change. A page traps on a `script` element, an attribute whose name starts with `on`, `formaction`, `srcdoc`, and a `javascript:` or `data:text/html` value, since each would run code that the program did not write. A keyed diff, for lists that reorder, waits for a program that needs it.

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
