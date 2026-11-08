---
title: Types, choices, and unions
description: Record types, closed choices with match, union types, optionals, and narrowing.
---

## Types

A `type` holds named fields. Constructor arguments are always named, in any order. A field with a default may be left out.

```lucid
type User
    name: text
    age: int = 0

var ada = User(name: "Ada", age: 36)
ada.age += 1
print ada.name
print ada        # User(name: "Ada", age: 37)
```

A `type` is a value, like a list. `==` compares every field, and `print` shows every field. An alias uses `=`, and is another name for the same type: `type UserId = int`.

## Choices

A `choice` is a closed set of variants. A variant may hold fields.

```lucid
choice Shape
    Circle(radius: float)
    Square(side: float)
    Point

task area(self: Shape) -> float
    return match self
        Circle(radius) -> 3.14159 * radius * radius
        Square(side) -> side * side
        Point -> 0.0

let shapes: list[Shape] = [Circle(radius: 1.0), Square(side: 2.0), Point]
for s in shapes
    print s.area()
```

## match

`match` must cover every case, and an arm that can never run is an error. Arms are tried in order, and `_` matches anything.

```lucid
task describe(n: int) -> text
    return match n
        0 -> "zero"
        1 -> "one"
        _ -> "many"

let command = "add"
match command
    "add" -> print "adding"
    "list" -> print "listing"
    _ -> print "unknown"

print describe(5)
```

A pattern binds each field of a variant in order, or matches a literal. Patterns do not nest.

## Unions and optionals

`A or B` is a value of either type. `T?` is short for `T or none`. There is no null: absence is `none`, and the type says where it can appear.

```lucid
type Node
    value: int
    next: Node?

task label(name: text?) -> text
    if name is none
        return "nobody"
    return name              # narrowed to text, since the branch returned

print label(none)
print label("Ada")

var node: Node? = Node(value: 1, next: Node(value: 2, next: none))
while node is not none
    print node.value         # narrowed to Node
    node = node.next
```

`is` tests a type, `none`, or `empty`, and narrows the name where the test holds. It never tests equality. Only a local name or parameter narrows, since a field may change between reads: copy it with `let` first.

A union can also be matched by type:

```lucid
task show(value: int or text) -> text
    return match value
        n: int -> "number {n}"
        t: text -> "text {t}"

print show(4)
print show("four")
```
