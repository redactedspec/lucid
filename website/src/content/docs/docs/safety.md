---
title: Safety
description: What safe Lucid code cannot do, how traps work, unsafe blocks, and the limits of these guarantees.
---

## Guarantees

Safe Lucid code cannot produce:

| Failure | Prevented by |
| --- | --- |
| Use after free, double free | Ownership, reference counting, and exclusivity checks |
| Buffer overflow | Bounds checks |
| Null dereference | There is no null. A `T?` must be narrowed before use |
| Uninitialized reads | Every value is initialized |
| Data races | Threads share mutable data only through `lock` and `channel` |
| Silent integer overflow | Overflow traps in every build |
| Stack overflow past the stack's end | A check on entry to each task traps first |
| Ignored errors | An unused error does not compile |
| Unchecked casts | Every narrowing is proved by a test |

Safe code has no undefined behavior. A compiler or runtime bug that breaks this is treated as a release blocker.

## Traps

A trap is a controlled stop, not undefined behavior. It prints the source position and a stack trace, and exits with code 101. A trap in any thread ends the whole process, so run servers under a supervisor that restarts them.

## Maps and hash flooding

Map and set hashes use SipHash with keys chosen once per process, so whoever supplies the keys cannot plan collisions. A map's order is insertion order, and never depends on the hash.

## unsafe

Foreign C calls and raw pointers are only allowed inside an `unsafe` block, which needs a comment that says why it is sound.

```lucid
unsafe extern task strlen(s: pointer[byte]) -> u64

var raw = "hello".bytes()
raw.push(0)

# raw ends in a zero byte, and stays unchanged for the call.
unsafe
    print strlen(raw.pointer())
```

Inside `unsafe`, the guarantees above are the author's job. `lucid check --unsafe` lists every `unsafe` block and `extern` task in a program and all it imports, so a reviewer can find each one. In a package, `unsafe` needs the `unsafe` permission.

## Libraries that are safe by default

- `std.html` and `std.dom` escape text, and trap on script elements, event attributes, and `javascript:` links.
- The `std.http` client always verifies TLS certificates, with no option to turn the check off.
- The `std.http` server rejects ambiguous requests that could be read as two, and limits header size, body size, idle time, and connections.
- `net.listen` listens only on `127.0.0.1` unless told otherwise.
- Planned for M8: `std.sql` takes values only through `?` marks, and `std.crypto` hashes passwords with Argon2id.

## Package permissions

A package gets no file, network, environment, or `unsafe` access unless its `lucid.toml` grants it, and each dependency's access must be approved by its parent. See [Modules and packages](/docs/modules-and-packages/#permissions). Permissions are not a sandbox: run untrusted code under operating system isolation.

Packages have no build scripts or install hooks, so compiling a dependency never runs its code.
