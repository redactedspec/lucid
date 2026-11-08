# Language Reference

This document is normative. The formal syntax is in [grammar.md](grammar.md).

## Files and programs

Source files use the `.lcd` extension and UTF-8. Each file is one module.

The entry file may contain statements at top level. They run in order. Library modules hold declarations and constant `let` values only.

```lucid
print "Hello, world!"
```

## Layout

- Blocks are defined by indentation of exactly 4 spaces. A tab in the indentation of a line of code is an error.
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

A task may read a top-level `let` whose value is a literal, with or without a leading `-`. A text with no interpolation is a literal. Such a value exists before any statement runs, so a task never sees it uninitialized. Reading any other top-level `let` reports E0901, by decision 151.

## Basic types

| Type | Meaning |
| --- | --- |
| `int` | 64-bit signed integer |
| `float` | 64-bit IEEE float |
| `bool` | `true` or `false` |
| `text` | Immutable UTF-8 string |
| `sql` | A query for `std.sql`, made only from text literals |
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

`x.sqrt()` gives the square root of a `float` or `f32`, and NaN below zero, as IEEE 754 says.

Integer overflow traps in every build. Wrapping math uses explicit tasks: `a.wrapping_add(b)`, `a.wrapping_sub(b)`, and `a.wrapping_mul(b)`, on any integer type, wrap at the width of that type. The argument has the same type as `a`.

Bit operators `&`, `|`, `^`, `~`, `<<`, and `>>` work on any integer type, by decisions 153 and 181. They bind tighter than comparisons and looser than `+`, as in Python, so `x & 1 == 0` tests the low bit. `~x` flips every bit of the type's width, so `~a` is `55` for a `byte` of `200`. The shift count has the same type as the value. A count below 0, or not less than the width, traps. `<<` drops the bits shifted out, and does not trap on them. `>>` keeps the sign of a signed type, and fills with zeros on an unsigned one.

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

A text literal with no `{...}` has the type `sql` where the context expects `sql`, as a number literal takes the type the context expects. `sql + sql` is `sql`, and a text literal, or `+` of text literals, next to a `sql` in `+`, `==`, or `!=` is one too. Nothing else makes a `sql`: there is no conversion from `text`, and `sql` has only `Show` and `Equal`, so `json.parse` cannot give one. So a query holds only text that the program's author wrote, and `std.sql` takes each value through a `?` mark, by decision 195. A `sql` shows as its text, and `==` compares the text. Giving `text` where `sql` is expected is error E0303.

```lucid
let all: sql = "SELECT name FROM users"
let adults = all + " WHERE age >= ?"
```

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

```lucid
print "a,b,,c".split(",")    # ["a", "b", "", "c"]
print "Straße".upper()       # STRASSE
print "ÀB".lower()           # àb
print "a.lcd".ends_with(".lcd")  # true, and starts_with too
print "héllo".contains("ll") # true
print "héllo".find("l")      # int?, here 2
print " hi
".trim()         # hi
let n = "-42".to_int()       # int?, here -42
let bad = "4x".to_int()      # none
```

`x.split(separator)` gives a `list[text]` of the parts between each `separator`, so it has one more part than there are separators, and a text with none gives `[x]`. An empty separator traps: use `for` to visit characters.

`x.upper()` and `x.lower()` change case by Unicode's default rules, which do not depend on a language. So the length can change, as `"ß".upper()` is `"SS"`.

`x.starts_with(part)`, `x.ends_with(part)`, and `x.contains(part)` give a `bool`. `x.find(part)` gives the `int?` character index of the first match, or `none`, so it can slice `x`. An empty `part` matches at the start, so `x.find("")` is 0.

`x.trim()` gives `x` without the Unicode white space at either end, the characters with the `White_Space` property.

`x.to_int()` gives the `int` that the whole text spells, or `none`. It takes an optional `+` or `-`, then ASCII digits, and nothing else: no space, `_`, or base prefix. A number that does not fit in `int` gives `none`.

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

`for` visits a list's elements, a set's values, or a map's keys. The body can read what the loop visits but cannot change it, which is error E0326. To change it, loop over a copy: `let copy = items`, then `for x in copy`. Other fields of the same variable can change.

`for change x in items` visits a list's elements, and `x` is the element itself: assigning to it or changing it changes the list, on every way out of the body. The body cannot use `items` at all, or for a path such as `a.b.items`, anything under `a.b`. Each write is in place. `for change` does not work on a map or set.

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

