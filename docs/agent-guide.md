# Lucid Guide for Agents

Everything needed to write Lucid programs that run today. Lucid has no training data: do not assume a feature from Python, TypeScript, or Rust unless it is shown here.

Every `lucid` block below runs. Run a file with `lucid run file.lcd`, or a package with `lucid run dir`. Check one with `lucid check file.lcd`, or `lucid check --json file.lcd` for data. Exit codes: 0 success, 1 compile error or a failed top-level `try`, 101 trap.

## Layout

- A block is an indent of exactly 4 spaces. A tab in indentation is an error.
- A statement ends at the end of its line. To continue a line, wrap it in `()`, `[]`, or `{}`. A trailing comma is allowed.
- `#` starts a comment.
- Keywords cannot be names: `and`, `any`, `as`, `break`, `change`, `choice`, `continue`, `else`, `empty`, `expect`, `extern`, `false`, `for`, `if`, `in`, `is`, `let`, `match`, `none`, `not`, `or`, `print`, `public`, `return`, `self`, `shared`, `spawn`, `take`, `task`, `test`, `then`, `trait`, `true`, `try`, `type`, `unsafe`, `use`, `var`, `weak`, `while`.
- There are no colons after `if` or `task`, no semicolons, and no braces around blocks.
- Statements can sit at top level, and they run in order.

## Variables and numbers

```lucid
let name = "Ada"      # cannot change
var count = 0         # can change
count += 1
let limit: int = 10   # annotation is optional here
print count           # `print` is a statement that takes one value
```

Every value starts with a value. Shadowing a name in the same scope is an error, and a nested block can shadow.

| Type | Meaning |
| --- | --- |
| `int` | 64-bit signed integer |
| `float` | 64-bit float |
| `bool` | `true`, `false` |
| `text` | immutable UTF-8 text |
| `none` | no value |
| `byte`, `i8`, `i16`, `i32`, `u16`, `u32`, `u64`, `f32` | sized numbers |

- There are no implicit conversions, and both sides of an operator have one type. Convert with `x.to_float()`, `x.to_int()`, `x.to_u32()`, and so on, one per type name. A task for the type a value already has does not exist.
- `/` always gives `float`, even on `int`: `7 / 2` is `3.5`. `//` rounds down and `%` takes the sign of the divisor, as in Python, so `-7 // 2` is `-4` and `-7 % 3` is `2`. Both take integers only, and a zero divisor traps.
- Integer overflow traps. `a.wrapping_add(b)`, `.wrapping_sub`, and `.wrapping_mul` wrap at the width of `a`, and `b` has the same type.
- `.to_int()` rounds a `float` toward zero. A value that does not fit traps.
- A number literal takes a sized type from its context: `let a: u8 = 200`, and `a + 1` is a `u8`. An integer literal never becomes a `float`: write `2.0`.
- Operators: `+ - * / // %`, unary `-`, `== != < <= > >=`, `and or not`, `+= -= *=`. There is no `**`. A chain such as `a < b < c` is an error.
- Bit operators `& | ^ ~ << >>` take any integer type, with Python's precedence, so `x & 1 == 0` works. Both sides of a shift have one type. A count below 0 or not less than the width traps, and `<<` drops the bits shifted out. `~a` on a `byte` stays a `byte`.
- A `float` prints as `3.0`, `0.1`, or `1.0e21`.

## Text

```lucid
let name = "Ada"
let age = 36
let line = "Hi {name}, next year {age + 1}"
print line
print "literal braces: {{ and }}"

let word = "héllo"
print word.len()       # 5, counts characters
print word[1..3]       # él
print word[0..1]       # h, the first character
print word[1..=3]      # éll
print word.is_empty()  # false
for letter in word     # each one is a one-character text
    print letter
print word.bytes()     # list[byte], the UTF-8 bytes
print "a,b".split(",") # ["a", "b"]; an empty separator traps
print word.upper()     # HÉLLO, and lower() too
print word.find("l")   # int?, the character index of the first match
print " x ".trim()     # x; also starts_with, ends_with, contains
let n = "42".to_int()  # int?, none unless the whole text is an integer
```

