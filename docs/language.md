# Language Reference

This document is normative. The formal syntax is in [grammar.md](grammar.md).

## Files and programs

Source files use the `.lcd` extension and UTF-8. Each file is one module.

The entry file may contain statements at top level. They run in order. Library modules hold declarations and constant `let` values only.

```lucid
print "Hello, world!"
```

## Layout

- Blocks are defined by indentation of exactly 4 spaces. A tab in indentation is an error.
- A statement ends at the end of its line.
- To continue over several lines, wrap the expression in `()`, `[]`, or `{}`. Newlines inside brackets are ignored. There is no other continuation rule.
- Comments start with `#`. Doc comments start with `##`.
- Names use ASCII letters, digits, and `_`. Text and comments may hold any Unicode, except NUL in text.

## Variables

```lucid
let name = "Ada"     # immutable
var count = 0        # mutable
count += 1
let limit: int = 10  # optional annotation
```

Every value must be initialized. Shadowing in the same scope is an error.

Tasks cannot read or write a top-level `var`. Pass it as a parameter. This keeps exclusivity checks static.

A task may read a top-level `let` whose value is a literal, with or without a leading `-`. A text with no interpolation is a literal. Such a value exists before any statement runs, so a task never sees it uninitialized. Reading any other top-level `let` reports E0901, see the open questions in `decisions.md`.

## Basic types

| Type | Meaning |
| --- | --- |
| `int` | 64-bit signed integer |
| `float` | 64-bit IEEE float |
| `bool` | `true` or `false` |
| `text` | Immutable UTF-8 string |
| `byte` | 8-bit unsigned integer |
| `i8` `i16` `i32` `i64` `u8` `u16` `u32` `u64` `f32` | Sized numbers |
| `list[T]` | Growable ordered list |
| `map[K, V]` | Hash map, keeps insertion order |
| `set[T]` | Hash set, keeps insertion order |
| `none` | Absence of a value |

There are no implicit conversions. Use `.to_float()`, `.to_int()`, and similar tasks.

Integer overflow traps in every build. Wrapping math uses explicit tasks such as `a.wrapping_add(b)`.

`/` always yields `float`, and follows IEEE 754, so `1 / 0` is `inf`. Two `int` operands are converted to `float` first.

`//` and `%` take `int` operands only. `//` rounds down, and `%` takes the sign of the divisor, so `(a // b) * b + a % b == a`. A zero divisor traps. A `float` operand is a compile error.

Arithmetic and comparison need one type on both sides. `==` and `!=` also work on `bool`. `and` and `or` skip their right side when the left side decides the result.

`x.to_float()` converts an `int`. `x.to_int()` rounds a `float` toward zero, and traps on NaN or a value outside `int`.

A number literal must fit its type, and a `float` literal that is not zero must not round to zero. A `-` directly before a number joins the literal, unless a call, index, or field follows the number. So `-9223372036854775808` is the lowest `int`, and `-1.5.to_int()` negates the result of the call.

A `byte` compares with `==`, `<`, and the like, prints as a number, and converts with `.to_int()`. For now `.bytes()` is the only source of one, and arithmetic on it reports E0901 until sized numbers arrive.

`print` takes one value. A finite `float` prints as a valid literal, with the fewest digits that read back as the same value, such as `3.0`, `0.1`, `1000.0`, or `1.0e21`. The others print as `inf`, `-inf`, and `NaN`. A `none` prints as `none`.

## Text

```lucid
let line = "Hi {name}, you are {age + 1}"
let brace = "use {{ for a literal brace"
let block = """
    Multi-line text.
    Common indentation is removed.
    """
```

Escapes are `\n`, `\t`, `\r`, `\\`, and `\"`. Interpolated expressions may contain text of their own. In `"""` text, the first line break, the common indentation, and a final line holding only the closing quotes are all removed. So `block` above has two lines and no trailing line break.

Interpolated values may be `text`, `int`, `float`, `bool`, or `none`. They read as `print` shows them. Interpolation is the only way to join text, so `+` does not work on `text`.