`x.sort()` puts a list in order, smallest first, by `<` on its elements, so they need `Order`. It changes its receiver, which follows the rules for a `change` argument. The sort is stable: equal elements keep their order. It is a merge sort, so it takes time `n log n` and a second list of `n` elements. In a derived order a NaN counts as a tie, so a list that holds one may not end in order. `sort` is a task of the prelude, written in Lucid, so a program cannot define another `sort` on lists.

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

A `task` is a function. Parameters always have types. With no `->`, a task returns `none`. Return types are never inferred, so a signature is always complete. A task with a return type must return on every path, except that a task that gives only `none` or error types, such as `-> none or FileError`, returns `none` at its end. `return` is only allowed in a task.

Top-level tasks may be used before their line. A parameter cannot be assigned.

```lucid
task add(a: int, b: int) -> int
    return a + b

task connect(host: text, port: int = 443, secure: bool = true)
    ...
```

Arguments are positional. A parameter with a default must be passed by name, and only such a parameter can be. Parameters with defaults come last. Named arguments follow positional ones, in parameter order, so arguments always run in the order written. A default is a literal, which includes `none` and an empty `[]` or `{}`. Each call that uses an empty one gets a new collection.

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

Naming a task gives its value. A generic one needs its types, as in `first[int]`. A `self` task has no value form: write a lambda. A task with a `change` parameter has no value form at all, since a task type has no way to say `change`. A `take` parameter is plain to a caller, so it needs no mark in a task type. All its parameters count, so a default does not apply through a task value.

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

A task inside a task is a nested task. It takes no type parameters, `self`, defaults, or `change` parameters. It is a value made at its line, so it can only be used after that line, and it can call itself.

```lucid
task total(n: int) -> int
    let step = 2
    task sum(x: int) -> int
        if x <= 0
            return 0
        return x + sum(x - step)
    return sum(n)
```

A lambda or nested task reads names around it by capturing their values when it is made, so it can outlive them. Each pass of a loop makes a new capture. A `var`, a `change` or `take` parameter, or a `for change` name cannot be captured, which is error E0325, since a later change would not be seen and a captured name cannot change: write `let copy = count` first. A captured name keeps the type it was narrowed to, and does not narrow further inside: test a `let` copy of it.

A task value has `Show`, and prints as its type. It has no `Equal`, `Order`, or `Hash`, so it cannot be a key, and a value that holds one cannot use `==`. In an `any`, `==` is true only for one value that was copied.

`x.f(a)` also calls a field of `x` that holds a task value, if no task named `f` takes `x`.

`...` is a placeholder statement that traps at run time. A planned `lucid build --release` will reject it.

## Parameter modes

Parameters are read-only views by default. Two keywords change this. See [memory-model.md](memory-model.md).

```lucid
task total(items: list[int]) -> int        # read only
task sort(change items: list[int])         # mutates caller's value
task consume(take items: list[int])        # owns the value, may mutate it

sort(change numbers)     # `change` is required at the call
```

A `change` argument is a `var`, or a path into one, such as `x.age` or `grid[i]`. Its type must equal the parameter's type exactly, since the task may store any value of that type. So a name narrowed to `int` can go to `change n: int`, and keeps its narrowing. A name that goes to a parameter of its declared type loses what tests proved. A `change` parameter has no default, so its argument goes by position, and a path runs its indexes once, in the order on the page.

A task can read a `change` parameter, assign it, and pass it on as `change`. The caller sees what it holds when the task ends.

A `take` parameter is the task's own value, and it can change. The caller keeps its own value and writes nothing at the call. See [memory-model.md](memory-model.md) for what this costs.

While a statement changes a variable through a `change` argument, no other part of that statement can use the same value, and two `change` arguments cannot overlap. Both are error E0326. An argument of the same call that gives a plain number or `bool` is allowed, as in `items.grow(items.len())`, since it is done before the call starts. `swap(change x[i], change x[j])` compiles, and traps if `i` and `j` are equal.

A statement passes a variable to one `change` call only, and cannot also write to it. Split it in two, and name the value with `let`.

## Types

A `type` holds named fields. Constructor arguments are always named.

```lucid
type User
    name: text
    age: int = 0

let ada = User(name: "Ada", age: 36)
print ada.name
```

