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

`/` always yields `float`. `//` is floor division and yields `int`. `%` is remainder.

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

Length, slicing, and iteration work on Unicode characters by default. `.bytes()` gives raw bytes.

## Collections

```lucid
let primes = [2, 3, 5]
let ages = {"Ada": 36, "Linus": 21}
let tags = set(["new", "sale"])
let first = primes[0]        # traps when out of bounds
let maybe = primes.get(9)    # int?
```

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

## Tasks

A `task` is a function. Parameters always have types. With no `->`, a task returns `none`. Return types are never inferred, so a signature is always complete.

```lucid
task add(a: int, b: int) -> int
    return a + b

task connect(host: text, port: int = 443, secure: bool = true)
    ...
```

Arguments are positional. A parameter with a default must be passed by name:

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

Copying is by value: `var b = a` then changing `b` never affects `a`.

An alias uses `=`:

```lucid
type UserId = int
```

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

`match` must cover every case. It is a compiler error otherwise. `_` matches anything.

## Union types and optionals

`A or B` is a value of either type, like TypeScript. `T?` is shorthand for `T or none`. Unions flatten, so `(A or B) or A` is `A or B`.

```lucid
task find(name: text) -> User?
    ...

let user = find("Ada")
if user is not none
    print user.name      # narrowed to User
```

`is` tests type, `none`, or `empty`, and narrows the value in that branch. It never tests equality. Use `==` for equality. `x is empty` calls `x.is_empty()`.

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

A **trap** is an unrecoverable bug: out-of-bounds index, overflow, failed `expect`, or a `shared` access conflict. A trap prints a stack trace and exits with code 101. Traps cannot be caught.

## Generics

```lucid
task first[T](items: list[T]) -> T?
    return items.get(0)

type Pair[A, B]
    left: A
    right: B
```

Generics are monomorphized: each used combination gets its own compiled code.

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