`==`, `!=`, `<`, `<=`, `>`, and `>=` work on `text`. Order is by Unicode code point.

Length, slicing, and iteration work on Unicode characters. A character is one code point. `x.len()` is the number of characters, and `x.is_empty()` tests for none.

```lucid
let word = "héllo"
print word[1..3]     # él
print word[1..=3]    # éll

for letter in word   # h, then é, and so on
    print letter
```

`x[a..b]` is a new text with the characters from `a` up to but not including `b`. `x[a..=b]` means `x[a..b + 1]`. Both ends are required. `a..b` traps unless `0 <= a <= b <= x.len()`. So `x[2..2]` and `x[2..=1]` are empty, as the same ranges are in `for`, and `x[3..2]` traps. A single index such as `x[0]` is a compile error: write `x[0..1]`.

`for` visits each character as a one-character `text`. There is no separate character type. The loop keeps the text it started with, so assigning to `x` in the body does not affect it. `for change` does not work on text, which cannot change.

A slice scans and copies, so its cost grows with the length of `x`. A `for` is linear in total, and is the better way to visit every character.

`x.bytes()` gives the UTF-8 bytes as a `list[byte]`.

## Collections

```lucid
let primes = [2, 3, 5]
let ages = {"Ada": 36, "Linus": 21}
let tags = set(["new", "sale"])
let first = primes[0]        # traps when out of bounds
let age = ages["Ada"]        # traps when the key is missing
```

Lists, maps, and sets are values. `var b = a` then changing `b` never affects `a`.

A list literal needs one element type. An empty `[]` or `{}` takes its type from where it is used: an annotation, a parameter, a return type, the target of an assignment, or an earlier element or `then` branch next to it. So `[[1], []]` works, and `[[], [1]]` is error E0314, as is any empty literal with no such type. An empty set is `set([])`, since `{}` is a map.

```lucid
var names: list[text] = []
names.push("Ada")            # add at the end
names[0] = "Grace"           # traps when out of bounds
names.swap(0, 0)             # traps when out of bounds

var counts = {"a": 1}
counts["b"] = 2              # add or replace
counts["a"] += 1             # traps when the key is missing
counts.remove("b")           # does nothing when the key is missing

var seen = set([1, 2, 2])    # repeats are dropped
seen.add(3)
seen.remove(1)
```

An index is an `int` from 0, and a negative index traps. A map or set has `int`, `bool`, `byte`, or `text` keys. A `float` cannot be a key, because NaN does not equal itself.

All three have `.len()` and `.is_empty()`. `x in items` and `x not in items` test a list, or the keys of a map or set. A list test is linear, and works for the same types as keys.

A map or set keeps insertion order. Replacing a value keeps its key's place, and a key that is removed and added again goes last. In a map literal, a repeated key replaces the value before it, and keeps its place.

`for` visits a list's elements, a set's values, or a map's keys. It keeps the value it started with, so changing the variable in the body does not affect the loop. `for change` over a list is planned with parameter modes, and it does not work on a map or set.

`push`, `swap`, `add`, and `remove` change their receiver and give no value, so each one is a statement. The receiver is a `var` or a path into one, such as `grid[0].push(1)`. In `x[i] = f()`, the index runs first, and then the value. Each index in a path runs once.

`x.get(i)` on a list and `x.get(key)` on a map read like `[]`, and give `none` in place of a trap. So they return `T?`, and a negative index gives `none`. If a map's values may be `none`, the two cases look alike: `in` tells them apart.

`x.pop()` removes the last element of a list and returns it, or returns `none` for an empty list. It changes its list while giving a value, so it must be a whole statement, or all of the value of `let`, `var`, `return`, or an assignment to a name. `f(x.pop())` is a compile error: write `let last = x.pop()` first.

```lucid
let first = primes.get(0)    # int?
var stack = [1, 2]
var top = stack.pop()        # int?
while top is int
    print top
    top = stack.pop()
```

