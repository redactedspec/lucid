---
title: Collections
description: Lists, maps, and sets in Lucid, which are values that never alias.
---

```lucid
let primes = [2, 3, 5]
let ages = {"Ada": 36, "Linus": 21}
let tags = set(["new", "sale"])
print primes[0]        # traps when out of bounds
print ages["Ada"]      # traps when the key is missing
print "new" in tags
```

## Values, not references

Lists, maps, and sets are values. After `var b = a`, changing `b` never affects `a`. The compiler shares the buffer until one of them is written, so the copy costs nothing until then.

```lucid
var a = [1, 2, 3]
var b = a
b.push(4)
print a        # [1, 2, 3]
print b        # [1, 2, 3, 4]
```

## Lists

```lucid
var names: list[text] = []
names.push("Grace")          # add at the end
names.push("Ada")
names[0] = "Linus"           # traps when out of bounds
names.swap(0, 1)
names.sort()                 # stable, smallest first
print names
print names.len()
print names[0..1]            # a slice is a new list

var stack = [1, 2]
var top = stack.pop()        # int?, none when empty
while top is int
    print top
    top = stack.pop()
```

An empty `[]` takes its type from where it is used, such as an annotation or a parameter. `x.get(i)` reads like `x[i]`, but gives `none` in place of a trap.

## Maps

A map keeps insertion order.

```lucid
var counts = {"a": 1}
counts["b"] = 2              # add or replace
counts["a"] += 1             # traps when the key is missing
counts.remove("b")           # does nothing when the key is missing
print counts
print counts.get("z")        # none
for key in counts
    print "{key} = {counts[key]}"
```

A key needs the `Hash` trait: an integer, `bool`, `text`, or a value made of those. A `float` cannot be a key, since NaN does not equal itself.

## Sets

```lucid
var seen = set([1, 2, 2])    # repeats are dropped
seen.add(3)
seen.remove(1)
print seen                   # set([2, 3])
print 2 in seen
```

An empty set is `set([])`, since `{}` is a map.

## Changing while visiting

The body of a `for` can read what it visits, but not change it. `for change` visits each element of a list in place:

```lucid
var prices = [10, 20, 30]
for change p in prices
    p += 1
print prices      # [11, 21, 31]
```

Inside `for change`, the body cannot use the list itself. The compiler rejects code that could see a value while it changes, see [Ownership and memory](/docs/ownership/).

## All built-in tasks

Lists have `len`, `is_empty`, `get`, `push`, `pop`, `swap`, and `sort`. Maps have `len`, `is_empty`, `get`, and `remove`. Sets have `len`, `is_empty`, `add`, and `remove`. `in` and `not in` test membership. Tasks such as `map`, `filter`, `sum`, and `join` are not built in yet: write them as tasks, as shown in [Tasks](/docs/tasks/).
