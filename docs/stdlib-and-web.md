# The Standard Library and the Web

The standard library covers what most programs need, so a small project
needs no dependencies. It is written in Lucid, under `std/`, and the
compiler carries a copy, so a program needs no files beside it. Only
`std` may call the runtime, through `extern` tasks whose names start with
`lucid_`, and only its low-level parts use `unsafe`.

Cryptography is never written by hand. `std.crypto` wraps vetted Rust
libraries.

| Module | Contents |
| --- | --- |
| prelude | `text`, `list`, `map`, `set`, math, the core traits, splitting, searching, trimming and case change of text, integer parsing, sorting, `channel`, `lock`, `spawned` |
| `std.args` | Command-line arguments |
| `std.console` | Lines typed at the terminal, and text written with no newline |
| `std.env` | Environment variables |
| `std.fs` | Files and paths |
| `std.json` | JSON, typed through the `Json` trait |
| `std.time` | Clocks, durations, dates |
| `std.crypto` | SHA-2, HMAC, constant-time comparison, random bytes, Argon2id passwords, hex |
| `std.sql` | SQLite |
| `std.net` | TCP sockets, and TLS as a client |
| `std.http` | An HTTP/1.1 server, and a client with TLS |
| `std.process` | Other programs, run directly with no shell |
| `std.html` | HTML built with escaping |
| `std.dom` | Browser pages, in the Elm architecture |

