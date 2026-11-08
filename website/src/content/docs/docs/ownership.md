---
title: Ownership and memory
description: How Lucid stays memory safe with no garbage collector and no lifetime annotations.
---

Lucid infers ownership. You write no lifetimes, and there is no garbage collector. It combines three known ideas: mutable value semantics, as in Swift and Hylo; compile-time reference counting, as in Koka and Lean; and the law of exclusivity, as in Swift and Rust.

## The rules

1. **Every value is independent.** `var b = a` gives `b` its own logical value. Changing `b` never changes `a`.
2. **There are no stored references.** A field, element, or variable holds a value, not a pointer to one. Nothing can dangle.
3. **Parameters are read-only views by default.** Passing a value never copies it.
4. **`change` gives exclusive access.** The task may change the caller's variable in place, and nothing else can touch it during the call.
5. **`take` gives the task its own value**, which it may change. The caller writes nothing at the call, and keeps its own value.

```lucid
task add_all(take items: list[int], extra: int) -> list[int]
    items.push(extra)
    return items

let numbers = [1, 2, 3]
let more = add_all(numbers, 4)   # last use of numbers: moved, changed in place
print more
```

## What the compiler does

Logical copies are almost never real copies.

| Code | What happens |
| --- | --- |
| A value used for the last time | Moved, at no cost |
| A heap value used again later | Its count goes up, and the buffer is shared |
| A write to a buffer held once | In place |
| A write to a shared buffer | One copy, then in place |

The program is correct either way. Only its cost changes, and `lucid check --costs` lists each copy and count site:

```sh
lucid check --costs main.lcd
```

## Exclusivity

While a value is being changed, nothing else may use it. The compiler rejects each of these with error E0326:

```text
swap(change a, change a)      # two exclusive uses of a
f(change a, a)                # a is read while it changes

for item in items
    items.push(item)          # items changes while it is visited
```

So no code can observe a value halfway through a change, and no iterator can be invalidated.

## Threads

A value sent to another thread is a copy, and threads share state only through a `channel` or a `lock`. So safe code has no data races. See [Concurrency](/docs/concurrency/).

## Limits

- Heavily shared data pays for reference counting, which hand-tuned Rust can avoid.
- A reference cycle needs `shared` and `weak`, which parse but are not built yet. Without them, a design that needs a graph uses indexes into a list.
- Every `type` and `choice` value is a counted heap record for now. Storing small types inline is not planned yet.
