---
title: Standard library
description: The prelude and the std modules that are built today, and what is planned.
---

The standard library is written in Lucid and ships inside the compiler, so a program needs no files beside it. `lucid context std.NAME` prints the public API of any module, with its doc comments.

| Module | Contents | State |
| --- | --- | --- |
| prelude | Text, lists, maps, sets, math, core traits, text search, split, trim, case change, list sort, `channel`, `lock` | Built |
| `std.fs` | Files and directories | Built |
| `std.json` | Parse and write, typed by the `Json` trait | Built |
| `std.env`, `std.args` | Environment variables and command-line words | Built |
| `std.time` | Clocks, durations, and UTC dates | Built |
| `std.dom` | Browser pages, see [WebAssembly](/docs/webassembly/) | Built |
| `std.net` | TCP sockets | Built |
| `std.http` | HTTP/1.1 server, and a client with TLS | Built |
| `std.html` | HTML building that escapes on render | Built |
| `std.crypto` | SHA-256, SHA-512, HMAC, constant-time compare, random bytes, Argon2id | Built |
| `std.sql` | SQLite, with values only through `?` marks | Built |

## Arguments and environment

```lucid
use std.args
use std.env

let name = args.get(0) else "world"    # the first word after the program
print "Hello, {name}!"
print env.get("HOME") else "no HOME"
```

## Files

```lucid
use std.fs

task copy(from: text, to: text) -> none or fs.FileError
    let content = try fs.read_text(from)
    try fs.write_text(to, content)

print fs.exists("notes.txt")
copy("notes.txt", "backup.txt") else none   # ignore a missing file, on purpose
```

`std.fs` reads and writes whole files: `read_text`, `write_text`, `append_text`, `read_bytes`, `write_bytes`, `list_dir`, `make_dir`, `remove`, and `rename`. Each gives its result or an `fs.FileError`, which is `NotFound`, `Denied`, `NotText`, or `Failed`. `exists` gives a `bool`. `/` works in paths on every system.

## JSON

```lucid
use std.json

type Book
    title: text
    pages: int = 0

let raw = json.write([Book(title: "Dune", pages: 412)])
print raw
let books = json.parse[list[Book]](raw) else []
print books[0].title
```

`Json` is derived, with no annotations. A `type` maps to an object, a `choice` to `"Red"` or `{"Circle": {"radius": 1.0}}`, and a field left out takes its default.

## Time

```lucid
use std.time

let start = time.clock()
time.sleep(time.millis(5))
print time.clock().since(start).to_millis() >= 5
print time.Date(year: 2026, month: 11, day: 6).time().date()
```

`time.now()` is the wall clock, and `time.clock()` only goes forward, for measuring. There are no time zones: dates are UTC.

## HTTP

A server routes by method and exact path:

```lucid
use std.html as h
use std.http

task home(request: http.Request) -> http.Response
    return http.page(h.document("Lucid", [h.h1("Hello from Lucid")]))

var app = http.App()
app.get("/", home)
try app.serve(8080)
```

`app.serve` listens on `127.0.0.1` unless given `host: "0.0.0.0"`, runs one thread per connection, and never returns. `http.json(value)` and `http.text(content)` build other responses.

The client checks `https` certificates, with no way to turn the check off, and follows no redirect:

```lucid
use std.http

let got = try http.get("https://example.com/")
print got.status
```

## HTML

`std.html` escapes every text and attribute value when it renders, and traps on a `script` element, an `on...` attribute, or a `javascript:` link. So text from a user cannot become markup or code.

## Crypto

`std.crypto` wraps vetted code: `ring`, `subtle`, and `argon2`. Each task takes bytes, so pass text through `.bytes()`:

```lucid
use std.crypto

print crypto.hex(crypto.sha256("abc".bytes()))
let hash = crypto.hash_password("correct horse")
print crypto.check_password("correct horse", hash)
let token = crypto.random_bytes(32)
print crypto.same(token, token)
```

Passwords use Argon2id at OWASP's setting, in a PHC string. Compare secrets with `crypto.same` or `crypto.hmac_check`, which take the same time wherever the bytes differ. `crypto.random_bytes` is the only source of random values. On the web, `std.crypto` is error E0907.

## SQL

`std.sql` is SQLite, built into the program. A query has the type `sql`, which only a text literal with no `{...}` makes, so every value goes through a `?` mark:

```lucid
use std.sql

let db = try sql.open(":memory:")
try db.run("CREATE TABLE users (name TEXT UNIQUE, age INTEGER)", [])
try db.run("INSERT INTO users VALUES (?, ?)", [sql.Text(value: "ada"), sql.Int(value: 36)])
for row in try db.query("SELECT name FROM users WHERE age > ?", [sql.Int(value: 30)])
    print row.values[0]
```

`db.run` gives how many rows changed, and `db.query` gives rows, each with `columns` and `values`. Queries join with `+`, but never with `text`: `"... WHERE name = '{name}'"` is a compile error. A failure is a `SqlError`: `Constraint`, `Busy`, or `Failed`. Opening a file needs the `files` permission, and no query can attach another file. On the web, `std.sql` is error E0907.

## Not built yet

There is no console input, no tuples (use a `type`), and no `join`, `replace`, `map`, `filter`, `sum`, `min`, or `max`. Write the ones you need as tasks.