- `{...}` interpolates any value. A literal `{` is `{{`, and a literal `}` is `}}`. Interpolation is the only way to join text. `+` does not work on text: write `"{a}{b}"`.
- There is no `x[0]` on text, and no character type. Write `x[0..1]`.
- Slice ends are both required, and can be any `int` expression, as in `x[i..i + 1]`. A slice out of range traps. There is no negative index.
- `==`, `<`, and the like compare text by code point.
- `"""` starts multi-line text, with common indentation removed.
- Escapes are `\n`, `\t`, `\"`, and `\\`.

## Collections

Lists, maps, and sets are values. After `var b = a`, changing `b` never changes `a`.

```lucid
var names: list[text] = []    # an empty literal needs a type from its context
names.push("Ada")
names.push("Linus")
names[0] = "Grace"
names.swap(0, 1)
print names                   # ["Linus", "Grace"]
print names.len()
print names[0..1]             # a new list
print "Ada" in names          # false
print names.is_empty()

let last = names.pop()        # text?, none on an empty list
let first = names.get(0)      # text?, none when out of range

var ages = {"Ada": 36}        # a map keeps insertion order
ages["Linus"] = 21            # add or replace
ages["Ada"] += 1              # traps when the key is missing
ages.remove("Linus")          # does nothing when the key is missing
print ages["Ada"]             # traps when the key is missing
print ages.get("Nobody")      # none
print "Ada" in ages
for key in ages               # keys, in insertion order
    print "{key}: {ages[key]}"

var seen = set([1, 2, 2])     # type set[int]. An empty one: `var s: set[int] = set([])`
seen.add(3)
seen.remove(1)
print seen                    # set([2, 3])
print 2 not in seen
print seen.len()

var ranks = [3, 1, 2]
ranks.sort()                  # [1, 2, 3], stable, elements need Order

var empty_map: map[text, int] = {}
empty_map["a"] = 1
```

These are all the built-in tasks on collections: `len`, `is_empty`, `get`, `push`, `pop`, `swap`, `add`, `remove`, and `sort` on a list. Text has `len`, `is_empty`, `bytes`, `split`, `upper`, `lower`, `to_int`, `starts_with`, `ends_with`, `contains`, `find`, `trim`. A `float` has `sqrt`. Write anything else yourself.

- An index is an `int` from 0, and a negative one traps. `.get` gives `none` for any index out of range, negative included. Sized numbers need `.to_int()` to be an index.
- A key can be an integer, `bool`, `text`, or a value made of those. A `float` cannot be a key.
- `.pop()` must be a whole statement, or all of the value of `let`, `var`, `return`, or `name =`. `f(x.pop())` is an error.
- A `for` body cannot change what it visits. To remove or add while looping, loop over a copy: `let copy = items`, then `for x in copy`.
- `for change x in items` makes `x` the element itself, and the body cannot use `items`.

```lucid
var scores = [1, 2, 3]
for change s in scores
    s *= 2
print scores          # [2, 4, 6]

var grid = [[1, 2], [3, 4]]
grid[1][0] = 9
grid[0].push(5)
print grid            # [[1, 2, 5], [9, 4]]
```

`lucid check --costs file.lcd` lists where a value is held twice and a write copies it.

## Control flow

```lucid
let score = 85
if score >= 90
    print "A"
else if score >= 80
    print "B"
else
    print "C"

var n = 0
while n < 3
    n += 1
    if n == 2
        continue
    print n

for i in 0..3         # 0, 1, 2
    print i
for i in 0..=3        # 0, 1, 2, 3
    if i == 2
        break

let label = if n == 1 then "item" else "items"
print label
```

Conditions must be `bool`. There is no truthiness: write `items is empty` or `x != 0`. `is empty` works on text and collections.

## Tasks

A task is a function. Parameters and return types are always written. With no `->`, a task returns `none`. Every path of a task with a return type must return. Top-level tasks can be used before their line.

```lucid
task add(a: int, b: int) -> int
    return a + b

task connect(host: text, port: int = 443, secure: bool = true) -> text
    return "{host}:{port} {secure}"

print add(1, 2)
print connect("example.com")
print connect("example.com", port: 80)   # a default goes by name, others by position
```

- A parameter cannot be assigned. Copy it to a `var`, or use `change`.
- A task cannot read or write a top-level `var`: pass it. A task may read a top-level `let` only if its value is a literal.
- A default is a literal or an empty `[]` or `{}`, and parameters with defaults come last.

### Parameter modes