Arguments may come in any order, and run in the order written. A field with no default must be given. A default is a literal, as for a parameter, so `items: list[int] = []` works.

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

task reset(change self: User)
    self = User(name: self.name)

if ada.is_adult()
    print "welcome"
```

`x.f(a)` runs the task `f` whose `self` type is the type of `x`. A `self` type can be an `any`, except for a task of its own trait. So one name can have one task per type, and such a name stays out of the shared namespace of "Choices". `f(x, a)` is an error, since one job has one form. `self` needs a type, which cannot be a bare type parameter, and `self: list[T]` covers every list. Two tasks of one name whose receivers can overlap are an error, counting each type parameter as free, and so is a built-in name on a built-in type, such as `len` on `text`. A dot call does not widen its receiver: an `int` does not reach `self: int?`. `x is empty` calls `x.is_empty()`.

In a package, a task with `self` needs its receiver's type, or a trait that names the task, declared in that package, or else it is error E0342. This is the orphan rule, by decisions 196 and 210. A built-in type belongs to `std`, and so do `list`, `map`, `set`, unions, and task types, whatever they hold, so `self: list[User]` is outside every package. An `any` belongs where its trait is declared. A file with no `lucid.toml` is exempt.

The receiver must be a `var`, or a path into one, when `self` is `change`, and it follows the rules for a `change` argument. The `change` keyword is not repeated on a dot call, but it is required on any other argument. `x is empty` cannot call an `is_empty` that changes `self`.

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
- **Loops:** a loop forgets anything about a name assigned in its body, or passed whole as a `change` argument, before it starts.
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

- `try x` and `x else y` act on the error types in the union of `x`. If the union has no error type, they act on `none`. So `none` from a `none or FileError` task counts as success. A value with nothing to act on, or with nothing left after it, is error E0327.
- `try x` unwraps the value, or returns the error from the current task or lambda. The return type must accept it, and a lambda must get its result type from its context. At top level of an entry file, it prints the error's `error_message()` to stderr and exits with code 1. For `none` it prints `none`.
- `x else y` gives `y` in place of the error, and runs `y` only then. `y` must have the type of what is left of `x`, or the type of the context if that accepts it: `let n: int? = parse(s) else none`.
- An unused error is error E0328: a statement that drops a value with an error type in it, or a `let` or `var` name that holds one and is never read. To ignore an error, say so: `save() else none`.

`Error` is a trait that every file has, so its name is taken. Use it as a bound, or as `any Error` for a task that can return any error type.

A **trap** is an unrecoverable bug: out-of-bounds index or slice, a missing map key, overflow, a zero divisor, a failed conversion, stack overflow, a reached `...`, failed `expect`, a `shared` access conflict, running out of memory, or a failed write to stdout. A trap prints its source position and exits with code 101, except that running out of memory and a failed write to stdout have no position. After the position, a stack trace lists up to 16 calls that led to the trap, innermost first, as `at file:line:column in name`. The list stops at a call made by the runtime, such as a written `equal` that a map runs. Traps cannot be caught.

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

A task of a trait takes `self` first, with no type. A type has the trait if it has a task of that name with the same other parameters, the same `change` parameters, and the same return type. An `any` cannot call a task with a `change` parameter, so `any Trait` does not have a trait that holds one. A bound names a trait, and `and` joins several: `[T: Show and Equal]`. Generic types take bounds too, as in `type Sorted[T: Order]`. A type that lacks a bound is error E0320 at the call, or in the type that names it.

Bounds use static dispatch. `any Describe` is a boxed value with dynamic dispatch, which allows mixed lists:

```lucid
let pets: list[any Describe] = [Dog(name: "Rex"), Cat(lives: 9)]
for pet in pets
    print pet.describe()
