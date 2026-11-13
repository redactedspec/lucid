# Security

This document lists what safe Lucid code cannot do, where `unsafe` lifts
those limits, how package permissions work, and what the standard library
does by default to keep a program out of trouble. Any way around a
guarantee here is a release blocker.

## Guarantees

Safe Lucid code cannot produce any of the following.

| Failure | Prevented by |
| --- | --- |
| Use after free, double free | Ownership, reference counting, and the exclusivity checks |
| Buffer overflow | Bounds checks, unless the compiler proves one redundant |
| Null dereference | There is no null. A `T?` must be narrowed before use |
| Reading uninitialized memory | Every value starts with a value |
| Data races | Threads share mutable data only through `lock` and `channel` |
| Silent integer overflow | Overflow traps in every build |
| Running off the end of the stack | A check on entry to each task traps first |
| Ignored errors | An error left unused does not compile |
| Unchecked casts | There are none. The compiler proves each narrowing from a tag test, and assignment and loops end what a test proved. A call on an `any` reads the value only after testing its tag |

A trap ends the program. It is a controlled stop, not undefined
behavior. A trap in any thread ends the whole process, so a request that
traps can stop a server; run servers under a supervisor that restarts
them. Traps are not contained to a thread because a trap can stop a
write through a lock halfway.

Safe code has no undefined behavior. A compiler or runtime bug that
breaks this is a release blocker.

### Hash flooding

Maps and sets hash with SipHash through Rust's `RandomState`, keyed once
per process. On the web, where `RandomState` has no source of
randomness, the keys come from `crypto.getRandomValues` through
`SipHasher::new_with_keys`. Whoever supplies a map's keys cannot plan
collisions, and a map's order never depends on the hash.

A derived hash of a compound key chains the same keyed steps. A
hand-written `hash` returns a value that is then hashed the same way, so
its only collisions are its own equal results. A hand-written `hash` or
`equal` that breaks the rules can lose a key but cannot break memory
safety: a probe always ends, and each task only reads borrowed values.

Rust does not promise this algorithm forever. Review it when the
toolchain changes.

## Unsafe code

Foreign calls and raw pointers are allowed only inside `unsafe` blocks
(E0330). Each block needs a comment saying why it is sound (E0332); the
compiler checks that the comment is there, not that it is true. `lucid
check --unsafe` lists every `unsafe` site in a program and in everything
it imports, dependencies included. It skips `std`, which ships with the
compiler.

Inside `unsafe`, the guarantees above are the author's job. A C call can
do anything. A wrong declared type corrupts values. A pointer used after
its list changes is a use after free. A `bool` from C is normalized, and
an integer result is extended from its declared width, so a correct
declaration always gives a valid Lucid value.

## Package permissions

A package gets no powers unless they are granted. It declares what it
needs in `lucid.toml`, and each dependency is allowed some subset:

```toml
[package]
name = "blog"
version = "0.1.0"

[permissions]
net = true
files = ["./data"]
env = ["PORT"]

[dependencies]
markdown = { path = "../markdown", allow = [] }
mailer = { path = "../mailer", allow = ["net"] }
```

| Permission | Value | Unlocks |
| --- | --- | --- |
| `net` | `true` | `std.net`, `std.http` |
| `files` | `true`, or a list of paths | `std.fs` and `std.sql`, limited to the listed paths |
| `env` | `true`, or a list of names | `std.env`, limited to the listed names |
| `process` | `true` | `std.process` |
| `unsafe` | `true` | `unsafe` blocks and `extern` declarations |

`std.args`, `std.console`, `std.json`, and `std.time` need no permission.

