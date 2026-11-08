# Security

## Guarantees

Safe Lucid code cannot produce:

| Failure class | Prevented by |
| --- | --- |
| Use after free, double free | Ownership, reference counting, and the exclusivity checks |
| Buffer overflow | Bounds checks, unless the compiler proves them redundant |
| Null dereference | No null. `T?` must be narrowed before use |
| Uninitialized reads | All values initialized |
| Data races | Tasks do not share mutable data outside `lock` and `channel` |
| Silent integer overflow | Traps in all builds |
| Stack overflow past the stack's end | A check on entry to each task traps first |
| Ignored errors | Unused errors do not compile |
| Unchecked casts | No unchecked casts. The compiler proves each narrowing, and a tag test made it true: assignment and loops end what a test proved. A call on an `any` reads the value only after a test of its tag |

A trap ends the program. It is a controlled stop, not undefined behavior. A trap in any thread ends the whole process, by decision 172, so a request that traps can stop a server. Run a server under a process supervisor that restarts it. A trap is not contained because it can stop a write through a lock halfway.

Safe code has **no undefined behavior**. A compiler or runtime bug that breaks this is a release blocker.

## Hash flooding

Map and set hashes use SipHash through Rust's `RandomState`, with keys chosen once per process. On the web, where `RandomState` has no random source, the keys come from `crypto.getRandomValues`, through `SipHasher::new_with_keys`. So whoever supplies a map's keys cannot plan collisions, and a map's order never depends on the hash. A derived hash of a compound key is a chain of the same keyed steps. A hand-written `hash` gives a value that is hashed the same way, so its only collisions are its own equal results. A hand-written `hash` or `equal` that breaks the rules can lose a key, and cannot break memory safety: a probe always ends, and each task only reads borrowed values. The Rust documentation does not promise this algorithm forever, so review it when the toolchain changes.

## Unsafe code

`unsafe` blocks are the only place for foreign calls and for taking a raw pointer. Error E0330 enforces this. Each block needs a comment, and error E0332 enforces that one exists, not that it is true. `lucid check --unsafe` lists all `unsafe` sites in a program and all it imports, dependencies too, but not `std`, which ships with the compiler.

Inside `unsafe`, the guarantees above are the author's job. A C call can do anything, a wrong declared type corrupts values, and a pointer used after its list changes is a use after free. `bool` from C is normalized, and integer results are extended from their declared width, so a correct declaration gives a valid Lucid value.

## Package permissions

A package gets no powers unless granted. Each package declares what it needs in `lucid.toml`:

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
| `files` | `true`, or a list of paths | `std.fs`, and `std.sql` when planned for M8, limited to the listed paths |
| `env` | `true`, or a list of names | `std.env`, limited to the listed names |
| `process` | `true` | `std.process` |
| `unsafe` | `true` | `unsafe` blocks and `extern` declarations |

`std.args`, `std.json`, and `std.time` need no permission. `std` itself needs none, since it ships with the compiler.