```lucid
task double_all(change items: list[int])    # changes the caller's value
    for i in 0..items.len()
        items[i] *= 2

task sum(take items: list[int]) -> int      # owns a copy, and may change it
    var total = 0
    while items is not empty
        let last = items.pop()
        if last is int
            total += last
    return total

var numbers = [1, 2, 3]
double_all(change numbers)    # `change` is required at the call too
print sum(numbers)
print numbers
```

A read parameter cannot change. A `change` argument is a `var`, or a path into one such as `x.age` or `grid[i]`. In one statement, a variable that goes to `change` may not be used elsewhere, except as a plain number or `bool` argument of that call, as in `items.grow(items.len())`. Otherwise name the value first with `let`.

### Task values, lambdas, nested tasks

```lucid
task apply(f: task(int) -> int, x: int) -> int
    return f(x)

task triple(x: int) -> int
    return x * 3

print apply(triple, 2)
print apply(x => x + 1, 2)

let plus: task(int, int) -> int = (a, b) => a + b
print plus(1, 2)

task adder(n: int) -> task(int) -> int
    return x => x + n

let add5 = adder(5)
print add5(1)

task sum_to(n: int) -> int
    task go(x: int, total: int) -> int    # a nested task can call itself
        if x == 0
            return total
        return go(x - 1, total + x)
    return go(n, 0)

print sum_to(4)
```

- A lambda is one expression. Its parameter types come from the context. For more, use a nested task.
- `task(text)` is the type of a task that takes a `text` and returns `none`.
- A lambda or nested task captures values when it is made, so it never sees a later change. Capturing a `var`, a `change` or `take` parameter, or a `for change` name is an error: write `let copy = count` first. A captured name cannot change, so a closure cannot keep a running count.
- A nested task can only be used after its line. It takes no defaults, `change` parameters, or type parameters.
- A task with a `change` parameter has no value form.

## Types

```lucid
type User
    name: text
    age: int = 0

type UserId = int             # an alias

var ada = User(name: "Ada", age: 36)    # constructor arguments are always named
ada.age += 1
print ada                     # User(name: "Ada", age: 37)
print ada.name
print ada == User(name: "Ada", age: 37)
print User(name: "Bob")
```

Values copy: after `var b = a`, changing `b` never changes `a`. A type cannot hold itself directly: `next: Node?` or `children: list[Node]` is fine.

### Methods

There are no classes. A task with first parameter `self` is called with a dot, and only with a dot. `f(x)` is an error for such a task, and `x.f()` is an error for any other task.

```lucid
type Counter
    value: int = 0

task bump(change self: Counter, by: int)
    self.value += by

task is_big(self: Counter) -> bool
    return self.value > 10

task twice(self: list[int]) -> list[int]    # works on built-in types too
    var out: list[int] = []
    for x in self
        out.push(x * 2)
    return out

var c = Counter()
c.bump(20)              # no `change` on the receiver
print c.is_big()
print [1, 2].twice()
```

`self` always has a type. Several tasks can share a name if their `self` types differ. A built-in name on a built-in type, such as `len` on `text`, cannot be defined again.

In a package, one with a `lucid.toml`, `self` must be a `type` or `choice` of that package, or the task must be named by a trait of that package, or else E0342. So `twice(self: list[int])` above works only in a file with no `lucid.toml`. In a package, write `twice(items: list[int])` and call it as `twice(xs)`.

## Choices and match

```lucid
choice Shape
    Circle(radius: float)
    Rect(width: float, height: float)
    Point

task area(self: Shape) -> float
    return match self
        Circle(radius) -> 3.14 * radius * radius
        Rect(w, h) -> w * h
        Point -> 0.0

let shapes = [Circle(radius: 2.0), Rect(width: 2.0, height: 3.0), Point]
for shape in shapes
    print "{shape}: {shape.area()}"

match shapes[0]
    Circle(r) -> print r
    Rect(w, h) ->              # a block arm: `->`, then an indented block
        let size = w * h
        print size
    _ -> print "other"
```

- A variant with fields is built with named arguments. A variant with no fields has no `()`.
- A pattern binds every field by position, whatever the names. `_` skips one, so `Rect(_, _)` tests only the variant.
- `match` must cover every case. `_` matches anything.
- Types, choices, variants, and tasks share one namespace, so two choices cannot use one variant name. Field access such as `shape.radius` does not work on a choice: use `match`.
- Patterns do not nest, and there are no guards. A nested `match` goes in a block arm.
- Other patterns: an `int`, `float`, `text`, or `bool` literal, and a name that binds anything left. On those types, end with `_` or a name.
- As a statement, an arm is an expression, `print`, `return`, or an indented block. As a value, each arm is one expression or a `return`.

