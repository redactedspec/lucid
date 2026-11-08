---
title: Syntax
description: Lucid's layout rules, statements, comments, names, and keywords.
---

## Files

Source files use the `.lcd` extension and UTF-8. Each file is one module. The entry file may hold statements at the top level, and they run in order.

```lucid
print "Hello, world!"
```

## Layout

- A block is indented by exactly four spaces. A tab in indentation is an error.
- A statement ends at the end of its line. There are no semicolons.
- To continue an expression over several lines, wrap it in `()`, `[]`, or `{}`. Line breaks inside brackets are ignored. There is no other continuation rule.
- A block starts after a line such as `if`, `for`, or `task`, with no colon.

```lucid
let primes = [
    2, 3, 5,
    7, 11, 13,
]

for p in primes
    if p > 5
        print p
```

## Comments

Comments start with `#`. Doc comments start with `##`, sit above a declaration, and show in `lucid context`.

```lucid
## The area of a circle.
task area(radius: float) -> float
    return 3.14159 * radius * radius   # pi times r squared

print area(2.0)
```

## Names

Names use ASCII letters, digits, and `_`. By convention, types and variants are `UpperCase`, and everything else is `snake_case`. Text and comments may hold any Unicode.

## One way to write it

Lucid gives each common job one form, and `lucid fmt` gives every file one layout, with no options. Code from different people and agents looks the same, and a diff shows only meaning.

There are no macros, no exceptions, no inheritance, no implicit conversions, and no null. Each of these would hide what a line does.

## Keywords

`and` `any` `as` `break` `change` `choice` `continue` `else` `empty` `expect` `extern` `false` `for` `if` `in` `is` `let` `match` `none` `not` `or` `print` `public` `return` `self` `shared` `spawn` `take` `task` `test` `then` `trait` `true` `try` `type` `unsafe` `use` `var` `weak` `while`

`shared` and `weak` parse, but do not run yet.
