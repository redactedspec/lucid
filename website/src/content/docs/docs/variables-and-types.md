---
title: Variables and types
description: let and var, the basic types, numbers, conversions, and text.
---

## Variables

```lucid
let name = "Ada"     # cannot change
var count = 0        # can change
count += 1
let limit: int = 10  # an optional annotation
print "{name} {count} {limit}"
```

Every value must be initialized. Declaring the same name twice in one scope is an error. Local types are inferred, so annotations are optional.

A task cannot read or write a top-level `var`: pass it as a parameter. A task may read a top-level `let` whose value is a literal.

## Basic types

| Type | Meaning |
| --- | --- |
| `int` | 64-bit signed integer |
| `float` | 64-bit IEEE float |
| `bool` | `true` or `false` |
| `text` | Immutable UTF-8 string |
| `byte` | 8-bit unsigned integer |
| `i8` `i16` `i32` `u16` `u32` `u64` `f32` | Sized numbers |
| `list[T]`, `map[K, V]`, `set[T]` | Collections, see [Collections](/docs/collections/) |
| `none` | The absence of a value |

## Numbers

There are no implicit conversions. Each number type has a task for every other one, named after its target:

```lucid
let n = 7
let x = n.to_float() / 2.0
let small = 200.to_u8()
print x               # 3.5
print small           # 200
print 2.0.sqrt()
```

- `/` always gives a float type, so `7 / 2` is `3.5`. `//` divides integers and rounds down, and `%` takes the sign of the divisor.
- Integer overflow traps in every build. For wrapping math, call it out: `a.wrapping_add(b)`, `wrapping_sub`, and `wrapping_mul`.
- A conversion to an integer type that does not fit traps.
- Bit operators `&`, `|`, `^`, `~`, `<<`, and `>>` work on every integer type.
- A number literal takes a sized type from its context, as in `let b: u8 = 200`.

```lucid
let a: u8 = 250
let b = a + 5        # u8, since a is
print b
print 17 // 5        # 3
print -7 % 3         # 2
```

## Text

```lucid
let name = "Ada"
let age = 36
print "Hi {name}, next year you are {age + 1}"
print "use {{ for a literal brace"

let block = """
    Multi-line text.
    Common indentation is removed.
    """
print block
```

Interpolation is the only way to join text, so `+` does not work on `text`. Any value can be interpolated, and reads as `print` shows it. Escapes are `\n`, `\t`, `\r`, `\\`, and `\"`.

Length, slicing, and iteration count Unicode characters:

```lucid
let word = "héllo"
print word.len()          # 5
print word[1..3]          # él
for letter in word
    print letter
```

A single index such as `word[0]` is a compile error: write `word[0..1]`. There is no separate character type.

Text also has these tasks:

```lucid
print "a,b,,c".split(",")        # ["a", "b", "", "c"]
print "Straße".upper()           # STRASSE
print "a.lcd".ends_with(".lcd")  # true, and starts_with too
print "héllo".contains("ll")     # true
print "héllo".find("l")          # 2, an int? that is none if absent
print "  hi  ".trim()            # hi
print "-42".to_int()             # -42, an int? that is none if invalid
print "abc".bytes()              # the UTF-8 bytes, a list[byte]
```

`==`, `<`, and the other comparisons work on text, in Unicode code point order.
