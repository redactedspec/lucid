# Lucid installer: reserved. No binaries are published yet, so this
# script downloads nothing and stops with an error. It throws rather
# than calling exit, so `irm ... | iex` does not close your shell.
# Status: https://lucidlang.dev/downloads/
throw 'Lucid has no published binaries yet. Nothing was installed. See https://lucidlang.dev/downloads/ for status.'