`x[a..b]` on a list is a new list with the elements from `a` up to but not including `b`, and `x[a..=b]` means `x[a..b + 1]`. Both ends are required, and a range out of bounds traps, by the same rules as text. A slice copies, so it cannot be written to.

Planned: tasks that take a lambda such as `.map`. Printing a collection and `==` on one wait for the derived `Show` and `Equal`. Collections as keys wait for `Hash`.

## Control flow

```lucid
if score >= 90
    print "A"
else if score >= 80
    print "B"
else
    print "C"

while count < 3
    count += 1

for i in 0..10       # 0 to 9
    print i

for i in 0..=10      # 0 to 10
    print i

let label = if n == 1 then "item" else "items"
```

Conditions must be `bool`. There is no truthiness.

A `for` over a range reads its bounds once. The loop name cannot be assigned, and `0..=x` ends cleanly even when `x` is the highest `int`.

## Tasks

A `task` is a function. Parameters always have types. With no `->`, a task returns `none`. Return types are never inferred, so a signature is always complete. A task with a return type must return on every path. `return` is only allowed in a task.

Tasks may be used before their line. A parameter cannot be assigned.

```lucid
task add(a: int, b: int) -> int
    return a + b

task connect(host: text, port: int = 443, secure: bool = true)
    ...
```

Arguments are positional. A parameter with a default must be passed by name, and only such a parameter can be. Parameters with defaults come last. Named arguments follow positional ones, in parameter order, so arguments always run in the order written. A default is a literal, which includes `none`.

```lucid
connect("example.com", port: 8080)
```

Lambdas use `=>` and hold a single expression. For anything longer, define a nested task.

```lucid
let doubled = primes.map(x => x * 2)
```

Task types are written `task(int, int) -> int`.

`...` is a placeholder statement that traps at run time. `lucid build --release` rejects it.

## Parameter modes

Parameters are read-only views by default. Two keywords change this. See [memory-model.md](memory-model.md).

```lucid
task total(items: list[int]) -> int        # read only
task sort(change items: list[int])         # mutates caller's value
task consume(take items: list[int])        # owns the value, may mutate it

sort(change numbers)     # `change` is required at the call
```

## Types

A `type` holds named fields. Constructor arguments are always named.

```lucid
type User
    name: text
    age: int = 0

let ada = User(name: "Ada", age: 36)
print ada.name
```

Arguments may come in any order, and run in the order written. A field with no default must be given. A default is a literal.

Copying is by value: `var b = a` then changing `b` never affects `a`. A field of a `var` can change, as in `b.age += 1` or `team.lead.name = "Ada"`. Each index in such a path runs once, before the value.

An alias uses `=`. It is another name for the type, not a new type:

```lucid
type UserId = int
```

Types may refer to each other in any order. A type that always holds another value of itself has no finite value, and is error E0315. So `next: Node` needs a list, a union such as `Node?`, or a `choice` with a variant that ends the chain.

Planned: printing a value and `==` on one wait for the derived `Show` and `Equal`. A value as a key waits for `Hash`.

## Methods

A task with first parameter `self` may be called with a dot. There is no `impl` block or class.

```lucid
task is_adult(self: User) -> bool
    return self.age >= 18

task birthday(change self: User)
    self.age += 1

if ada.is_adult()
    print "welcome"
```

`x.f(a)` means `f(x, a)`. The receiver must be a `var` when `self` is `change`. The `change` keyword is not repeated on a dot call.

## Choices

