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
| `i8` `i16` `i32` `u16` `u32` `u64` `f32` | Sized numbers |
| `i64`, `u8` | Other names for `int` and `byte` |
| `list[T]` | Growable ordered list |
| `map[K, V]` | Hash map, keeps insertion order |
| `set[T]` | Hash set, keeps insertion order |
| `none` | Absence of a value |

There are no implicit conversions. Every number type has a task for each other one, named after its target: `x.to_float()`, `x.to_int()`, `x.to_u8()`, `x.to_f32()`, and so on. Each name in the table above works, and a task for the type a value already has does not exist.

- To an integer type, a value that does not fit traps. A `float` rounds toward zero first, and NaN traps.
- To `float` or `f32`, a value rounds to the nearest one. A `float` too large for `f32` becomes `inf`.

Integer overflow traps in every build. Wrapping math uses explicit tasks: `a.wrapping_add(b)`, `a.wrapping_sub(b)`, and `a.wrapping_mul(b)`, on any integer type, wrap at the width of that type. The argument has the same type as `a`.

`/` always yields a `float` type, and follows IEEE 754, so `1 / 0` is `inf`. Two integers of any type are converted to `float` first. Two `f32` values give an `f32`.

`//` and `%` take integers only. `//` rounds down, and `%` takes the sign of the divisor, so `(a // b) * b + a % b == a`. A zero divisor traps. A `float` operand is a compile error.

Arithmetic and comparison need one type on both sides. `==` and `!=` also work on `bool`. `and` and `or` skip their right side when the left side decides the result. `-x` needs a signed type or a `float` type.

A number literal is an `int` or `float`, unless its context asks for a sized type. An annotation, parameter, field, or element gives that type, and so does the other side of an operator or `in`. A list, map, or set literal on the right of `==`, `in`, and the like takes its type from the left in the same way:

```lucid
let a: u8 = 200
let b: i32? = 5
let c = a + 1        # u8
let d = 2 * 3 + a    # u8
let e: i16 = 1 + 2   # both literals are i16
```

An integer literal never becomes a `float` type. A number literal must fit its type, and a `float` literal that is not zero must not round to zero. A `-` directly before a number joins the literal, unless a call, index, or field follows the number. So `-9223372036854775808` is the lowest `int`, and `-1.5.to_int()` negates the result of the call.

Every integer type can be a key. Ranges, indexes, and `.len()` use `int`, so convert a sized number first.

A `byte` prints as a number, as do the other sized integers. An `f32` prints with the fewest digits that read back as the same `f32`, so `0.1` stays `0.1`.

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

Any value can be interpolated, and reads as `print` shows it. Interpolation is the only way to join text, so `+` does not work on `text`.

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

An index is an `int` from 0, and a negative index traps. A key of a map or set needs the `Hash` trait: an integer, `bool`, `text`, or a value made of those. A `float` type cannot be a key, nor can a value that holds one with no hand-written `hash`, because NaN does not equal itself. A key is a value like any other, so changing the variable it came from never changes the key.

All three have `.len()` and `.is_empty()`. `x in items` and `x not in items` test a list, or the keys of a map or set. A list test is linear, and compares with `==`, so it finds no NaN.

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

`print` shows a list as `[1, 2]`, a map as `{"a": 1}`, and a set as `set([1, 2])`. Text inside a value has quotes and escapes, so what prints reads back as a literal. `==` and `!=` compare two values of one type. Lists compare in order. Maps and sets compare what they hold, whatever its order. `<` and the like work on a list if they work on its elements: the first difference decides, and a shorter list comes first.

Planned: built-in tasks that take a lambda, such as `.map`. A program can write its own, see "Tasks".

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

Top-level tasks may be used before their line. A parameter cannot be assigned.

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

A task is a value. Its type is written `task(int, int) -> int`, or `task(text)` if it returns `none`. A task value is called like a task, with positional arguments:

```lucid
task apply(f: task(int) -> int, x: int) -> int
    return f(x)

task triple(x: int) -> int
    return x * 3

print apply(triple, 2)
print apply(x => x + 1, 2)
```