`false`, an empty list, and a permission left out all grant nothing. A listed path or name cannot be empty, and cannot hold a control character such as a line break. A Windows path in a `"..."` string must write each `\` as `\\`, or else `"\new"` would hold a line break. E0338 rejects an unknown permission. The rules below are built, by decision 146.

**Who is checked:** every module of a package, the root package too. A `use ./` cannot reach a file under another `lucid.toml`, or else E0339, so a file cannot take a nested package's permissions. A program with no `lucid.toml` is not a package, and has full trust, as a script has today.

**At compile time:** error E0340 rejects:

- a `use` of a `std` module, in a package without the permission that unlocks it
- an `unsafe` block or `extern` declaration, in a package without `unsafe`
- a dependency that holds a permission that its `allow` entry does not name
- an `allow` entry that names a permission its own package does not hold
- a dependency's path or name list that its parent's list does not cover. `true` covers any list. A path covers itself and every path below it, compared after `.` and `..` are removed. So `.` covers a relative path that stays inside it, and not an absolute path or one that starts with `..`

**At run time:** paths and names are checked against the root package's lists, for the whole program, under `lucid run`, `lucid test`, and in a built program. A listed path is relative to the working directory at start, and the program cannot change that directory. Each `std.fs` call resolves its path, and each listed path, through any links that exist, and then the path must lie at or below a listed path. A link must lie there too, and so must what it points to, since `fs.remove` and `fs.rename` act on the link and the other tasks on its target. A link that cannot be followed, such as one whose target does not exist yet, is refused, since writing through it could create a file anywhere. Outside them, a task that can fail returns `Failed`, with a detail that names the `files` permission, and `fs.exists` returns `false`. `env.get` of an unlisted name traps, since a name is almost always fixed in the code, and so a wrong one is a bug. Names compare exactly, so on Windows `path` does not match a listed `PATH`. That errs toward refusing.

**Limit of the run-time check:** a dependency can reach any path or name that the root lists, not just its own. The compile-time rules still make the root's author list, and so approve, each path a dependency asks for. Checking per package would need each package to have its own copy of `std` tasks, while their types stay shared. Deno also checks paths for the whole process. A link changed between the check and the call can escape the check. Both are within the limit below: permissions are not a sandbox.

**Transitive rule:** a package must hold every permission that its dependencies hold. Otherwise, a package with no `net` permission could reach the network by calling a dependency that has it.

**Limit:** `unsafe` and `process` can bypass all of this. Native code ignores these checks, and a child process does not inherit them. Treat both as full trust, and grant them rarely. Deno and Node.js document the same limits.

Permissions are not a sandbox. They limit what dependencies can do by accident or casual misuse. To run untrusted code, use operating system isolation.

## Supply chain

- `lucid.lock` will pin every registry package by the SHA-256 of its archive, by decision 199. A hash that differs is an error, never a new fetch. Planned for M8. Today a dependency is a local path, so nothing pins it.
- The lock protects only after the first download. Lucid has no checksum log like Go's, so a registry that serves a bad archive to a new user is not caught.
- An archive holds only paths of lowercase ASCII letters, digits, `_`, `-`, `.`, and `/`, with no `.` or `..` part, so it cannot write outside its cache directory, by decision 200. A registry package cannot have a `path` dependency.
- Packages have no build scripts and no install hooks. Compiling a dependency never runs its code.
- A compile-time execution feature, if ever added, must have no access to files or the network.

## Standard library defaults

- HTML building escapes by default. `std.html` escapes every text and attribute value, and traps on what `std.dom` traps on, by decision 180. It also traps on a tag or attribute name that is not plain letters, digits, and `-`, by decision 186, since a name such as `a onclick=x` would add an attribute that no escape covers.
- `std.dom` never reads text as HTML: `dom.Text` makes a text node, and an attribute is set with `setAttribute`. It traps on a `script` element, an attribute whose name starts with `on`, `formaction`, `srcdoc`, and a `javascript:` or `data:text/html` value, by decision 168, so a page runs only code that the program wrote, even when an attribute's value comes from a user. Handlers go through `dom.On`. A trap in a handler stops the page from taking events.
- `std.sql` takes values only through `?` marks, and a query has the type `sql`, which only text literals and `sql + sql` make, by decision 195. So text from a user cannot become part of a query. Planned for M8.
- `std.crypto` hashes passwords with Argon2id at OWASP's setting, and compares secrets in constant time, by decision 193. Planned for M8.
- TLS verification is always on, by decisions 179 and 187: `connection.secure(host)`, and the `std.http` client for an `https` URL, check that the certificate is for the host and chains to a root that `webpki-roots` holds, and there is no option to turn the check off. The `std.http` client takes only a URL of printable ASCII, so a URL cannot add a line to the request, and traps on a media type with a line break. It refuses a response with two lengths, a length with chunks, or another coding, a head over 16 KiB, or a body over 16 MiB, and fails after 30 seconds with no bytes. It follows no redirect, so it never sends a body to a host the program did not name.
- `net.listen` listens on `127.0.0.1`, so only this machine can connect, unless the program passes `host: "0.0.0.0"`, by decision 185. A `Connection` has `set_timeout`, so a silent client cannot hold a thread forever.
- By decisions 178 and 186, the `std.http` server rejects a request with both `Content-Length` and `Transfer-Encoding`, two `Content-Length` values, a coding other than `chunked`, or a folded header, and closes the connection, so a proxy in front cannot read one request where the server reads two. By decision 207 it first closes its write half and reads what is left, for at most 2 seconds of silence and 1 MiB. A request head over 16 KiB, a body over 1 MiB, and a connection silent for 30 seconds are refused, and at most 256 connections run at once. It also rejects an HTTP/1.1 request with no `Host` or two, and a space before a header's colon. A handler cannot set `Content-Length`, `Transfer-Encoding`, or `Connection`, nor a header with a line break: each one traps, since it would let a handler write a second response.
- Planned, with no milestone: a `secret` type that prints as `***`.
- Random values come from a cryptographic source: `crypto.random_bytes`, planned for M8. There is no other source of random values.
