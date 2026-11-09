# Standard Library and Web

## Rule

The standard library covers what most programs need, so that small projects need no dependencies. It is written in Lucid. Only low-level parts use `unsafe` and the runtime.

Modules arrive in stages. See [roadmap.md](roadmap.md). The sources are in `std/`, and the compiler holds a copy of them, so a program needs no files beside it. `std` alone can call the runtime, through `extern` tasks that start with `lucid_`. All of the M5 modules are built, and `std.dom`, `std.net`, `std.http`, `std.html`, `std.crypto`, and `std.sql`.

| Module | Contents | Milestone |
| --- | --- | --- |
| prelude | `text`, `list`, `map`, `set`, math, core traits, text split, integer parsing, case change, text search and trim, list sort, `channel`, `lock`, `spawned` | M3, M6, M7, M8 |
| `std.fs` | Files and paths | M5 |
| `std.json` | Parse and write, typed through the `Json` trait | M5 |
| `std.env`, `std.args` | Environment and command-line input | M5 |
| `std.time` | Clocks, durations, dates | M5 |
| `std.dom` | Browser pages in the Elm architecture | M6 |
| `std.net` | TCP sockets | M7, built |
| `std.http` | HTTP/1.1 server, and client with TLS | M7, built |
| `std.html` | Escaped HTML building | M7, built |
| `std.sql` | SQLite, with values only through `?` marks and queries of type `sql`, by decisions 194, 195, and 209 | M8, built |
| `std.crypto` | SHA-256, SHA-512, HMAC, constant-time compare, random bytes, Argon2id passwords, hex, by decisions 193 and 208 | M8, built |

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
| `fs.real_path(path)` | `text or FileError`: the absolute path, with no `.`, `..`, or links |
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

| `crypto.sha256(data)`, `crypto.sha512(data)` | `list[byte]`: the digest, 32 or 64 bytes |
| --- | --- |
| `crypto.hmac_sha256(key, data)` | `list[byte]`: the 32-byte tag |
| `crypto.hmac_check(key, data, tag)` | `bool`, in constant time |
| `crypto.same(a, b)` | `bool`: the same bytes, in a time that depends only on the lengths |
| `crypto.random_bytes(count)` | `list[byte]` from the system's cryptographic source. A negative `count` traps |
| `crypto.hash_password(password)` | `text`: an Argon2id PHC string, with a new random salt |
| `crypto.check_password(password, hash)` | `bool`. A `hash` that is not an Argon2 PHC string gives false |
| `crypto.hex(data)` | `text`: lowercase hex |

`std.crypto` wraps `ring`, `subtle`, and `argon2` in the runtime, by decisions 193 and 208. Passwords use m=19456, t=2, p=1, OWASP's first setting. The check reads the setting from the hash, so a later rise still checks old hashes. Text goes in through `.bytes()`. It needs no permission, and under `--target web` it is E0907.

| `sql.open(path)` | `sql.Db or sql.SqlError`. `":memory:"` gives a new empty database in memory |
| --- | --- |
| `db.run(query, values)` | `int or SqlError`: how many rows one statement changed, with those that triggers and foreign keys changed |
| `db.query(query, values)` | `list[sql.Row] or SqlError`: each `Row` has `columns`, a `list[text]`, and `values`, a `list[sql.Value]` in the same order |

`std.sql` is SQLite 3.54.1, built into the runtime from `libsqlite3-sys`, by decisions 194, 195, and 209. A `query` has the type `sql`, which only a text literal with no `{...}`, and `sql + sql`, make. So a value goes in only through a `?` mark: `values` holds one `sql.Value` for each mark, in order, and a different count traps. A `Value` is `Null`, `Int(value)`, `Float(value)`, `Text(value)`, or `Bytes(value)`, and a row gives back the same kinds. A query holds exactly one statement, which a comment may follow. A connection waits up to 5 seconds for another connection's lock. A `SqlError` is `Constraint(detail)`, `Busy(detail)`, or `Failed(detail)`, and `error_message()` gives the detail. A `Db` closes when no value holds it, and may go to another thread. `ATTACH` and `VACUUM INTO` a file fail, since only the `files` permission says which files open. `sql.open` needs the `files` permission for its path, and under `--target web` it is E0907.

## Websites

Lucid covers both sides of a website.

**Server side** compiles to a native binary. See [examples/website.lcd](../examples/website.lcd). `std.net`, `std.html`, and `std.http` are built, by decisions 177 to 180 and 185 to 187:

- `std.net`: `net.listen(port)` gives a `Listener`, whose `accept()` gives a `Connection`, as `net.connect(host, port)` does. Port 0 takes a free port, and `listener.port` tells which. It listens on `127.0.0.1` unless given `host: "0.0.0.0"`. A connection has `read(max)`, `write(bytes)`, and `set_timeout(length)`, which takes a `time.Duration`. `read` gives an empty list once the other end closes. Each fails with a `NetError`: `Refused`, `TimedOut`, `InUse`, `Closed`, or `Failed(detail)`. A socket closes when no value holds it. `connection.secure(host)` starts TLS as a client, and gives the connection to use from then on.

- `std.http`: `http.App()`, with `app.get(path, handler)` and `app.post(path, handler)`, routes by method and exact path. `app.serve(port)` serves HTTP/1.1 with keep-alive, one thread per connection and at most 256 at once, on `127.0.0.1` unless given `host: "0.0.0.0"`, and gives a `NetError` only if it cannot listen. `app.answer(connection)` serves one connection that a program accepted itself. A handler is a `task(http.Request) -> http.Response`. A `Request` has `method`, `path`, `query` (what follows the `?`), `headers` with names in lowercase, and `body` bytes, with `request.header(name)` and `request.body_text()`. A `Response` has `status`, `headers`, and `body`, and `http.page(node)`, `http.json(value)`, and `http.text(content)` build one with status 200. `HEAD` runs the `GET` handler. An unknown path gives 404, and a known path with another method gives 405. Limits and what is refused are in [security.md](security.md). After a refusal the server closes in stages, by decision 207, so the client can read the answer. A handler that traps ends the server, by decision 172.
- `std.html`: `html.Element(tag, attrs, children)`, `html.Text(content)`, `html.Attribute(name, value)`, `html.document(title, children)`, `html.render(node)`, which escapes every text and attribute value, and `html.escape(content)`. Tags that hold text take it: `h1`, `h2`, `h3`, `p`, `li`, `span`, `strong`, `em`, `code`, `pre`, and `a(href, content)`. Tags that hold elements take a list: `div`, `ul`, `ol`, `main`, `section`. A void element such as `br` renders with no end tag. `std.html` works on the web too.

- `std.http` client: `http.get(url)` and `http.post(url, kind, body)`, where `kind` is the media type and `body` is bytes, give a `Response or NetError`, with header names in lowercase, and `response.header(name)` and `response.body_text()`. An `https` URL uses TLS, and the certificate must be for the host and come from a root that Mozilla trusts, with no way to turn the check off. Each request uses its own connection. A redirect is not followed. Limits are in [security.md](security.md).

Under `--target web`, `std.net`, `std.http`, `std.crypto`, and `std.sql` are error E0907.

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
