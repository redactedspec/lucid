---
title: Error handling
description: Errors as values, the Error trait, try, else, match, and traps.
---

There are no exceptions. A task that can fail says so in its return type, as a union with an error type. An error that nothing uses does not compile.

## Error types

A type is an error type when it has the `Error` trait, which needs one task: `error_message(self) -> text`.

```lucid
choice ConfigError
    Missing(path: text)
    Invalid(line: int)

task error_message(self: ConfigError) -> text
    return match self
        Missing(path) -> "missing file: {path}"
        Invalid(line) -> "bad config at line {line}"

task parse_port(raw: text) -> int or ConfigError
    let port = raw.to_int()
    if port is none
        return Invalid(line: 1)
    return port

print parse_port("8080") else 80
print parse_port("eighty") else 80
```

## try

`try x` gives the value, or returns the error from the current task. The task's return type must accept the error.

```lucid
use std.fs

task copy(from: text, to: text) -> none or fs.FileError
    let content = try fs.read_text(from)
    try fs.write_text(to, content)

copy("missing.txt", "out.txt") else none
```

At the top level of a program, a failed `try` prints the error's message and exits with code 1.

## else

`x else y` gives `y` in place of an error, and runs `y` only then:

```lucid
use std.fs

let text = fs.read_text("settings.txt") else "default"
print text
```

To ignore an error on purpose, say so: `save() else none`.

## match

To handle each case, match on the type:

```lucid
use std.fs

match fs.read_text("notes.txt")
    t: text -> print t
    e: fs.FileError -> print e.error_message()
```

## none

When a union has no error type, `try` and `else` act on `none` instead:

```lucid
let port = "80x".to_int() else 8080
print port
```

## Traps

A trap is a bug that cannot be recovered: an index out of bounds, a missing map key, integer overflow, a zero divisor, a failed conversion, a failed `expect`, or a stack overflow. A trap prints its source position and a stack trace, and exits with code 101. Traps cannot be caught.

```lucid title="main.lcd"
task third(items: list[int]) -> int
    return items[2]

print third([1, 2])
```

```text
main.lcd:2:12: trap: list index out of bounds
    at main.lcd:4:7 in top level
```

So recoverable failures are values in the type, and bugs stop the program at the line that caused them.