A `choice` is a closed set of variants.

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
```

A variant with fields is built like a `type`, with named arguments, as in `Circle(radius: 2.0)`. A variant with no fields is written with no parentheses, as in `Point`. A value of a `choice` has no fields of its own: `match` reaches them.

Types, choices, variants, and tasks share one namespace, so two choices cannot use the same variant name.

`match` must cover every case, and an arm that can never run is an error. Arms are tried in order. `_` matches anything.

- **Variant:** `Rect(w, h)` binds every field, in the order written in the `choice`. `_` skips one. A variant with no fields is `Point`. On a `choice`, a name alone must be one of its variants, so a misspelled variant is an error and not a name that catches everything.
- **Literal:** an `int` or `float`, with or without a leading `-`, a `text`, or a `bool`. A text pattern cannot hold `{}`. To cover `int` or `text`, end with `_` or a name.
- **Name:** on any other type, `n -> print n` matches anything and binds the value.

Patterns do not nest. A name bound by a pattern cannot be a type or variant name, so `Cons(h, Nil)` is an error and not a test of the tail.

On a union, an arm tests what the value holds:

- **`none`:** matches `none`.
- **Typed:** `n: int` matches that member and binds it. `v: int or text` matches either one.
- **Name:** `n` matches what earlier arms left, and binds it with that type.

A union takes no literal or variant arm: match the member first. A typed arm on anything but a union is an error.

The subject runs once. An arm may assign the variable it came from, and its names keep the values they bound. As a statement, an arm can be an expression, `print`, `return`, or a block, and its value is dropped. As the value of `let`, `var`, or `return`, each arm is one expression of one type, or a `return`.

## Union types and optionals

`A or B` is a value of either type, like TypeScript. `T?` is shorthand for `T or none`. Unions flatten, so `(A or B) or A` is `A or B`.

```lucid
task find(name: text) -> User?
    ...

let user = find("Ada")
if user is not none
    print user.name      # narrowed to User
```

`is` tests type, `none`, or `empty`, and narrows the value in that branch. It never tests equality. Use `==` for equality. `x is empty` calls `x.is_empty()`. A union target tests each member, so `x is (int or text)` and `x is int?` work.

A value of a member type, or of a smaller union, can go wherever a union is expected. A union literal needs that expected type: `let x: list[int?] = [1, none]` works, and `[1, none]` alone is an error. There is no conversion between collections, so a `list[int]` is not a `list[int?]`.

A value of type `T?` needs `return none`: a bare `return` is for a task that returns `none`.

Every member of an `is` target or typed arm must be one the value can hold, and the test must be able to give both answers. So `x is none` on an `int`, `x is text` on an `int?`, and `x is (int or none)` on an `int?` are errors.

A name narrows where the test is known to hold:

```lucid
task label(name: text?) -> text
    if name is none
        return "nobody"
    return name              # narrowed, since the branch above returned

print name is not none and name.len() > 0
let size = if name is none then 0 else name.len()

while node is not none
    print node.value
    node = node.next         # `next` has type Node?
```

- **Conditions:** `if`, `else if`, `while`, `and`, `or`, `not`, and `then`.
- **After an `if`:** when each `if` and `else if` branch ends with `return`, `break`, `continue`, or `...`, the code after it knows that every condition was false. An `else` adds nothing to this, and it need not end.
- **Assignment:** assigning to a name ends what was known about it, on all later code. It gives no new knowledge, so after `x = 1`, a name of type `int?` needs another test.
- **Loops:** a loop forgets anything about a name assigned in its body, before it starts.
- **Writes:** `x.age += 1` and `x.push(1)` on a narrowed `x` change the value it holds, and keep the narrowing.

Only a local name or parameter narrows. A field or a call may not hold the same value when read again, so `if a.b is not none` does not narrow `a.b`. Write `let b = a.b`, then test `b`. The same goes for a top-level `let` read in a task.

Planned: showing a union and `==` on one wait for the derived `Show` and `Equal`. A union as a key waits for `Hash`, and `in` on a list of unions waits with them.

## Errors

There are no exceptions. A task that can fail returns a union that includes an error type. A type is an error type when it satisfies the `Error` trait, which requires `task error_message(self) -> text`.

```lucid
choice ConfigError
    Missing(path: text)
    Invalid(line: int)

task error_message(self: ConfigError) -> text
    return match self
        Missing(path) -> "missing file: {path}"
        Invalid(line) -> "bad config at line {line}"

task load(path: text) -> Config or ConfigError or FileError
    let source = try read_text(path)      # returns the error to the caller
    return parse(source)