```

`any` names one trait, of your own, or `Show`, or `Error`. The other built-in traits ask for two values of one type, so `any Order` and the like are errors. A value goes in wherever `any Describe` is expected, if its type has `Describe`. A union counts as one type here: it needs a task of its own, or `any Show`, which every type has. An expected union with an `any` member takes a single value, and never a union. As with unions, this needs an expected type, and there is no conversion between collections or between two `any` types.

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
| `Json` | `json.parse` and `json.write` | `none`, `bool`, numbers, `text`, and lists, sets, maps with `text` keys, unions, and `type` or `choice` values made of those. See below |

The compiler derives `Equal`, `Hash`, `Order`, `Show`, `Copy`, and `Json` for a type when all its fields support them. Writing the task by hand overrides the derived version, and works inside larger values too. There are no macros. So far `Equal`, `Hash`, `Order`, `Show`, and `Json` are derived, by the rules in "Collections", "Types", and "Choices", and below. In a derived order a NaN compares as neither less nor more, so it counts as a tie. `Json` has no hand-written form yet. Planned: `Copy`.

`Json` lets `std.json` read and write a value:

| Type | JSON |
| --- | --- |
| `none` | `null` |
| `bool`, numbers, `text` | the same. An `int` type reads no number with a fraction or exponent, and a number that does not fit is an error. A NaN or infinity writes as `null` |
| `list`, `set` | an array. A repeated element of a set reads as one |
| `map` | an object. The key must be `text` |
| `type` | an object with one name per field |
| `choice` | `"Red"` for a variant with no fields, and `{"Circle": {"radius": 1.0}}` for one with fields |
| a union | what its member gives |

Reading skips names that the type lacks. A field that is left out takes its default, and with no default it reads `null`, which only a type that holds `none` accepts. If a name repeats, the last one counts. A union has `Json` only if no two members read from one kind of JSON value, so `int or text` has it, and `int or float` does not, nor does `text` with a `choice` that has a variant with no fields. So a read never tries a member and goes back. A mix that appears only through type arguments, as `T or U` with two number types, is not rejected: the first member, in the compiler's type order, whose kind of JSON value fits reads it, and no other member is tried. `JsonError` itself has no `Json`, since a read gives it for a failure. A task value, a `pointer`, an `any`, and a map with another key type have no `Json`.

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

These names replace derived tasks or give an operator, so each must have this form, on a `type`, `choice`, collection, or union such as `text?`. Any other form is error E0323.

`hash` gives what stands for the value in a key. It may be any type with `Hash`, and it often names the fields that `equal` reads. Equal values must give equal results, or a key may not be found again. A breach is a bug in the program, and it stays memory safe. The result cannot lead back to the same type, even through a list or a field, since hashing it would never end, and such a type has no `Hash`. A type with a hand-written `equal` has no `Hash` unless it also has a hand-written `hash`, since a derived hash would not match it.

Operators are defined by the traits `Add`, `Sub`, `Mul`, `Div`, `Equal`, and `Order`. No other operators can be overloaded, and no new operators can be defined.

## Modules

```lucid
use std.http
use std.json as js
use ./models
use ./models.User
use ./models.load as read_models
```

`use ./a.b` is relative to the file that holds it, and reads `a/b.lcd`. If that file does not exist, it takes the name `b` from `a.lcd`. A `./` path cannot go up a directory. A file that cannot be found is error E0334, and the spelling must match the file's name exactly, on every system.

`use std.a` is a module of the standard library, and `use std.a.b` takes the name `b` from it. `std` has no `./`, and needs no dependency entry. In a package, `std.fs`, `std.sql`, `std.env`, `std.net`, `std.http`, and `std.process` need a permission in `lucid.toml`, or else error E0340, see [security.md](security.md). A module not built yet reports E0901. The modules are in [stdlib-and-web.md](stdlib-and-web.md).

A whole module goes by its last name, or by the name after `as`, and its names need that prefix: `models.load()`, `x: models.User`. A taken name has no prefix. The name a `use` takes must be new in the file, or else error E0304. A taken name also must not be a built-in type or trait. A whole module may share a name with one, as `use std.sql` does with the type `sql`, since a module name is always followed by a dot. A local in a block may hide it, as with any top-level name. A `use` works in its own file only, and a module passes nothing on from its imports.

Names are private to their module unless marked `public`, on a task, `type`, `choice`, or `trait`. Using a private name from another module is error E0333. A public `type` shows all its fields, and a public `choice` shows all its variants. A variant is a name of its module, so build one with `models.Red`. A pattern needs no prefix or import, since the type of the subject finds the variant: `Red -> ...`. So no binding in a pattern can reuse the name of a type or variant of any module, or else error E0318.

A task with `self` also needs `public` before another module can call it, or else E0333. A private one still counts when a type must have a trait, and when it replaces a derived task, so a type means one thing in the whole program. A receiver type has one task of each name in the whole program.

Only the entry file runs code. An imported module holds declarations and `let` with a literal value, or else error E0336. So importing a module never has an effect, and order of imports does not matter. Its `test` blocks are dropped: run `lucid test` on that file or its package. Import cycles are error E0335.

## Packages

A package is a directory with a `lucid.toml` file and a `src` directory:

```toml
[package]
name = "blog"
version = "0.1.0"