`false`, an empty list, and a permission left out all grant nothing. A
listed path or name cannot be empty or hold a control character such as
a line break. A Windows path in `"..."` must write each `\` as `\\`, or
`"\new"` would hold a line break. An unknown permission is E0338.

Every module of a package is checked, the root package's too. A `use ./`
cannot reach a file under another `lucid.toml` (E0339), so a file cannot
borrow a nested package's permissions. A program with no `lucid.toml` is
not a package and has full trust, as a script does.

### At compile time

E0340 rejects:

* a `use` of a `std` module in a package without the permission that
  unlocks it;
* an `unsafe` block or `extern` declaration in a package without
  `unsafe`;
* a dependency that holds a permission its `allow` entry does not name;
* an `allow` entry that names a permission its own package does not
  hold;
* a dependency's list of paths or names that its parent's list does not
  cover. `true` covers any list. A path covers itself and everything
  below it, compared after `.` and `..` are removed, so `.` covers a
  relative path that stays inside it, but not an absolute path or one
  that starts with `..`.

A package must therefore hold every permission its dependencies hold.
Otherwise a package without `net` could reach the network by calling a
dependency that has it.

### At run time

Paths and names are checked against the root package's lists, for the
whole program, under `lucid run`, `lucid test`, and in a built program.

A listed path is relative to the working directory at start, and the
program cannot change that directory. Each `std.fs` call resolves its
path, and each listed path, through any links that exist; the path must
then lie at or below a listed one. A link must lie there too, and so
must what it points to, since `fs.remove` and `fs.rename` act on the
link and the other tasks on its target. A link that cannot be followed,
such as one whose target does not exist yet, is refused, since writing
through it could create a file anywhere.

Outside the listed paths, a task that can fail returns `Failed`, with a
detail that names the `files` permission, and `fs.exists` returns
`false`. `env.get` of a name not listed traps, since a name is nearly
always fixed in the code, and so a wrong one is a bug. Names compare
exactly, so on Windows `path` does not match a listed `PATH`; that errs
toward refusing.

The Lucid compiler runs a program in its own process, and before the
program starts it gives the runtime the program's lists in place of its
own.

### What permissions do not do

A dependency can reach any path or name the root lists, not only the
ones it asked for. The compile-time rules still make the root's author
list, and so approve, every path a dependency needs. Checking per
package would need each package to have its own copy of the `std` tasks
while their types stay shared. Deno checks paths for the whole process
too.

A link changed between the check and the call can escape the check.

`unsafe` and `process` can bypass all of this. Native code ignores these
checks, and a child process does not inherit them. Treat both as full
trust and grant them rarely. Deno and Node.js document the same limits.

Permissions are not a sandbox. They stop a dependency doing something by
accident or through casual misuse. To run code you do not trust, use the
operating system's isolation.

## Supply chain

`lucid.lock` pins every registry package by the SHA-256 of its archive. A
hash that differs from the index or the lock is E0345, never a fresh
fetch. A cached package's directory is named by its whole hash, so two
registries' packages of the same name and version never share one. Path
dependencies are not pinned.

The lock protects only after the first download. Lucid has no checksum
log like Go's, so a registry that serves a bad archive to a new user is
not caught.

An archive holds only `lucid.toml`, `README.md`, `LICENSE`, and `.lcd`
files under `src`. Their paths hold only lowercase ASCII letters,
digits, `_`, `-`, `.`, and `/`, with no empty, `.`, or `..` part and no
Windows device name such as `con` or `com1`, so an archive cannot write
outside its cache directory. `lucid package` and the client both check
this. A registry package cannot have a `path` dependency.

The index and the archives are data. Fetching runs no code from them,
and the client checks each name, version, and hash in the index before
building a path from it.

Packages have no build scripts and no install hooks. Compiling a
dependency never runs its code. If compile-time execution is ever added,
it must have no access to files or the network.

## Standard library defaults

**HTML.** `std.html` escapes every text and attribute value, and traps
on whatever `std.dom` traps on. It also traps on a tag or attribute name
that is not plain letters, digits, and `-`, since a name such as `a
onclick=x` would add an attribute no escape covers.

**The DOM.** `std.dom` never reads text as HTML. `dom.Text` makes a text
node, and attributes are set with `setAttribute`. It traps on a `script`
element, on an attribute whose name starts with `on`, on `formaction`
and `srcdoc`, and on a `javascript:` or `data:text/html` value, so a
page runs only code the program wrote, even when an attribute's value
came from a user. Event handlers go through `dom.On`. A trap in a
handler stops the page taking events.

**SQL.** `std.sql` takes values only through `?` marks. A query has type
`sql`, and only text literals without interpolation, and `sql + sql`,
make one. No conversion, trait, or `json.parse` produces a `sql`, so text
from a user cannot become part of a query. A query with more or fewer
values than `?` marks traps, since SQLite would read a missing one as
null.

`sql.open` checks its path against the `files` permission as `std.fs`
does, and a path starting with `file:` opens as a plain file, not as a
URI that could name another one. An authorizer refuses `ATTACH` of any
name but the literal empty one `VACUUM` uses, including a name from a
`?` mark, and with it `VACUUM INTO` a file; it also refuses the pragmas
`temp_store_directory` and `data_store_directory`. So a query cannot open
a file outside the permission. Loading extensions through SQL stays off,
as SQLite ships it.

**Cryptography.** `std.crypto` hashes passwords with Argon2id at OWASP's
setting, m=19456, t=2, p=1, with a 16-byte random salt, and compares
secrets in constant time through `crypto.same` and `crypto.hmac_check`.
It wraps `ring`, `subtle`, and `argon2`, and implements no primitive of
its own. Random values come only from a cryptographic source,
`crypto.random_bytes`, through `ring`'s `SystemRandom`.

**TLS.** Verification is always on. `connection.secure(host)`, and the
`std.http` client for an `https` URL, check that the certificate is for
the host and chains to a root in `webpki-roots`. There is no option to
turn the check off.

**The HTTP client.** It takes only a URL of printable ASCII, so a URL
cannot add a line to the request, and it traps on a media type with a
line break. It refuses a response with two lengths, a length together
with chunks, or any other coding, a head over 16 KiB, or a body over
16 MiB, and fails after 30 seconds without a byte. It follows no
redirects, so it never sends a body to a host the program did not name.

**Listening.** `net.listen` listens on `127.0.0.1`, so only the same
machine can connect, unless the program passes `host: "0.0.0.0"`. A
`Connection` has `set_timeout`, so a silent client cannot hold a thread
forever.

**The HTTP server.** It rejects a request with both `Content-Length` and
`Transfer-Encoding`, with two `Content-Length` values, with a coding
other than `chunked`, or with a folded header, and closes the
connection, so a proxy in front cannot read one request where the server
reads two. Before closing, it shuts its write half and reads what is
left, for at most 2 seconds of silence and 16 reads: 1 MiB and 32
seconds at most. It refuses a request head over 16 KiB, a body over
1 MiB, and a connection silent for 30 seconds, and runs at most 256
connections at once. It rejects an HTTP/1.1 request with no `Host` or
with two, and a space before a header's colon. A handler cannot set
`Content-Length`, `Transfer-Encoding`, or `Connection`, or any header
with a line break; each traps, since it would let a handler write a
second response.

Planned, with no milestone: a `secret` type that prints as `***`.

The decisions behind these defaults are 146, 168, 172, 178 to 180, 185 to
187, 193 to 195, 199, 200, 207 to 209, and 211 in
[decisions.md](decisions.md).