```lucid
task describe(n: int) -> text
    return match n
        0 -> "zero"
        1 -> "one"
        _ -> "many"

print describe(1)
```

## Unions and optionals

`A or B` holds either type. `T?` means `T or none`.

```lucid
task find(items: list[text], wanted: text) -> int?
    for i in 0..items.len()
        if items[i] == wanted
            return i
    return none

let found = find(["a", "b"], "b")
if found is not none
    print found + 1        # narrowed to int

task label(name: text?) -> text
    if name is none
        return "nobody"
    return name            # narrowed, since the branch above returned

print label(none)

let mixed: list[int or text] = [1, "two"]
for item in mixed
    match item
        n: int -> print n + 1
        t: text -> print t

let maybe: int? = 5
match maybe
    none -> print "none"
    n: int -> print n
```

- `is` tests a type, `none`, or `empty`, and it narrows. It never tests equality.
- A `match` on a union names no variant. Match `e: ParseError`, then `match e` inside that arm.
- `x == 1` on an `int?` is an error: test it with `is` first.
- Only a local name or parameter narrows, never a field or call. Write `let b = a.b`, test `b`, and use `b`.
- Assigning to a name ends its narrowing. A loop forgets what it knows about a name assigned in its body.
- A union literal needs an expected type: `let x: list[int?] = [1, none]` works, and `[1, none]` alone is an error. A `list[int]` is not a `list[int?]`.
- Return `none` with `return none`. A bare `return` is for a task with no return type.

## Errors

There are no exceptions. A task that can fail returns a union with an error type, which is any type with `task error_message(self: T) -> text`.

```lucid
choice ParseError
    Empty
    Bad(line: int)

task error_message(self: ParseError) -> text
    return match self
        Empty -> "empty input"
        Bad(line) -> "bad line {line}"

task parse(source: text) -> int or ParseError
    if source is empty
        return Empty
    return source.len()

task twice(source: text) -> int or ParseError
    let size = try parse(source)    # returns the error to the caller
    return size * 2

task report(source: text) -> none or ParseError
    print try twice(source)         # no `return` needed at the end

print twice("") else 0              # fallback value
try report("abc")
report("") else none                # ignores the error, on purpose

match twice("")
    n: int -> print n
    e: ParseError -> print e.error_message()
```

- `try` and `else` act on error types. If the union has no error type, they act on `none`, so `try` works on any `T?`, and `none` from a `none or E` task is success.
- `try` covers the whole postfix chain: write `(try f()).len()`.
- The return type of the task must accept what `try` returns. At top level, `try` prints the message to stderr and exits with code 1.
- Every error must be used. A statement that drops one, or a name that holds one and is never read, is E0328.
- `else` needs the same type on both sides, or an annotation: `let n: int? = parse(s) else none`.
- `any Error` holds any error type: `task f() -> int or any Error`.

## Generics

```lucid
task first[T](items: list[T]) -> T?
    return items.get(0)

type Pair[A, B]
    left: A
    right: B

task map[T, U](items: list[T], f: task(T) -> U) -> list[U]
    var out: list[U] = []
    for item in items
        out.push(f(item))
    return out

print first([1, 2])
print Pair[int, text](left: 1, right: "a")
print map([1, 2], x => x * 2)

task nothing[T]() -> list[T]
    return []

let a = nothing[text]()    # brackets name a type that the values do not show
print a
```

A method on a generic type takes the type parameters too:

```lucid
type Stack[T]
    items: list[T]

task push_one[T](change self: Stack[T], x: T)
    self.items.push(x)

var stack = Stack[int](items: [])
stack.push_one(3)
print stack.items
```

Type arguments come only from the values, left to right, never from the expected type. Only a constructor takes them from its context.

## Traits

A body may only store, pass, and return a `T`, unless a bound lets it do more. That includes `print`, which needs `Show`.

```lucid
task sort[T: Order](change items: list[T])
    for i in 1..items.len()
        var j = i
        while j > 0 and items[j - 1] > items[j]
            items.swap(j - 1, j)
            j -= 1

task show_all[T: Show](items: list[T])
    for item in items
        print item

task count_of[T: Equal and Show](items: list[T], wanted: T) -> text
    var n = 0
    for item in items
        if item == wanted
            n += 1
    return "{wanted}: {n}"

var numbers = [3, 1, 2]
sort(change numbers)
show_all(numbers)
print count_of(numbers, 2)
```