Naming a task gives its value. A generic one needs its types, as in `first[int]`. A `self` task has no value form: write a lambda. All its parameters count, so a default does not apply through a task value.

Lambdas use `=>` and hold a single expression. For anything longer, define a nested task. Parameters take their types from the context: an annotation, a parameter, a field, or an element. A lambda with no such context is error E0324. A generic call reads arguments left to right, so `map(items, x => x * 2)` knows `x` from `items`, and the lambda's result can give a type parameter. In a `then` branch a lambda needs parentheses.

```lucid
task map[T, U](items: list[T], f: task(T) -> U) -> list[U]
    var out: list[U] = []
    for item in items
        out.push(f(item))
    return out

let doubled = map(primes, x => x * 2)
let plus: task(int, int) -> int = (a, b) => a + b
```

A task inside a task is a nested task. It takes no type parameters, `self`, or defaults. It is a value made at its line, so it can only be used after that line, and it can call itself.

```lucid
task total(n: int) -> int
    let step = 2
    task sum(x: int) -> int
        if x <= 0
            return 0
        return x + sum(x - step)
    return sum(n)
```

A lambda or nested task reads names around it by capturing their values when it is made, so it can outlive them. Each pass of a loop makes a new capture. A `var` cannot be captured, which is error E0325, since a later change would not be seen and a captured name cannot change: write `let copy = count` first. A captured name keeps the type it was narrowed to, and does not narrow further inside: test a `let` copy of it.

A task value has `Show`, and prints as its type. It has no `Equal`, `Order`, or `Hash`, so it cannot be a key, and a value that holds one cannot use `==`. In an `any`, `==` is true only for one value that was copied.

`x.f(a)` also calls a field of `x` that holds a task value, if no task named `f` takes `x`.

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

`print` shows a value as `User(name: "Ada", age: 36)`, in the order the fields are written. `==` and `!=` compare every field. `<` and the like compare fields in that order, and need every field to have an order.

## Methods

A task with first parameter `self` is called with a dot. There is no `impl` block or class.

```lucid
task is_adult(self: User) -> bool
    return self.age >= 18

task birthday(change self: User)
    self.age += 1

if ada.is_adult()
    print "welcome"
```

`x.f(a)` runs the task `f` whose `self` type is the type of `x`. A `self` type can be an `any`, except for a task of its own trait. So one name can have one task per type, and such a name stays out of the shared namespace of "Choices". `f(x, a)` is an error, since one job has one form. `self` needs a type, which cannot be a bare type parameter, and `self: list[T]` covers every list. Two tasks of one name whose receivers can overlap are an error, counting each type parameter as free, and so is a built-in name on a built-in type, such as `len` on `text`. A dot call does not widen its receiver: an `int` does not reach `self: int?`. `x is empty` calls `x.is_empty()`.

The receiver must be a `var` when `self` is `change`, which arrives with parameter modes. The `change` keyword is not repeated on a dot call.

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

A variant prints as `Circle(radius: 2.0)` or `Point`. `==` compares variant and fields. In `<` and the like, an earlier variant comes first, and fields decide within a variant.

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

A union shows what it holds, and text in it has no quotes unless the union is inside another value. `==` on two values of one union type is true if they hold equal values of one type. `x == 1` on an `int?` is an error, since the sides differ in type: test it first. A union has no order.

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

Generics are monomorphized: each used combination gets its own compiled code. An alias can be generic too, as in `type Table[V] = map[text, V]`.

A generic body is checked once, where it is written, and a parameter is an opaque type there. So it works for every type or for none, and a call can never fail inside the body. Without a bound, a body can only store, pass, and return a `T`. A bound gives it more, see "Traits". A generic program that keeps asking for a larger type of itself is error E0903, even if it would stop at run time.

Type arguments come from the values, left to right, as an empty list takes its type from an earlier element. So `pair(1, [])` works for `pair[T](a: T, b: list[T])`, and `T` is `int` in `first([1, 2])`. A type that the values do not show is error E0319. Brackets name it, and they hold any type:

```lucid
let a = same[int?](none)
let b = Pair[int, text](left: 1, right: "a")
let c: list[text] = nothing[text]()
let d: Maybe[int] = Nothing       # the context gives the type to a constructor
let e = Nothing[int]
```

