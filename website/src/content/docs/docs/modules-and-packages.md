---
title: Modules and packages
description: use, public, packages with lucid.toml, path dependencies, and permissions.
---

## Modules

Each file is one module. `use` brings in another file, relative to the file that holds it:

```text title="models.lcd"
public type User
    name: text

public task greet(user: User) -> text
    return "Hi {user.name}"
```

```text title="main.lcd"
use ./models
use ./models.User

print models.greet(User(name: "Ada"))
```

- `use ./a.b` reads `a/b.lcd`, or else takes the name `b` from `a.lcd`. A `./` path cannot go up a directory.
- A whole module goes by its last name, or by the name after `as`: `use std.json as js`.
- A taken name, such as `use ./models.User`, needs no prefix.
- Names are private unless marked `public`. A public `type` shows all its fields, and a public `choice` all its variants.

Only the entry file runs code. An imported module holds declarations and `let` values with a literal, so importing a module never has an effect, and import order does not matter. Import cycles are an error.

## The standard library

`use std.name` brings in a standard module, with no dependency entry:

```lucid
use std.json

type Book
    title: text
    pages: int = 0

print json.write(Book(title: "Dune", pages: 412))
```

See [Standard library](/docs/standard-library/).

## Packages

A package is a directory with a `lucid.toml` file and a `src` directory. A program has `src/main.lcd`, and a library has `src/lib.lcd`, never both.

```text
blog/
    lucid.toml
    src/
        main.lcd
```

```toml title="lucid.toml"
[package]
name = "blog"
version = "0.1.0"

[permissions]
files = ["./data"]

[dependencies]
markdown = { path = "../markdown", allow = [] }
```

`lucid run`, `build`, `check`, and `test` take a package directory, or use the package around the current directory when given no path.

`use markdown` reads the dependency's `src/lib.lcd`. `use markdown.render` reads `src/render.lcd`, or else takes `render` from `src/lib.lcd`. A package can only `use` what its own `lucid.toml` lists.

### Registry

A dependency with a `version` and no `path` comes from the registry that `[registry]` names, by `https://` address or directory:

```toml title="lucid.toml"
[registry]
url = "https://packages.example.com"

[dependencies]
markdown = { version = "1.2.0" }
```

A version is a floor. The program gets the highest floor that any manifest names, never a newer one. `lucid run`, `check`, `test`, and `build` fetch what is missing into a cache, and write `lucid.lock`, which pins the SHA-256 of each archive: check it in. Two major versions of one package are E0344, a package the registry lacks is E0343, and an archive that changed is E0345.

A registry is static files: `index/NAME`, with one JSON line per version, and `files/NAME/VERSION.lpk`. `lucid package` writes both for a library.

## Permissions

A package gets no powers unless its manifest grants them:

| Permission | Unlocks |
| --- | --- |
| `net = true` | `std.net`, `std.http` |
| `files = true`, or a list of paths | `std.fs`, limited to those paths |
| `env = true`, or a list of names | `std.env`, limited to those names |
| `process = true` | `std.process` |
| `unsafe = true` | `unsafe` blocks and `extern` tasks |

A `use` of a module without its permission is error E0340. A dependency's permissions must be named in its `allow` entry, and its parent must hold them too, so no dependency gains a power its user did not approve.

Permissions limit what dependencies can do by accident or casual misuse. They are not a sandbox: `unsafe` code can bypass them. To run untrusted code, use operating system isolation. A single file with no `lucid.toml` has every permission, as a script does.