| Trait | Allows | Who has it |
| --- | --- | --- |
| `Show` | `print`, interpolation | every type |
| `Equal` | `==`, `!=`, `in` on a list | every type but task values |
| `Order` | `<`, `<=`, `>`, `>=` | numbers, `text`, and lists and types made of those |
| `Hash` | being a key | integers, `bool`, `text`, and lists, types made of those |
| `Add`, `Sub`, `Mul` | `+`, `-`, `*` | numbers |
| `Div` | `/` that gives `T` | `float`, `f32` |

A bound gives only what its row says: `Order` does not give `==`, so write `[T: Order and Equal]` for both. Built-in trait names, such as `Order`, cannot name a type.

`Show`, `Equal`, `Order`, and `Hash` are derived for a `type` or `choice`, in field order. Write these tasks to replace a derived one or to give an operator, in exactly this form:

```lucid
type Money
    cents: int

task show(self: Money) -> text
    return "${self.cents // 100}"

task equal(self: Money, other: Money) -> bool
    return self.cents == other.cents

task compare(self: Money, other: Money) -> int    # negative, zero, or positive
    return self.cents - other.cents

task add(self: Money, other: Money) -> Money      # also sub, mul, div
    return Money(cents: self.cents + other.cents)

task hash(self: Money) -> int
    return self.cents

print Money(cents: 250) + Money(cents: 100)
print Money(cents: 1) < Money(cents: 2)
```

Define your own trait with a list of required tasks. Any type with matching tasks has it, with no declaration. Use `any Trait` to hold mixed types in one list:

```lucid
trait Describe
    task describe(self) -> text

type Dog
    name: text

type Cat
    lives: int

task describe(self: Dog) -> text
    return "dog {self.name}"

task describe(self: Cat) -> text
    return "cat with {self.lives} lives"

task show_all[T: Describe](items: list[T])
    for item in items
        print item.describe()

let pets: list[any Describe] = [Dog(name: "Rex"), Cat(lives: 9)]
for pet in pets
    print pet.describe()
show_all([Dog(name: "Fido")])
```

`any` needs an expected type, as a union does. An `any` value has only the tasks of its trait, and no test gets the value back: use a union if code must know the type. `any Show` is allowed, and `any Order` and `any Hash` are not.

## Tests

```lucid
task add(a: int, b: int) -> int
    return a + b

test "adds numbers"
    expect add(2, 3) == 5
```

`lucid test file.lcd` runs each `test` and lists the results, or `lucid test --json file.lcd` for data. It runs no top-level statements, and exits with 1 if a test fails. `lucid run` drops tests. A test sees what a task sees. A trap or failed `try` fails only that test.

`expect` also works outside tests. A failed comparison shows both values, as `expect failed: 4 == 5`, and what passes narrows: after `expect x is int`, `x` is an `int`.

## Threads

```lucid
task square(n: int) -> int
    return n * n

var jobs: list[spawned[int]] = []
for n in [1, 2, 3]
    jobs.push(spawn square(n))
for job in jobs
    print job.wait()
```

`spawn` takes one call of a task, runs it on a new thread, and gives a `spawned[T]`. The arguments run first, in this thread, and the thread gets copies: a later change here does not reach it. `.wait()` blocks and gives the result. A task waits for every thread it spawned before it returns. A trap in any thread ends the program. `spawn` with a `change` argument, or with anything but a task call, is E0341. Under `--target web`, `spawn` is E0907.

```lucid
task worker(jobs: channel[int], total: lock[int])
    while true
        let job = jobs.receive()       # int?, none once closed and empty
        if job is none
            break
        total.update(n => n + job)    # runs while it holds the lock

let jobs = channel[int](16)           # blocks `send` while 16 wait
let total = lock(0)
for i in 0..4
    spawn worker(jobs, total)
for n in 1..101
    jobs.send(n)
jobs.close()
```

- Copying a channel or a lock shares it. A value sent or kept is a copy.
- `receive` is a whole statement or the whole value of `let`, `var`, `return`, or an assignment, like `.pop`. `send`, `close`, and `update` are statements.
- `read()` gives a copy. Inside `update(f)`, reading the same lock traps. So do `send` after `close`, a second `close`, and a capacity below 1.
- When every thread waits, the program traps with `deadlock: every thread waits`.