Only a constructor takes type arguments from its context. A generic call does not, since its arguments alone must explain it.

Unions flatten after substitution, so `T?` is `int?` for `T = int?`, and it is `none` for `T = none`.

## Traits

A trait lists required tasks. Satisfaction is structural: any type with matching tasks qualifies, with no declaration.

```lucid
trait Describe
    task describe(self) -> text

task show_all[T: Describe](items: list[T])
    for item in items
        print item.describe()
```

A task of a trait takes `self` first, with no type. A type has the trait if it has a task of that name with the same other parameters and the same return type. A bound names a trait, and `and` joins several: `[T: Show and Equal]`. Generic types take bounds too, as in `type Sorted[T: Order]`. A type that lacks a bound is error E0320 at the call, or in the type that names it.

Bounds use static dispatch. `any Describe` is a boxed value with dynamic dispatch, which allows mixed lists:

```lucid
let pets: list[any Describe] = [Dog(name: "Rex"), Cat(lives: 9)]
for pet in pets
    print pet.describe()
```

`any` names one trait, of your own or `Show`. The other built-in traits ask for two values of one type, so `any Order` and the like are errors. A value goes in wherever `any Describe` is expected, if its type has `Describe`. A union counts as one type here: it needs a task of its own, or `any Show`, which every type has. An expected union with an `any` member takes a single value, and never a union. As with unions, this needs an expected type, and there is no conversion between collections or between two `any` types.

An `any Describe` has only the tasks of `Describe`, and it meets a `Describe` bound. It shows as what it holds, and text in it has no quotes unless it is inside another value, which also holds for an `any` in a union. `==` is true if both hold equal values of one type. It has no `Order` or `Hash`. No test gets the value back out: `is` and typed arms do not work on it. Use a union if the code must know the type.

A trait with type parameters, and a generic task in a trait, are planned.

| Trait | Lets a `T` | Who has it |
| --- | --- | --- |
| `Show` | `print` and interpolation | every type, `any` included |
| `Equal` | `==`, `!=`, `in` on a list | every type, `any` included, except a task value or a value that holds one |
| `Order` | `<`, `<=`, `>`, `>=` | numbers, `text`, and lists and `type` or `choice` values made of those |
| `Hash` | be a key of a map or set | integers, `bool`, `text`, and lists, maps, sets, unions, and `type` or `choice` values made of those. A `none` counts only inside such a value |
| `Add`, `Sub`, `Mul` | `+`, `-`, `*` | numbers |
| `Div` | `/`, which gives a `T` | `float`, `f32`. An integer does not have it, since `/` on integers gives `float` |

The compiler derives `Equal`, `Hash`, `Order`, `Show`, `Copy`, and `Json` for a type when all its fields support them. Writing the task by hand overrides the derived version, and works inside larger values too. There are no macros. So far `Equal`, `Hash`, `Order`, and `Show` are derived, by the rules in "Collections", "Types", and "Choices". In a derived order a NaN compares as neither less nor more, so it counts as a tie. Planned: `Copy` and `Json`.

```lucid
task show(self: Money) -> text
    return "${self.cents // 100}"

task equal(self: Money, other: Money) -> bool
    return self.cents == other.cents

task compare(self: Money, other: Money) -> int      # negative, zero, or positive
    return self.cents - other.cents

task add(self: Money, other: Money) -> Money        # also sub, mul, and div
    return Money(cents: self.cents + other.cents)

task hash(self: Money) -> int                       # any result with `Hash`
    return self.cents
```

These names replace derived tasks or give an operator, so each must have this form, on a `type`, `choice`, or collection. Any other form is error E0323.

`hash` gives what stands for the value in a key. It may be any type with `Hash`, and it often names the fields that `equal` reads. Equal values must give equal results, or a key may not be found again. A breach is a bug in the program, and it stays memory safe. The result cannot lead back to the same type, even through a list or a field, since hashing it would never end, and such a type has no `Hash`. A type with a hand-written `equal` has no `Hash` unless it also has a hand-written `hash`, since a derived hash would not match it.

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
