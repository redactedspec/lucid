---
title: Control flow
description: if, while, for, ranges, break, continue, and then expressions.
---

## if

```lucid
let score = 85

if score >= 90
    print "A"
else if score >= 80
    print "B"
else
    print "C"
```

A condition must be a `bool`. There is no truthiness, so `if count` is an error: write `if count > 0`.

## then expressions

`if ... then ... else` is an expression, for choosing a value:

```lucid
let n = 3
let label = if n == 1 then "item" else "items"
print "{n} {label}"
```

## while

```lucid
var count = 0
while count < 3
    count += 1
print count
```

## for

`for` visits a range, a text's characters, a list's elements, a set's values, or a map's keys.

```lucid
for i in 0..3        # 0, 1, 2
    print i

for i in 0..=3       # 0, 1, 2, 3
    print i

for name in ["Ada", "Linus"]
    print name
```

A range reads its bounds once, and the loop name cannot be assigned. The body of a `for` can read what it visits, but cannot change it. To change each element of a list, use `for change`, see [Collections](/docs/collections/).

## break and continue

```lucid
for i in 0..10
    if i % 2 == 0
        continue
    if i > 7
        break
    print i
```

## Logic

`and`, `or`, and `not` work on `bool`. `and` and `or` skip their right side when the left side decides the result. Comparisons do not chain: `a < b < c` is an error.

```lucid
let age = 20
let member = true
if age >= 18 and not member
    print "guest"
else if age >= 18 and member
    print "member"
```

Matching on values is covered in [Types, choices, and unions](/docs/types-and-choices/).
