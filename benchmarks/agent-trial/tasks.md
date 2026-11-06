# Agent Trial Tasks

Each task is one Lucid program with no input. Data is written in the program. Output must match exactly. Use the task, type, and field names given.

## 01 Word frequency

For the text `the quick brown fox jumps over the lazy dog the fox`, count each word. Words are split on single spaces. Print one line per word in the form `word: count`, by count from high to low, and alphabetically within a count.

## 02 Text tools

Write `caesar(s: text, shift: int) -> text`. It moves each ASCII letter forward by `shift` in the alphabet, wrapping around and keeping its case. Other characters stay. `shift` can be negative.

Write `is_palindrome(s: text) -> bool`. It ignores case, and looks only at ASCII letters.

Print, one per line: `caesar("Attack at Dawn!", 3)`, then that result passed through `caesar` with `-3`, then `is_palindrome("A man, a plan, a canal: Panama")`, then `is_palindrome("Hello")`.

## 03 RPN calculator

Write `eval_rpn(source: text) -> int or EvalError`, where `EvalError` is a `choice` with these variants: `Underflow`, `Leftover(count: int)`, `BadToken(token: text)`. Tokens are split on spaces. A token is a run of digits, or one of `+`, `-`, `*`, `/`, where `/` is floor division. The first problem found stops the evaluation. `Underflow` means an operator found too few values. `Leftover` means that the end was reached with more than one value on the stack, and holds how many.

For each of `3 4 + 2 *`, `100 7 / 2 -`, `1 +`, `1 2`, `1 x +`, print the input, then ` => `, then the result. A result prints as the number, or as `underflow`, `leftover 2`, or `bad token x`.

## 04 Inventory

Write a `type Item` with `name: text`, `price: float`, `qty: int`, and a `type Inventory` holding a list of items.

Write these, each changing an inventory:

- `restock(name: text, n: int)` adds to an item's `qty`. An unknown name adds an item with price `1.0`.
- `sell(name: text, n: int) -> bool` lowers `qty` and gives `true`. It gives `false` and changes nothing if the item is unknown or has too little.
- `discount(percent: float)` multiplies every price by `1.0 - percent / 100.0`.
- `drop_empty()` removes items with `qty` 0.

And `total(...) -> float`, the sum of `price * qty`.

Start with apple 0.5 qty 10, pear 0.75 qty 4, kiwi 2.0 qty 1. Then run, in order: sell apple 3, sell pear 5, sell kiwi 1, sell plum 1, restock plum 2, discount 10, drop_empty. Print each `sell` result as it happens, then each item as `name price qty` with spaces, then the total.

## 05 Shapes

Write a `choice Shape` with `Circle(radius: float)`, `Rect(width: float, height: float)`, and `Triangle(base: float, height: float)`. Use `3.0` for pi. Write `area`, and `name` that gives `circle`, `rect`, or `triangle`.

For Circle 2.0, Rect 2.0 by 3.0, Triangle 4.0 by 5.0, Rect 1.0 by 1.0, Circle 1.0:

1. Print each shape as `name area`, by area from high to low. No two areas are equal.
2. Print `total area`, a space, and the sum.
3. Print each name with its count as `name: count`, in order of first appearance in the list above.

## 06 Binary search tree

Write a `type Node` with `value: int`, `left: Node?`, `right: Node?`. Write these, working on a `Node?`: `insert` gives a tree with the value added, and a value already there changes nothing. `contains` gives a `bool`. `in_order` gives a sorted `list[int]`. `height` gives an `int`, and an empty tree has height 0. `smallest` gives an `int?`.

Insert 50, 30, 70, 20, 40, 60, 80, 30 in order. Print `in_order`, then `height`, then `contains` of 60, then `contains` of 65, then `smallest`, then `smallest` of an empty tree.

## 07 Generic containers

Write a generic `type Stack[T]` with tasks `push`, `pop` that gives `T?`, `peek` that gives `T?`, and `size`. Write a generic `type Queue[T]` built from two `Stack[T]` values, with `enqueue` and `dequeue`, where `dequeue` gives `T?`.

Write generic `map`, `filter`, and `reduce(items, start, f)`.

With a queue of `int`: enqueue 1, 2, 3, dequeue and print, enqueue 4, then dequeue and print until `none`, printing that `none` too. With a stack of `text`: push `a`, `b`, print `size`, `peek`, `pop`, `pop`, `pop`.

Print `map` of `[1, 2, 3]` with squares, `filter` of `[1, 2, 3, 4]` for even values, `reduce` of `[1, 2, 3]` from 0 with sum, and `map` of `["a", "bb"]` to lengths.

## 08 Event bus

An event bus maps an event name to a list of handlers. A handler takes an `int` and gives an `int`. Firing an event passes a value through its handlers in order, and gives the final value. An unknown event gives the value unchanged. The bus keeps a count of all handlers run so far, over all fires.

Write `compose(f, g)`, which gives a task that runs `f` and then `g`. Write `make_adder(n)` and `make_multiplier(n)`, which each give a task.

Register, for `double`, a multiplier of 2. For `calc`, an adder of 3, then a multiplier of 2, then `compose` of an adder of 1 with a multiplier of 10. Fire `calc` with 1, `double` with 21, `missing` with 5, and `calc` with 0, printing each result. Then print the count.

## 09 Traits

Write a `type Money` with `cents: int`, so that `print` shows `$1.05` for 105 cents, with two digits after the point. `+` and `<` work on it.

Write a `type Point` with `x: int`, `y: int`. Equal points are the same key in a `set`.

Write a trait `Describe`, with `describe(self) -> text`. `Money` and `Point` have it, and a `Point` describes as `point 1,2`, and a `Money` as `money $1.05`.

Write generic `largest(items)`, which gives the largest item of a list or `none` when it is empty, and `sort(items)` that puts a list in order.

Print `Money` 105 plus `Money` 200. Print `largest` of Money 5, 300, 20. Print `largest` of an empty list of `Money`. Print `sort` of Money 300, 5, 20 as a list, one item per line. Print the size of a set holding points (1,2), (1,2), (2,1). Then print `describe` for a mixed list of Point (1,2) and Money 105.

## 10 Checksums

Over the bytes of the UTF-8 text `héllo wörld`:

- `djb2`, a `u32`: start at 5381. For each byte, set `h` to `h * 33 + byte`, wrapping at 32 bits.
- `adler32`, a `u32`: start with `a = 1`, `b = 0`. For each byte, `a = (a + byte) % 65521` and `b = (b + a) % 65521`. The result is `b * 65536 + a`.
- `checksum8`, a `byte`: the sum of all bytes, wrapping at 8 bits.

Print each one on its own line as eight lowercase hex digits with leading zeros, except `checksum8` with two digits. Write `hex(value: u32, digits: int) -> text`.