## Foreign code

```lucid
unsafe extern task strlen(s: pointer[byte]) -> u64

var raw = "hello".bytes()
raw.push(0)

# `raw` ends in a zero byte, and stays unchanged for the call.
unsafe
    print strlen(raw.pointer())
```

- An `extern` task is a C function in the C runtime or `kernel32` on Windows, or in libc or libm on Linux. It takes and gives numbers, `bool`, and `pointer[byte]`. A C `int` is `i32`.
- Only an `unsafe` block can call one, or `.pointer()`. It is not a value. The block needs a `#` comment above it that says why it is sound.
- `.pointer()` works on a `list[byte]` in a name, and is valid until the list changes. A C string is `.bytes()` with a zero byte pushed.
- `lucid check --unsafe file.lcd` lists every site.

## Modules

```text
# models.lcd
public type User
    name: text

public task greet(user: User) -> text
    return "Hi {user.name}"
```

```text
# main.lcd
use ./models
use ./models.User

print models.greet(User(name: "Ada"))
```

- `use ./a.b` reads `a/b.lcd` next to this file, or else takes the name `b` from `a.lcd`. `as` renames.
- A name is private unless marked `public`, and so is a task with `self`. A public `type` shows its fields, and a public `choice` its variants.
- Build a variant with `models.Red`. A pattern is just `Red`, with no prefix.
- Only the entry file runs code. An imported module holds declarations and `let` with a literal value.

## Standard library

`use std.x` needs no dependency entry. In a package, `std.fs`, `std.env`, `std.net`, and `std.process` need a permission, see "Packages".

```lucid
use std.args
use std.console
use std.env
use std.fs
use std.json
use std.time

type Book
    title: text
    pages: int = 0

let name = args.get(0) else "world"    # the first word after the program, as `text?`
print "Hello, {name}!"
print args.all().len()
print env.get("LUCID_NOT_SET") else "unset"
console.write("Name? ")                        # no newline, and flushed
let typed = console.read_line() else ""        # without its line ending; `none` at the end of input

task copy(from: text, to: text) -> none or fs.FileError
    let content = try fs.read_text(from)
    try fs.write_text(to, content)

print fs.exists("no/such/file")
print fs.read_text("no/such/file") else "empty"

let start = time.clock()
time.sleep(time.millis(5))
print time.clock().since(start).to_seconds() > 0.0
print time.Date(year: 2026, month: 11, day: 6).time().date()      # 2026-11-06T00:00:00Z

let raw = json.write([Book(title: "Dune", pages: 412)])
print raw                                      # a list of one object
let books = json.parse[list[Book]](raw) else []
print books[0].title
print json.parse[Book]("[]")                   # a JsonError
```