let config = load("app.toml") else default_config()   # fallback value

match load("app.toml")
    c: Config -> run(c)
    e: ConfigError -> print e.error_message()
    e: FileError -> print e.error_message()
```

- `try x` and `x else y` act on the error types in the union of `x`. If the union has no error type, they act on `none`. So `none` from a `none or FileError` task counts as success.
- `try x` unwraps the value, or returns the error from the current task. The return type must accept it. At top level of an entry file, it prints the error and exits with code 1.
- `x else y` gives `y` in place of the error.
- An unused error is a compile error.

A **trap** is an unrecoverable bug: out-of-bounds index or slice, a missing map key, overflow, a zero divisor, a failed conversion, stack overflow, a reached `...`, failed `expect`, a `shared` access conflict, running out of memory, or a failed write to stdout. A trap prints its source position and exits with code 101, except that running out of memory and a failed write to stdout have no position. A stack trace is planned for M5. Traps cannot be caught.

## Generics

```lucid
task first[T](items: list[T]) -> T?
    return items.get(0)

type Pair[A, B]
    left: A
    right: B
```

Generics are monomorphized: each used combination gets its own compiled code.

Planned: brackets give type arguments at a call, as in `f[int?](x)` or `Pair[int, text](left: 1, right: "a")`, and they hold any type.

## Traits

A trait lists required tasks. Satisfaction is structural: any type with matching tasks qualifies, with no declaration.

```lucid
trait Describe
    task describe(self) -> text

task show_all[T: Describe](items: list[T])
    for item in items
        print item.describe()
```

Bounds use static dispatch. `any Describe` is a boxed value with dynamic dispatch, which allows mixed lists.

The compiler derives `Equal`, `Hash`, `Order`, `Show`, `Copy`, and `Json` for a type when all its fields support them. Writing the task by hand overrides the derived version. There are no macros.

Operators are defined by the traits `Add`, `Sub`, `Mul`, `Div`, `Equal`, and `Order`. No other operators can be overloaded, and no new operators can be defined.

## Modules

```lucid
use std.http
use std.json as js
use ./models
use ./models.User
```

Names are private to their module unless marked `public`. Import cycles are an error.

## Concurrency

`spawn` starts a task. Every spawn is owned by the enclosing block, which waits for it before exiting. Nothing outlives its parent, so no task leaks.

```lucid
let a = spawn fetch(url_a)
let b = spawn fetch(url_b)
print a.wait()
print b.wait()
```

Arguments move or copy into the new task. Safe code cannot share mutable data, so it cannot have data races. To communicate, use `channel[T]`, or `lock[T]` for shared mutable state.

There is no `async` or `await`. Tasks do not have colors: any task can block and any task can be spawned.

## Shared references

Values are independent by default. For graphs, caches, and other shared structures, use `shared`. See [memory-model.md](memory-model.md).

```lucid
type TreeNode
    value: int
    children: list[shared TreeNode]
    parent: weak TreeNode?

var root = shared TreeNode(value: 1, children: [], parent: none)
var leaf = shared TreeNode(value: 2, children: [], parent: weak root)
```

## Tests

Tests live next to the code and are removed from normal builds.

```lucid
test "adds numbers"
    expect add(2, 3) == 5
```

`expect` traps with both values printed when it fails, and it also works outside tests.

## Foreign code

```lucid
unsafe extern task strlen(s: pointer[byte]) -> u64

unsafe
    let n = strlen(raw)
```

`unsafe` blocks and `extern` declarations need the `unsafe` package permission. See [security.md](security.md).

## Keywords

`and` `any` `as` `break` `change` `choice` `continue` `else` `empty` `expect` `extern` `false` `for` `if` `in` `is` `let` `match` `none` `not` `or` `print` `public` `return` `self` `shared` `spawn` `take` `task` `test` `then` `trait` `true` `try` `type` `unsafe` `use` `var` `weak` `while`

Adding a keyword is a breaking change and needs a decision entry.