[dependencies]
markdown = { path = "../markdown" }
```

`name` uses lowercase letters, digits, and `_`, and cannot be a keyword or `std`. `version` is three numbers. A dependency has a `path`, relative to the file that names it, or a `version` and no `path`, which comes from the registry that `[registry] url` names: an `https://` address or a directory. Its key must equal the name of the package that it reaches. The manifest also holds permissions, see [security.md](security.md). A manifest that breaks these rules is error E0338.

A package is a program, with `src/main.lcd`, or a library, with `src/lib.lcd`. It cannot be both, only a library can be a dependency, and only a program can run. `src/lib.lcd` follows the rule for imported modules, so it runs no code. A fault in the shape of a package or a dependency is error E0339.

```lucid
use markdown
use markdown.render as to_html
use markdown.parser.block
```

`use markdown` is the module `src/lib.lcd`. A longer path follows the rule of `use ./`, from `src`: `markdown.parser.block` reads `src/parser/block.lcd`, or else takes `block` from `src/parser.lcd`. A single name such as `markdown.render` reads `src/render.lcd`, or else takes `render` from `src/lib.lcd`. A name that the file's own package does not list is error E0337, even if another package depends on it. `public` works across packages as it does across modules.

Each package is one copy in a program, however many packages depend on it. Two packages with one name in a program are error E0339.

### Registry

```toml
[registry]
url = "https://packages.example.com"

[dependencies]
markdown = { version = "1.2.0" }
```

A `version` is a floor. By minimal version selection, decision 198, the program gets the highest floor that it or any of its dependencies names for each package, never a newer version than some manifest asks for. A floor reaches up to the next major version, or the next minor while the major is 0, so two floors in different ranges are error E0344. A registry package cannot have a `path` dependency, by decision 200.

`lucid.lock`, beside the root manifest, lists each registry package as `NAME VERSION SHA256`, the hash of its archive. Commands write it, and it belongs in version control. When the lock and the cache do not cover a dependency, `lucid run`, `check`, `test`, and `build` fetch what is missing, and say so on stderr. A registry that cannot give a package is error E0343, and an archive whose hash differs from the index or the lock is error E0345. `lucid package` writes a library's archive and its line of the index. See "Registry" in [compiler.md](compiler.md).

## Concurrency

Planned for M7, by decisions 169 to 176. `spawn`, `spawned`, waiting at the end of a task, `channel`, `lock`, and the deadlock trap are built.

`spawn f(x)` runs the call `f(x)` on a new thread, and gives a `spawned[T]`, where `T` is what `f` returns. Its arguments run first, in the current thread. `.wait()` blocks until the thread ends, and gives its result. Every spawn ends before the task, lambda, or top level that ran it returns: that one waits for each spawn it did not wait for. Nothing outlives its parent, so no thread leaks. Spawns in a loop run at once. `spawn` takes a call of a task, with no `change` argument, or else gives E0341. A `spawned[T]` shows as its type, such as `spawned[int]`, and cannot be compared.

```lucid
var jobs: list[spawned[text]] = []
for url in urls
    jobs.push(spawn fetch(url))
for job in jobs
    print job.wait()
```

A value that goes to another thread is a value like any other: changing it there never changes it here. So safe code cannot have data races. To communicate, use a channel, or a lock for state that threads share. Copying a channel or a lock shares it.

```lucid
let results = channel[int](16)     # holds at most 16 values
let total = lock(0)
spawn worker(results, total)
results.send(1)                    # blocks while full
results.close()
total.update(n => n + 1)           # runs while it holds the lock
print total.read()                 # a copy
```

