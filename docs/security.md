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

`unsafe` blocks are the only place for foreign calls and for taking a raw pointer. Error E0330 enforces this. Each block needs a comment, and error E0332 enforces that one exists, not that it is true. `lucid check --unsafe` lists all `unsafe` sites in a file. Listing those in dependencies waits for packages.

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
markdown = { version = "1.2", allow = [] }
mailer = { version = "0.4", allow = ["net"] }
```

| Permission | Unlocks |
| --- | --- |
| `net` | `std.net`, `std.http` |
| `files` | `std.fs`, limited to listed paths |
| `env` | `std.env`, limited to listed names |
| `process` | `std.process` |
| `unsafe` | `unsafe` and `extern` |

**Enforcement:** imports are checked at compile time. A dependency that imports `std.net` without an `allow` entry from its parent fails the build. Path and name limits are enforced at run time in `std`.

**Transitive rule:** a package must hold every permission that its dependencies hold. Otherwise, a package with no `net` permission could reach the network by calling a dependency that has it.

**Limit:** `unsafe` and `process` can bypass all of this. Native code ignores these checks, and a child process does not inherit them. Treat both as full trust, and grant them rarely. Deno and Node.js document the same limits.

Permissions are not a sandbox. They limit what dependencies can do by accident or casual misuse. To run untrusted code, use operating system isolation.

## Supply chain

- `lucid.lock` pins every dependency by content hash.
- Packages have no build scripts and no install hooks. Compiling a dependency never runs its code.
- A compile-time execution feature, if ever added, must have no access to files or the network.

## Standard library defaults

- HTML building escapes by default.
- SQL APIs take parameters, and they reject plain interpolated text through a distinct `sql` type.
- TLS verification is on, and turning it off is an explicit, named option.
- Secrets use a `secret` type that prints as `***`.
- Random values default to a cryptographic source.