- Pass arguments after the path: `lucid run file.lcd add "buy milk"`.
- `std.fs` reads and writes whole files: `read_text`, `write_text`, `append_text`, `read_bytes`, `write_bytes`, `list_dir`, `real_path`, `make_dir`, `remove`, `rename`. Each gives its result or a `fs.FileError`. `exists` gives a `bool`.
- `FileError` has `NotFound(path)`, `Denied(path)`, `NotText(path)`, and `Failed(path, detail)`.
- `std.net` is TCP. `net.listen(port)` gives a `Listener` on `127.0.0.1`, or on every address with `host: "0.0.0.0"`. Port 0 takes a free one, read back as `listener.port`. `listener.accept()` and `net.connect(host, port)` give a `Connection`, with `read(max)`, `write(bytes)`, and `set_timeout(time.millis(n))`. `read` gives an empty list once the other end closes. Failures are a `net.NetError`: `Refused`, `TimedOut`, `InUse`, `Closed`, or `Failed(detail)`. A socket closes when nothing holds it. Under `--target web`, `use std.net` is E0907. `connection.secure(host)` starts TLS as a client.
- `std.http` serves HTTP/1.1. `var app = http.App()`, then `app.get("/", home)` and `app.post("/add", add)`, where `task home(request: http.Request) -> http.Response`, then `try app.serve(8080)`, which never returns. Build the answer with `http.page(node)`, `http.json(value)`, `http.text(content)`, or `http.Response(status: 303, headers: [http.Header(name: "Location", value: "/")])`. Read `request.path`, `request.query`, `request.header("cookie")`, and `request.body_text()`. Under `--target web`, `use std.http` is E0907. As a client, `let got = try http.get("https://example.com/")` gives an `http.Response` with `got.status`, `got.header("content-type")`, and `got.body_text()`, and `http.post(url, "application/json", data.bytes())` sends a body. Both take `headers: [http.Header(name: "Authorization", value: key)]` and `seconds: 120`, the longest wait for a read, 30 unless given. Both give a `NetError` on failure, follow no redirect, and check `https` certificates.
- `std.process` runs a program directly, with no shell. `try process.run("git", ["status"], input: "", seconds: 10)` waits, and gives an `Output` with `code`, `out`, and `err`. `try process.start(program, args)` gives a `Child` with `write(content)`, `read_line()` as `text?`, `close_input()`, `wait()` for the exit code, and `kill()`. Failures are a `process.ProcessError`: `NotFound(program)`, `TimedOut`, or `Failed(detail)`. Under `--target web`, `use std.process` is E0907.
- `std.crypto` takes and gives `list[byte]`, so pass text as `"abc".bytes()`. `crypto.sha256(data)`, `crypto.sha512(data)`, `crypto.hmac_sha256(key, data)`, and `crypto.hex(bytes)` for lowercase hex. Compare secrets with `crypto.same(a, b)` or `crypto.hmac_check(key, data, tag)`, never `==`, which can stop at the first differing byte. `crypto.random_bytes(count)` is the only random source. Store passwords as `crypto.hash_password(password)`, a `text`, and check them with `crypto.check_password(password, hash)`. Under `--target web`, `use std.crypto` is E0907.
- `std.sql` is SQLite. `let db = try sql.open("app.db")`, or `":memory:"`. A query is a text literal, and every value goes through a `?` mark: `try db.run("INSERT INTO users (name, age) VALUES (?, ?)", [sql.Text(value: name), sql.Int(value: 36)])` gives the count of changed rows, and `try db.query("SELECT name FROM users WHERE age > ?", [sql.Int(value: 30)])` gives a `list[sql.Row]`, each with `columns` and `values`. Pass `[]` for no values. Match a value: `Text(value) -> ...`, with `Null`, `Int`, `Float`, and `Bytes`. A query cannot be built from `text`, nor with `{...}`: `"... WHERE name = '{name}'"` is E0303. Join parts with `+`, as `base + " ORDER BY name"`. Use `RETURNING id` to read a new row's id. A failure is a `sql.SqlError`: `Constraint`, `Busy`, or `Failed`, each with `detail`. Needs the `files` permission. Under `--target web`, `use std.sql` is E0907.
- `std.html` builds a page that is escaped on render: `http.page(html.document("Title", [html.h1(name), html.a("/next", "Next")]))`. Other elements are `html.Element(tag: "form", attrs: [html.Attribute(name: "method", value: "post")], children: [...])`. A `script` element, an `on...` attribute, or a `javascript:` link traps.
- `json.parse[T](raw)` gives `T or json.JsonError`, and `json.write(value)` gives `text`. Both need `Json`, which is derived. A `type` is an object, and a left-out field takes its default, or reads `null`. A `choice` is `"Red"` or `{"Circle": {"radius": 1.0}}`. A map key must be `text`.
- A union has `Json` only if each member reads from its own kind of JSON value: `int or text` works, `int or float` does not.
- `std.dom` makes a browser page, and needs `--target web`, or else E0906. `dom.app(init, view, update)` shows `view(init)`, and on each event gives `update` a message: `task view(model: Model) -> dom.Node[Message]` and `task update(model: Model, message: Message) -> Model`. Build nodes with constructors, whose types come from context: `dom.Element(tag: "button", attrs: [dom.Attribute(name: "id", value: "add"), dom.On(event: "click", message: _ => Add)], children: [dom.Text(content: "+")])`. An `On` message gets the event's key, or the element's value: `dom.On(event: "input", message: v => Typed(value: v))`.
- `time.now()` gives a `Time`, and `.date()` its UTC `Date`. `time.clock()` gives an `Instant`, for measuring. `.since(earlier)` gives a `Duration`, which has `+`, `-`, `<`, `.to_seconds()`, and `.to_millis()`. `time.seconds(n)` and `time.millis(n)` make one.

