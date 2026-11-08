---
title: Installation
description: Release status of the Lucid compiler, supported platforms, and what a build from source needs.
---

:::note[No release yet]
There are no Lucid binaries to download, and the source repository is private. Nothing on this page can be installed by the public yet. [Downloads](/downloads/) will list releases when they exist.
:::

## Platforms

| Platform | State |
| --- | --- |
| Windows x86-64 | Works in development: `run`, `test`, `check`, `build`, and `build --target web` |
| Linux x86-64 | Works in development: `run`, `test`, `check`, `build`, and `build --target web` |
| Browser and Node | An output target: `lucid build --target web` |
| macOS | Deferred, with no milestone |

## Planned installers

These commands are reserved for the first release. Today each script prints that no release exists and stops with an error, without downloading anything.

```sh
# Windows, PowerShell
irm https://lucidlang.dev/install.ps1 | iex

# Linux
curl -fsSL https://lucidlang.dev/install.sh | sh
```

When a release exists, the installers will read the published release list, download over HTTPS, check each archive's SHA-256, and install into a user directory with no administrator rights.

## Building from source

For those with access to the repository. The compiler is a Rust workspace, and it builds with `cargo`:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release
```

This writes `target/release/lucid`. The build needs:

- A current stable Rust (the 2024 edition; development uses 1.97), with the `wasm32-unknown-unknown` target, which the WebAssembly runtime needs.
- On Windows, Visual Studio Build Tools and the Windows SDK, since `lucid build` links with the MSVC linker. LLVM, clang, and gcc are not needed.
- On Linux, a C compiler as `cc`, such as gcc, since `lucid build` links through it.
- Node, only to run the compiler's own test suite with `cargo test`.

## Editor support

There is no editor extension yet. `lucid lsp`, a language server, is planned for M8.
