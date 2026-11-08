---
title: Tasks and functions
description: Declaring tasks, defaults and named arguments, methods with self, task values, lambdas, and nested tasks.
---

## Tasks

A `task` is a function. Parameters always have types. With no `->`, a task returns `none`. Return types are never inferred, so a signature is always complete.

```lucid
task add(a: int, b: int) -> int
    return a + b

task log(message: text)
    print "log: {message}"

log("sum is {add(2, 3)}")
```

A task with a return type must return on every path. Top-level tasks may be used before their line. A parameter cannot be assigned.

## Defaults and named arguments

Arguments are positional. A parameter with a default comes last, and is passed by name:

```lucid
task connect(host: text, port: int = 443, secure: bool = true) -> text
    let scheme = if secure then "https" else "http"
    return "{scheme}://{host}:{port}"

print connect("example.com")
print connect("example.com", port: 8080, secure: false)
```

A default is a literal, such as a number, text, `none`, or an empty `[]`. Each call that uses an empty one gets a new collection.

## Methods

A task whose first parameter is `self` is called with a dot. There are no classes or `impl` blocks.

```lucid
type User
    name: text
    age: int = 0

task is_adult(self: User) -> bool
    return self.age >= 18

task birthday(change self: User)
    self.age += 1

var ada = User(name: "Ada", age: 17)
ada.birthday()
print ada.is_adult()     # true
```

`x.f()` and `f(x)` are not interchangeable: a task with `self` is called only with a dot.

In a package, `self` must be a `type` or `choice` of that package, or the task must be named by a trait of that package. This is the orphan rule, error E0342. It stops two packages from each adding a task of one name to `text`. A single file with no `lucid.toml` is exempt.

## Parameter modes

Parameters are read-only views by default, and passing a value never copies it. Two keywords change that:

```lucid
task total(items: list[int]) -> int         # reads only
    var sum = 0
    for n in items
        sum += n
    return sum

task double_all(change items: list[int])    # changes the caller's value
    for change n in items
        n *= 2

task with_extra(take items: list[int], extra: int) -> list[int]
    items.push(extra)                        # owns its copy, may change it
    return items

var numbers = [1, 2, 3]
double_all(change numbers)                   # change is written at the call
print total(numbers)                         # 12
print with_extra(numbers, 4)                 # [2, 4, 6, 4]
```

`change` must be written at the call, so a reader always sees what a call may modify. See [Ownership and memory](/docs/ownership/).

## Task values and lambdas

A task is a value. Its type is written `task(int) -> int`, or `task(text)` if it returns `none`. A lambda uses `=>` and holds one expression. Its parameter types come from the context.

```lucid
task apply(f: task(int) -> int, x: int) -> int
    return f(x)

task triple(x: int) -> int
    return x * 3

print apply(triple, 2)          # 6
print apply(x => x + 1, 2)      # 3

let plus: task(int, int) -> int = (a, b) => a + b
print plus(2, 3)
```

There is no built-in `.map` yet. Writing one takes a few lines:

```lucid
task map[T, U](items: list[T], f: task(T) -> U) -> list[U]
    var out: list[U] = []
    for item in items
        out.push(f(item))
    return out

print map([1, 2, 3], x => x * 2)
```

## Nested tasks

For anything longer than one expression, define a task inside a task. It can call itself, and is usable after its line.

```lucid
task total(n: int) -> int
    let step = 2
    task sum(x: int) -> int
        if x <= 0
            return 0
        return x + sum(x - step)
    return sum(n)

print total(6)    # 12
```

A lambda or nested task captures the values of names around it when it is made. It cannot capture a `var`: copy it to a `let` first.

## Placeholders

`...` is a statement that traps when reached. Use it for code not written yet.

```lucid
task later() -> int
    ...

print "later is not called"
```