## Packages

```text
blog/
    lucid.toml
    src/main.lcd      # or src/lib.lcd for a library, never both
```

```toml
[package]
name = "blog"
version = "0.1.0"

[permissions]
files = ["./data"]        # or true. `env` is the same, with names
unsafe = true

[dependencies]
markdown = { path = "../markdown", allow = [] }
```

- A package has no permission it does not list, or else E0340. A dependency's permissions must be named in its `allow`, and its parent must hold them.
- Outside `files`, `std.fs` gives `Failed`, and `env.get` of a name not in `env` traps. A file with no `lucid.toml` has every permission.

- `lucid run`, `lucid build`, `lucid check`, and `lucid test` take a file or a package directory. With no path they use the package around the current directory.
- `lucid build` writes a standalone program, `NAME.exe` on Windows or `NAME` on Linux, to the current directory. `lucid build --target web` writes `NAME.js` instead, for a page or `node`. A page loads it with `<script src="NAME.js">`. There a user `extern` is E0905.
- `lucid fix` makes the fixes that errors show as `fix:`, such as a misspelled name. Run it again after a fix of a lexer error.
- `lucid fmt` puts a file or package in the one layout. `lucid fmt --check` only lists what would change.
- `lucid context std.fs` prints the public tasks and types of a module, a file, or a package, with their `##` doc comments.
- `lucid test` on a package runs the tests of every file in `src`.
- `use markdown` is the dependency's `src/lib.lcd`. `use markdown.render` reads `src/render.lcd`, or else takes `render` from `src/lib.lcd`. `use markdown.a.b` follows the rule of `./`, from `src`.
- `use` needs a dependency in this package's own `lucid.toml`. A dependency of a dependency does not count. A local file needs `./`.
- A dependency from a registry has a `version` and no `path`: `markdown = { version = "1.2.0" }`, with `[registry]` and `url = "https://..."` or a directory. The version is a floor, and the program gets the highest floor any manifest names, never a newer one. `lucid run`, `check`, `test`, and `build` fetch what is missing, and write `lucid.lock`, which pins each archive's SHA-256: check it in. Two major versions of one package are E0344, a package the registry lacks E0343, and a changed archive E0345.
- `lucid package` checks a library, writes `NAME-VERSION.lpk` to the current directory, and prints its index line. Put the archive at `files/NAME/VERSION.lpk` in the registry, and add the line to `index/NAME`.

## What is not available

Some keywords parse but do not run yet, and report E0901: `shared`, `weak`.

There is no console input, and no tuples: use a `type`. These do not exist, so write your own: `join`, `replace`, `abs`, `min`, `max`, `sum`, `range`, `enumerate`, `zip`, `.map`, `.filter`, `.keys`, `.values`, `.insert`, `.clear`, text to `float`, and number to text other than interpolation. There is no way to turn a number into a character. To map characters to numbers, search a text such as `"abcdefghijklmnopqrstuvwxyz"`.

`print` is the only output, and it ends each value with a line break.

## Reading errors

Each error has a stable code and usually says what to do. `lucid explain E0303` gives its cause, an example, and a fix.

| Code | Usual cause |
| --- | --- |
| E0201 | Syntax, often a `:` after `if`, or a missing indent |
| E0301 | Name is not defined, often a built-in that does not exist |
| E0303 | Types do not fit, often `int` with `float`, or `text + text` |
| E0305 | Assigning to a `let` or parameter |
| E0306 | A task uses a top-level `var` |
| E0307 | Arguments in the wrong form |
| E0309 | A task may end without returning |
| E0314 | Empty `[]` or `{}` with no type: add an annotation |
| E0316 | `match` does not cover every case |
| E0319 | A type argument cannot be told: write `f[int]()` |
| E0320 | A type lacks a trait: add a bound such as `[T: Show]` |
| E0324 | A lambda has no context for its parameter types |
| E0325 | A lambda or nested task captures a `var`: copy it to a `let` |
| E0326 | A value is used while something changes it: split the statement |
| E0327 | `try` or `else` on a value that cannot fail |
| E0328 | An error is not used: add `try`, `else`, or `match` |
| E0329 | A `test` name holds `{}`, or repeats |
| E0330 | A call to an `extern` task or `.pointer()` is outside `unsafe` |
| E0332 | An `unsafe` block has no comment |
| E0901 | The feature parses, but does not run yet |
