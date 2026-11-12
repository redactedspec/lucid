---
title: Installation
description: Release status of the Lucid compiler, supported platforms, and what a build from source needs.
---

:::note[Build from source]
Version 1.0.0 is tagged, and the source is at [github.com/redactedspec/lucid](https://github.com/redactedspec/lucid). There are no Lucid binaries to download yet, so build it with `cargo` as shown below. [Downloads](/downloads/) will list binaries when they exist.
:::

## Platforms

| Platform | State |
| --- | --- |
| Windows x86-64 | Works in development: `run`, `test`, `check`, `build`, and `build --target web` |
| Linux x86-64 | Works in development: `run`, `test`, `check`, `build`, and `build --target web` |
| Browser and Node | An output target: `lucid build --target web` |
| macOS | Deferred, with no milestone |

## Planned installers

These commands are reserved for when binaries are published. Today each script prints that there are none and stops with an error, without downloading anything.

```sh
# Windows, PowerShell
irm https://lucidlang.dev/install.ps1 | iex

# Linux
curl -fsSL https://lucidlang.dev/install.sh | sh
```

Once binaries exist, the installers will read the published release list, download over HTTPS, check each archive's SHA-256, and install into a user directory with no administrator rights.

## Building from source

The compiler is a Rust workspace, and it builds with `cargo`:

```sh
git clone https://github.com/redactedspec/lucid
cd lucid
rustup target add wasm32-unknown-unknown
cargo build --release
```

This writes `target/release/lucid`. The build needs:

- A current stable Rust (the 2024 edition; development uses 1.97), with the `wasm32-unknown-unknown` target, which the WebAssembly runtime needs.
- On Windows, Visual Studio Build Tools and the Windows SDK, since `lucid build` links with the MSVC linker. LLVM, clang, and gcc are not needed.
- On Linux, a C compiler as `cc`, such as gcc, since `lucid build` links through it.
- Node, only to run the compiler's own test suite with `cargo test`.

## Editor support

Point any editor with a language server client at the command `lucid lsp`, for files ending in `.lcd`. It syncs whole documents. No editor extension ships yet, so VS Code, which needs one, has no support. In Neovim 0.11 or later:

```lua
vim.filetype.add({ extension = { lcd = 'lucid' } })
vim.lsp.config('lucid', { cmd = { 'lucid', 'lsp' }, filetypes = { 'lucid' }, root_markers = { 'lucid.toml' } })
vim.lsp.enable('lucid')
```
