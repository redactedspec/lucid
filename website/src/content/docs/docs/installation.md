---
title: Installation
description: Install Lucid 1.0.0 on Windows or Linux, or build it from source.
---

## Install

Lucid 1.0.0 is released for Windows and Linux on x86-64. One command installs it into your user directory:

```sh
# Windows, PowerShell
irm https://lucidlang.dev/install.ps1 | iex

# Linux, glibc 2.39 or newer
curl -fsSL https://lucidlang.dev/install.sh | sh
```

The script reads the release list at [`lucidlang.dev/releases.txt`](/releases.txt), downloads the archive over HTTPS, and checks its SHA-256 before installing. `lucid` goes in `~/.lucid/bin` on Linux, or `%LOCALAPPDATA%\Lucid\bin` on Windows. The script changes no shell profile or environment variable; it prints the line that puts `lucid` on your `PATH`. Run it again to update.

To install somewhere else, set `LUCID_HOME` first. To install by hand, take an archive from [Downloads](/downloads/), check it against `SHA256SUMS` from the [release](https://github.com/redactedspec/lucid/releases/tag/v1.0.0), and put the binary anywhere on your `PATH`. It carries the standard library and the runtime, so it needs no other file.

`lucid run`, `check`, and `test` need nothing else. `lucid build` needs a linker: on Windows, Visual Studio Build Tools with the Windows SDK; on Linux, a C compiler as `cc`, such as gcc.

## Platforms

| Platform | State |
| --- | --- |
| Windows x86-64 | Released: `run`, `test`, `check`, `build`, and `build --target web` |
| Linux x86-64 | Released, for glibc 2.39 or newer. On an older glibc, build from source |
| Browser and Node | An output target: `lucid build --target web` |
| macOS | Deferred, with no milestone |

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
