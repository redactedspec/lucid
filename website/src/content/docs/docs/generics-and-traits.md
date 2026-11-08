---
title: Generics and traits
description: Generic tasks and types, trait bounds, derived traits, operators, and any values.
---

## Generics

Tasks and types take type parameters in brackets. Each combination that a program uses gets its own compiled code, so generics cost nothing at run time.

```lucid
task first[T](items: list[T]) -> T?
    return items.get(0)

type Pair[A, B]
    left: A
    right: B

print first([1, 2])
let p = Pair[int, text](left: 1, right: "a")
print p
```

Type arguments come from the values, left to right. When the values do not show a type, name it: `first[int]([])`.

A generic body is checked once, where it is written. Without a bound, it can only store, pass, and return a `T`.

## Traits

A trait lists required tasks. Satisfaction is structural: any type with matching tasks has the trait, with no declaration.

```lucid
trait Describe
    task describe(self) -> text

type Dog
    name: text

task describe(self: Dog) -> text
    return "a dog called {self.name}"

task show_all[T: Describe](items: list[T])
    for item in items
        print item.describe()

show_all([Dog(name: "Rex")])
```

A bound names a trait, and `and` joins several: `[T: Show and Equal]`.

## Built-in traits

| Trait | Gives | Who has it |
| --- | --- | --- |
| `Show` | `print` and interpolation | every type |
| `Equal` | `==`, `!=`, `in` | every type except task values |
| `Order` | `<`, `<=`, `>`, `>=` | numbers, `text`, and values made of those |
| `Hash` | use as a map key or set value | integers, `bool`, `text`, and values made of those |
| `Add`, `Sub`, `Mul`, `Div` | `+`, `-`, `*`, `/` | numbers |
| `Json` | `std.json` read and write | most data types |

The compiler derives `Show`, `Equal`, `Order`, `Hash`, and `Json` for a type when its fields support them. Writing the task yourself replaces the derived one:

```lucid
type Money
    cents: int

task show(self: Money) -> text
    return "${self.cents // 100}"

task add(self: Money, other: Money) -> Money
    return Money(cents: self.cents + other.cents)

let total = Money(cents: 1250) + Money(cents: 300)
print total      # $15
```

Only arithmetic, equality, and ordering can be overloaded, and no new operators can be defined.

## any

A bound uses static dispatch. `any Trait` is a boxed value with dynamic dispatch, for mixed lists:

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

let pets: list[any Describe] = [Dog(name: "Rex"), Cat(lives: 9)]
for pet in pets
    print pet.describe()
```

An `any` value has only the tasks of its trait. Use a union if the code must know the type inside.