`channel[T](capacity)` makes a channel. Where the context gives the type, as in `let c: channel[int] = channel(16)`, `channel(16)` is enough, and with neither it is E0324. `lock(value)` takes its type from the value, or from the context. `receive()` gives `T?`: the next value, or `none` once the channel is closed and empty. Like `.pop`, it must be a whole statement, or all of the value of `let`, `var`, `return`, or an assignment to a name. `send`, `close`, and `update` give no value, so each must be a statement, or else E0303. `send` after `close` traps, and so does a second `close`. A capacity below 1 traps. `update(f)` takes a `task(T) -> T`, and keeps what it gives. While `f` runs, the value is out of the lock, so a write in `f` copies nothing. `read()` waits while another thread runs `update`. A thread that takes a lock it already holds, as `f` would by reading its own lock, traps. When every thread waits in `wait`, `send`, `receive`, a lock, or the end of a task, the program traps with a deadlock. A trap in any thread ends the program. A channel and a lock show as their type, such as `lock[int]`, and cannot be compared.

There is no `async` or `await`. Tasks do not have colors: any task can block and any task can be spawned. Under `--target web`, `spawn` is error E0907, and a channel and a lock work with one thread.

## Shared references

Planned, not built: `shared` and `weak` parse, and report E0901. They are not part of the version 1.0 promise, and adding them later breaks no program, since both are keywords now. Until then, a graph uses indexes into a list.

Values are independent by default. For graphs, caches, and other shared structures, `shared` will give a counted cell. See [memory-model.md](memory-model.md).

```lucid
type TreeNode
    value: int
    children: list[shared TreeNode]
    parent: weak TreeNode?

var root = shared TreeNode(value: 1, children: [], parent: none)
var leaf = shared TreeNode(value: 2, children: [], parent: weak root)
```

## Tests

Tests live next to the code. `lucid run` drops them unread. `lucid check` and `lucid test` check them.

```lucid
test "adds numbers"
    expect add(2, 3) == 5
```

A test sits at top level, and its name is plain text, new in the file, with no `{}`. It is a task with no parameters, so it sees what a task sees. `lucid test` runs each test in file order, and runs no top-level statements. A test fails if it traps, if a `try` in it fails, as at top level, or if foreign code ends the program. A failure does not stop the tests after it, and a test shares no values with another. Output of a passing test is hidden.

`expect` takes a `bool`, traps when it is false, and also works outside tests. On a comparison such as `==` or `<`, each side runs once, and the trap shows both values: `expect failed: 5 < 4`. Text has quotes, as it does inside a list. Any other condition, or a side with no `Show`, gives `expect failed`. What passes narrows what follows, as an `if` that ends does: after `expect x is int`, `x` is an `int`.

## Foreign code

```lucid
unsafe extern task strlen(s: pointer[byte]) -> u64

var raw = "hello".bytes()
raw.push(0)

# `raw` ends in a zero byte, and stays unchanged for the call.
unsafe
    print strlen(raw.pointer())
```

An `extern` task is a C function, found by its name in the C library, which is the C runtime or `kernel32` on Windows, or libc or libm on Linux, or else error E0904. It takes and gives numbers, `bool`, and `pointer[byte]`, and may give nothing. It sits at top level, and it has no `change`, `take`, `self`, defaults, or type parameters. A C type must match the declared width: a C `int` is `i32`. A name that starts with `lucid_` is kept for the runtime, which only `std` can call.

Only an `unsafe` block can call an `extern` task or `.pointer()`, or else error E0330. An `extern` task is not a value, since a task type says nothing of `unsafe`. A lambda or nested task inside the block is outside it. The block is a block like the body of an `if`, with its own scope. It needs a `#` comment on the line above, or at the end of its line, or else error E0332, and the comment says why it is sound.

`pointer[byte]` is a raw address. It has no traits, so it does not print or compare, and it holds no count. `items.pointer()` gives the address of the first element of a `list[byte]` held in a name. The address is valid while that name is in scope and until the list changes or moves. C code that writes through it changes every holder of the list. Text has no pointer: use `.bytes()`, and push a zero byte for a C string.

Under `lucid build --target web`, a user `extern` task or `.pointer()` is error E0905, since a page reaches no C library.

In a package, an `unsafe` block or an `extern` declaration in any module needs the `unsafe` permission, or else error E0340. See [security.md](security.md).

## Keywords

`and` `any` `as` `break` `change` `choice` `continue` `else` `empty` `expect` `extern` `false` `for` `if` `in` `is` `let` `match` `none` `not` `or` `print` `public` `return` `self` `shared` `spawn` `take` `task` `test` `then` `trait` `true` `try` `type` `unsafe` `use` `var` `weak` `while`

Adding a keyword is a breaking change and needs a decision entry.
