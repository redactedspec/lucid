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

A trap ends the program. It is a controlled stop, not undefined behavior. A trap can still be a denial-of-service risk, so servers need a containment design (see open questions in `decisions.md`).

Safe code has **no undefined behavior**. A compiler or runtime bug that breaks this is a release blocker.

## Hash flooding

Map and set hashes use SipHash through Rust's `RandomState`, with keys chosen once per process. So whoever supplies a map's keys cannot plan collisions, and a map's order never depends on the hash. A derived hash of a compound key is a chain of the same keyed steps. A hand-written `hash` gives a value that is hashed the same way, so its only collisions are its own equal results. A hand-written `hash` or `equal` that breaks the rules can lose a key, and cannot break memory safety: a probe always ends, and each task only reads borrowed values. The Rust documentation does not promise this algorithm forever, so review it when the toolchain changes.

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
| `files` | `true`, or a list of paths | `std.fs`, limited to the listed paths |
| `env` | `true`, or a list of names | `std.env`, limited to the listed names |
| `process` | `true` | `std.process` |
| `unsafe` | `true` | `unsafe` blocks and `extern` declarations |

`std.args`, `std.json`, and `std.time` need no permission. `std` itself needs none, since it ships with the compiler.

**State:** `lucid.toml` is read and its permissions are validated, and E0338 rejects an unknown one. Nothing enforces them yet. The design below is planned for M5, by decision 146.

**Who is checked:** every module of a package, the root package too. A program with no `lucid.toml` is not a package, and has full trust, as a script has today.

**At compile time (planned):** error E0340 rejects:

- a `use` of a `std` module, in a package without the permission that unlocks it
- an `unsafe` block or `extern` declaration, in a package without `unsafe`
- a dependency that holds a permission that its `allow` entry does not name
- an `allow` entry that names a permission its own package does not hold
- a dependency's path or name list that its parent's list does not cover. `true` covers any list. A path covers itself and every path below it, compared after `.` and `..` are removed

**At run time (planned):** paths and names are checked against the root package's lists, for the whole program. A listed path is relative to the working directory at start, and the program cannot change that directory. Each `std.fs` call resolves its path through any links that exist, and then must lie at or below a listed path. Outside them, a task that can fail returns `Failed`, with a detail that names the `files` permission, and `fs.exists` returns `false`. `env.get` of an unlisted name traps, since a name is almost always fixed in the code, and so a wrong one is a bug.

**Limit of the run-time check:** a dependency can reach any path or name that the root lists, not just its own. The compile-time rules still make the root's author list, and so approve, each path a dependency asks for. Checking per package would need each package to have its own copy of `std` tasks, while their types stay shared. Deno also checks paths for the whole process. A link changed between the check and the call can escape the check. Both are within the limit below: permissions are not a sandbox.

**Transitive rule:** a package must hold every permission that its dependencies hold. Otherwise, a package with no `net` permission could reach the network by calling a dependency that has it.

**Limit:** `unsafe` and `process` can bypass all of this. Native code ignores these checks, and a child process does not inherit them. Treat both as full trust, and grant them rarely. Deno and Node.js document the same limits.

Permissions are not a sandbox. They limit what dependencies can do by accident or casual misuse. To run untrusted code, use operating system isolation.

## Supply chain

- `lucid.lock` will pin every dependency by content hash, at M8 with the registry. Today a dependency is a local path, so nothing pins it.
- Packages have no build scripts and no install hooks. Compiling a dependency never runs its code.
- A compile-time execution feature, if ever added, must have no access to files or the network.

## Standard library defaults

- HTML building escapes by default.
- SQL APIs take parameters, and they reject plain interpolated text through a distinct `sql` type.
- TLS verification is on, and turning it off is an explicit, named option.
- Secrets use a `secret` type that prints as `***`.
- Random values default to a cryptographic source.