A package needs a permission for some modules: `files` for `std.fs` and
`std.sql`, `env` for `std.env`, `net` for `std.net` and `std.http`, and `process` for
`std.process`.
See [security.md](security.md#package-permissions).

Text from the system that is not valid UTF-8 has its bad parts replaced,
except that `fs.read_text` gives `NotText`.

## std.args, std.env, and std.console

| Task | Gives |
| --- | --- |
| `args.all()` | `list[text]`: what follows the program on its command line |
| `args.get(index)` | `text?`: one of them |
| `env.get(name)` | `text?`: a variable, if it is set |
| `console.read_line()` | `text?`: the next line of standard input, without its line ending, or `none` at the end of input |
| `console.write(content)` | Writes `content` with no newline, and flushes it, for a prompt |

In a package, `env.get` of a name the `env` permission does not list
traps.

## std.fs

| Task | Gives |
| --- | --- |
| `fs.read_text(path)` | `text or FileError` |
| `fs.write_text(path, content)` | `none or FileError`. Replaces the file, or makes it |
| `fs.append_text(path, content)` | `none or FileError`. Adds to its end, or makes it |
| `fs.read_bytes(path)` | `list[byte] or FileError` |
| `fs.write_bytes(path, data)` | `none or FileError` |
| `fs.exists(path)` | `bool` |
| `fs.list_dir(path)` | `list[text] or FileError`: the names, sorted |
| `fs.real_path(path)` | `text or FileError`: the absolute path, with no `.`, `..`, or links |
| `fs.make_dir(path)` | `none or FileError`, making any parents it needs |
| `fs.remove(path)` | `none or FileError`: a file or an empty directory |
| `fs.rename(from, to)` | `none or FileError`, replacing whatever is at `to` |

`FileError` is a `choice`: `NotFound(path)`, `Denied(path)`,
`NotText(path)`, or `Failed(path, detail)`. Paths are text, and `/` works
on every system, so `"{dir}/{name}"` joins two.

In a package, a path outside those the `files` permission lists gives
`Failed`, and `exists` gives `false`.

## std.process

A program runs directly, never through a shell, so no argument is read as
shell syntax. Rust's `std::process::Command` finds the program and, on
Windows, quotes each argument. An argument to a `.bat` or `.cmd` file
that cannot be quoted safely gives `Failed`.

| Task | Gives |
| --- | --- |
| `process.run(program, args, input: "", seconds: 0)` | `Output or ProcessError`. Waits for the program to end, with `input` on its standard input. With `seconds` above 0, a program that takes longer is killed, and the result is `TimedOut` |
| `process.start(program, args)` | `Child or ProcessError`, at once |
| `child.write(content)` | `none or ProcessError`. Writes to its standard input. Add `"\n"` to end a line |
| `child.close_input()` | Closes its standard input, so it reads the end of its input |
| `child.read_line()` | `text?`: the next line of its standard output, without its line ending, or `none` once it closes it |
| `child.wait()` | `int or ProcessError`: closes its standard input, waits for it to end, and gives its exit code |
| `child.kill()` | `none or ProcessError`. A child that has already ended is not a failure |

An `Output` has `code`, `out`, and `err`, the last two as text, with bytes
that are not UTF-8 replaced. On Unix, a program ended by a signal has the
signal's number, negated, as its code. A started child's standard error
goes to the program's own. A child closes its pipes when no value holds
it, and is not killed then. `ProcessError` is `NotFound(program)`,
`TimedOut`, or `Failed(detail)`.

```lucid
use std.process

match process.run("git", ["status", "--short"], seconds: 10)
    ran: process.Output -> print ran.out
    e: process.ProcessError -> print e.error_message()
```

## std.time

| Task | Gives |
| --- | --- |
| `time.now()` | `Time`: the wall clock, which can jump |
| `time.clock()` | `Instant`: a clock that only goes forward, for measuring |
| `later.since(earlier)` | `Duration`, between two `Time` or two `Instant` values |
| `time.seconds(n)`, `time.millis(n)` | `Duration` |
| `d.to_seconds()`, `d.to_millis()` | `float`, `int` |
| `time.sleep(length)` | Waits. A negative length does not wait |
| `t.date()`, `date.time()` | `Date` and `Time`, each from the other, in UTC |

A `Duration` has `+`, `-`, and an order. A `Time` holds nanoseconds since
1970 in its `nanos` field, which reaches from 1678 to 2262. A `Date` has
`year`, `month`, `day`, `hour`, `minute`, and `second`, the last three
defaulting to 0, and shows in ISO 8601.

There are no time zones, since local time needs the zone tables. Parsing
dates will come when a program needs it.

## std.json

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

The compiler derives `Json` for a type. No annotations or generators are
needed. How each type maps to JSON is under "Traits" in
[language.md](language.md).

`json.parse[T](raw)` gives `T or JsonError`, and `json.write(value)`
gives `text`, with no spaces. A `JsonError` has `message` and `path`, as
in `.books[2].title`. Input nested more than 128 levels deep is an
error, so it cannot overflow the stack. A `u64` reads only up to the
largest `int`.

For JSON whose shape is known only once it is read, such as the
`params` of a JSON-RPC message, `json.tree(raw)` gives
`json.Value or JsonError`, with the same reader and depth limit, and
`json.write_tree(value)` gives its text. A `Value` is `Null`,
`Flag(value: bool)`, `Whole(value: int)`, `Real(value: float)`,
`Str(value: text)`, `Items(value: list[Value])`, or
`Fields(value: map[text, Value])`. A number with no `.` or exponent that
fits an `int` is `Whole`. An object keeps its names in the order they
first appear, with the last value of a repeated name. A `Value` cannot
yet be a field of a type that `json.parse` reads.

```lucid
use std.json

let message = try json.tree("{{\"id\": 7, \"params\": [1, \"a\"]}}")
match message
    Fields(fields) -> print json.write_tree(fields["params"])
    _ -> print "not an object"
```

## std.crypto

| Task | Gives |
| --- | --- |
| `crypto.sha256(data)`, `crypto.sha512(data)` | `list[byte]`: the digest, 32 or 64 bytes |
| `crypto.hmac_sha256(key, data)` | `list[byte]`: the 32-byte tag |
| `crypto.hmac_check(key, data, tag)` | `bool`, in constant time |
| `crypto.same(a, b)` | `bool`: the same bytes, in a time that depends only on the lengths |
| `crypto.random_bytes(count)` | `list[byte]` from the system's cryptographic source. A negative `count` traps |
| `crypto.hash_password(password)` | `text`: an Argon2id PHC string, with a fresh random salt |
| `crypto.check_password(password, hash)` | `bool`. A `hash` that is not an Argon2 PHC string gives `false` |
| `crypto.hex(data)` | `text`: lowercase hex |

The runtime wraps `ring`, `subtle`, and `argon2`. Passwords use m=19456,
t=2, p=1, OWASP's first setting. `check_password` reads the setting from
the hash, so raising it later still checks old hashes. Text goes in
through `.bytes()`. The module needs no permission.

## std.sql

| Task | Gives |
| --- | --- |
| `sql.open(path)` | `sql.Db or sql.SqlError`. `":memory:"` gives a new, empty database in memory |
| `db.run(query, values)` | `int or SqlError`: how many rows one statement changed, counting those changed by triggers and foreign keys |
| `db.query(query, values)` | `list[sql.Row] or SqlError`. Each `Row` has `columns`, a `list[text]`, and `values`, a `list[sql.Value]` in the same order |

`std.sql` is SQLite 3.54.1, built into the runtime from `libsqlite3-sys`.

A query has type `sql`, and only a text literal without `{...}`, or
`sql + sql`, makes one. So a value goes in only through a `?` mark:
`values` holds one `sql.Value` per mark, in order, and a different count
traps. A `Value` is `Null`, `Int(value)`, `Float(value)`, `Text(value)`,
or `Bytes(value)`, and rows come back in the same kinds. A query holds
exactly one statement, which a comment may follow.

A connection waits up to 5 seconds for another connection's lock. A
`SqlError` is `Constraint(detail)`, `Busy(detail)`, or `Failed(detail)`,
and `error_message()` gives the detail. A `Db` closes when no value holds
it, and may be sent to another thread. `ATTACH` and `VACUUM INTO` a file
fail, since only the `files` permission says which files may open.

## Servers

The server side of a website compiles to a native program;
[examples/website.lcd](../examples/website.lcd) is one.

### std.net

`net.listen(port)` gives a `Listener`, whose `accept()` gives a
`Connection`, as `net.connect(host, port)` does. Port 0 takes any free
port, and `listener.port` says which. A listener is on `127.0.0.1` unless
given `host: "0.0.0.0"`.

A connection has `read(max)`, `write(bytes)`, and `set_timeout(length)`,
which takes a `time.Duration`. `read` gives an empty list once the other
end closes. Each fails with a `NetError`: `Refused`, `TimedOut`, `InUse`,
`Closed`, or `Failed(detail)`. A socket closes when no value holds it.
`connection.secure(host)` starts TLS as a client and gives the
connection to use from then on.

### std.http

```lucid
use std.http

var app = http.App()
app.get("/", request => http.text("hello"))
try app.serve(8080)
```

`app.get(path, handler)` and `app.post(path, handler)` route by method and
exact path. `app.serve(port)` serves HTTP/1.1 with keep-alive, one thread
per connection and at most 256 at once, on `127.0.0.1` unless given
`host: "0.0.0.0"`. It gives a `NetError` only if it cannot listen.
`app.answer(connection)` serves one connection the program accepted
itself.

A handler is a `task(http.Request) -> http.Response`. A `Request` has
`method`, `path`, `query` (what follows the `?`), `headers` with
lowercase names, and `body` bytes, with `request.header(name)` and
`request.body_text()`. A `Response` has `status`, `headers`, and `body`;
`http.page(node)`, `http.json(value)`, and `http.text(content)` each
build one with status 200.

`HEAD` runs the `GET` handler. An unknown path gives 404, and a known
path with another method gives 405. A handler that traps ends the
server. What the server refuses, and its limits, are in
[security.md](security.md#standard-library-defaults); after a refusal it
closes in stages, so the client can read the answer.

The client is `http.get(url)` and `http.post(url, kind, body)`, where
`kind` is the media type and `body` is bytes. Each gives
`Response or NetError`, with lowercase header names,
`response.header(name)`, and `response.body_text()`. An `https` URL uses
TLS, and the certificate must be for the host and chain to a root
Mozilla trusts; the check cannot be turned off. Each request uses its own
connection, and redirects are not followed.

Both take `headers: list[Header] = []`, such as `Authorization`, and
`seconds: int = 30`, the longest wait for any one read, which must be
above 0. A `User-Agent` in `headers` replaces `lucid`. Each header is
checked as the server checks its own, and `Host`, `Content-Length`,
`Content-Type`, `Connection`, and `Transfer-Encoding` cannot be given,
since the client writes them. A refused header, or `seconds` of 0 or
less, gives `Failed` before any connection.

### std.html

`html.Element(tag, attrs, children)`, `html.Text(content)`,
`html.Attribute(name, value)`, and `html.document(title, children)` build
a page, and `html.render(node)` writes it, escaping every text and
attribute value. `html.escape(content)` escapes one text.

Tags that hold text take it directly: `h1`, `h2`, `h3`, `p`, `li`,
`span`, `strong`, `em`, `code`, `pre`, and `a(href, content)`. Tags that
hold elements take a list: `div`, `ul`, `ol`, `main`, `section`. A void
element such as `br` renders with no end tag. `std.html` works on the
web too.

## The browser

`lucid build --target web` compiles a program to WebAssembly. The output
is one JavaScript file, `NAME.js`, holding a small loader and both
WebAssembly modules, so a page needs only `<script src="NAME.js">`.

Every feature works there except a user `extern` and `.pointer()`, which
are E0905. `std.net`, `std.http`, `std.crypto`, `std.sql`, and
`std.process` are E0907.
`std.fs` gives `Failed`, `env.get` gives `none`, `args.all()` gives `[]`,
`console.read_line` gives `none`, `console.write` goes where `print`
goes, and `time.sleep` traps.

WebAssembly cannot touch the page directly, so all access goes through
`std.dom`. It works only with `--target web`; anywhere else it is E0906.

### std.dom

A page follows the Elm architecture. The program has

* a model: any value holding the page's state;
* `view(model)`, which gives a `dom.Node[Msg]`, a tree whose handlers
  each produce a message;
* `update(model, message)`, which gives the new model.

`dom.app(init, view, update)` shows `view(init)` in the page's `<body>`.
After each event it runs `update`, then `view`, and changes only the
nodes that differ. A node is kept when the new one has the same tag in
the same place, so an `<input>` keeps its focus.
[examples/web](../examples/web) is a complete to-do page.

| Constructor | Makes |
| --- | --- |
| `dom.Element(tag:, attrs: [], children: [])` | An element |
| `dom.Text(content:)` | Text, never read as HTML |
| `dom.Attribute(name:, value:)` | An attribute. `value` also sets an `<input>`'s value, and `checked` whether it is checked, so a reused checkbox shows the model |
| `dom.On(event:, message:)` | A handler. `message` is a `task(text) -> Msg`, given the event's key for a key event, else the element's value, as an `<input>` has, else `""` |

Events run one at a time. An event caused by a change to the page, such
as `blur` when a focused node goes away, waits for the handler running
now, and is dropped if its node has left the page. A handler cannot hold
state of its own, since a capture cannot change.

A page traps on a `script` element, on an attribute whose name starts
with `on`, on `formaction` and `srcdoc`, and on a `javascript:` or
`data:text/html` value, since each would run code the program did not
write. A keyed diff, for lists that reorder, will come when a program
needs it.
