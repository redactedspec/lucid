# Security

## Guarantees

Safe Lucid code cannot produce:

| Failure class | Prevented by |
| --- | --- |
| Use after free, double free | Ownership and reference counting |
| Buffer overflow | Bounds checks, unless the compiler proves them redundant |
| Null dereference | No null. `T?` must be narrowed before use |
| Uninitialized reads | All values initialized |
| Data races | Tasks do not share mutable data outside `lock` and `channel` |
| Silent integer overflow | Traps in all builds |
| Ignored errors | Unused errors do not compile |
| Unchecked casts | No unchecked casts. Union narrowing is checked |

A trap ends the program. It is a controlled stop, not undefined behavior. A trap can still be a denial-of-service risk, so servers need a containment design (see open questions in `decisions.md`).

Safe code has **no undefined behavior**. A compiler or runtime bug that breaks this is a release blocker.

## Unsafe code

`unsafe` blocks are the only place for raw pointers and foreign calls. Each block needs a comment that states why it is sound. `lucid check` reports all `unsafe` sites, including those in dependencies.

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
