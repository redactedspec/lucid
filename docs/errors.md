# Error Codes

Every compile error, with its cause, an example, and a fix. `lucid explain CODE` prints one entry, and `lucid explain --json CODE` gives it as data. The compiler holds this file, so each entry must keep the same shape: a heading of the code and its meaning, the cause, an example in a `lucid` block if one file can show it, and a line that starts with `Fix:`. A golden test checks that each example gives its code.

The ranges are: `E00xx` files, `E01xx` lexer, `E02xx` parser, `E03xx` names and types, `E09xx` compiler limits and features not built yet. Codes never change meaning once released.

## E0001: Cannot read the file

The path does not name a file that can be read, or names a directory with no `lucid.toml` at or above it.

Fix: check the path, and run from the directory that the path is relative to.

## E0002: File is not valid UTF-8

Source must be UTF-8. The file holds bytes that are not, often from an editor that saved it in a legacy encoding.

Fix: save the file as UTF-8.

## E0101: Tab in indentation

Indentation is spaces only, so that every tool sees the same depth.

Fix: replace each tab at the start of a line with 4 spaces.

## E0102: Indentation width is wrong

Each block is exactly 4 spaces deeper than the line that opens it.

```lucid
if true
  print "x"
```

Fix: indent the block by 4 spaces more than its opening line.

## E0103: Unexpected character

The character starts no token in Lucid.

```lucid
let a = 1 $ 2
```

Fix: remove the character, or put it inside text.

## E0104: Text is not closed

A `"` starts text, and the line ends before the closing `"`.

```lucid
print "abc
```

Fix: close the text on the same line, or use `"""` for text over many lines.

## E0105: Unknown escape

Text knows only the escapes `\n`, `\t`, `\r`, `\\`, and `\"`.

```lucid
print "a\qb"
```

Fix: write `\\` for a backslash, or use one of the known escapes.

## E0106: Single `}` in text

`{` and `}` in text mark interpolation, so a lone `}` is ambiguous.

```lucid
print "a}b"
```

Fix: write `}}` for a literal `}`, and `{{` for a literal `{`.

## E0107: Malformed number

A number is digits, with `_` only between digits, an optional fraction, and an optional exponent. A prefix such as `0x` needs digits after it.

```lucid
let a = 0x
```

Fix: complete the number, as in `0xff`, or remove the stray part.

## E0108: Text after an opening `"""`

Text over many lines starts on the line after `"""`.

```lucid
print """abc
    """
```

Fix: end the line after the opening `"""`, and start the text on the next line.

## E0201: Unexpected token

The parser expected one kind of token and found another. The message says what it expected.

```lucid
let = 1
```

Fix: write what the message expects at that place.

## E0202: Invalid assignment target

Only a name, a field, or an index can take a new value.

```lucid
1 + 2 = 3
```

Fix: assign to a name, as in `let a = 1 + 2`.

## E0203: `match` used as a value has a block arm

A `match` that gives a value needs one expression in each arm.

```lucid
let b = 1
let a = match b
    _ ->
        print "x"
```

Fix: make each arm one expression, or use a `match` statement that assigns to a `var`.

## E0204: Chained comparison or range

`a < b < c` reads differently in different languages, so Lucid rejects it.

```lucid
let a = 1 < 2 < 3
```

Fix: join comparisons with `and`, as in `1 < 2 and 2 < 3`.

## E0205: Nesting over 128 levels, or over 1024 operators in one expression

The compiler limits depth so that deep input cannot overflow its stack.

Fix: split the expression or the blocks into smaller tasks or named values.

## E0206: Declaration only allowed at top level

`use`, `type`, `choice`, `trait`, `test`, and `extern` tasks sit at top level only.

```lucid
if true
    use std.fs
```

Fix: move the declaration to the top level of the file.

## E0301: Name is not defined

No name in scope has this spelling. A note names a close match, if one exists.

```lucid
let total = 1
print totl
```

Fix: correct the spelling, or define the name before its use.

## E0302: Type is not defined

No type in scope has this name.

```lucid
let a: Thing = 1
```

Fix: correct the spelling, declare the type, or `use` the module that has it.

## E0303: Types do not fit

A value has one type where another is needed. The message names both.

```lucid
let a: int = "one"
```

Fix: give a value of the needed type, or convert it, as in `text.to_int()` or `int.to_text()`.

## E0304: Name is already defined in this scope

Two declarations in one scope use the same name.

```lucid
let a = 1
let a = 2
```

Fix: rename one of them, or use `var` and assign the new value.

## E0305: Name cannot change

The target of an assignment or `change` is a `let`, a parameter that is not `change`, or a slice.

```lucid
let a = 1
a = 2
```

Fix: declare it with `var`, or mark the parameter `change`.

## E0306: Task uses a top-level `var`

A task sees top-level `let` values, but not `var` ones, so that every change shows in a signature.

```lucid
var count = 0

task f() -> int
    return count
```

Fix: pass the value as a parameter, with `change` if the task changes it.

## E0307: Arguments or parameters are in the wrong form

A call has missing, extra, repeated, or misnamed arguments, or a parameter has no default after one that has one.

```lucid
task f(a: int) -> int
    return a

print f()
```

Fix: match the call to the parameters, by position or by name.

## E0308: Called value is not a task

Only a task or a task value can be called.

```lucid
let a = 1
print a(2)
```

Fix: call a task, or remove the parentheses.

## E0309: Task may end without returning

A task with a result type must return on every path.

```lucid
task f(a: bool) -> int
    if a
        return 1
```

Fix: add a `return` at the end, or an `else` branch that returns.

## E0310: `return` outside a task

Top-level code is not a task, so it has nothing to return from.

```lucid
return
```

Fix: remove the `return`, or move the code into a task.

## E0311: `break` or `continue` outside a loop

These only work inside `while` or `for`, in the same task.

```lucid
break
```

Fix: remove it, or move it into a loop.

## E0312: Number does not fit its type

A literal is outside the range of its type.

```lucid
let a = 9223372036854775808
```

Fix: use a smaller number, or a type with a larger range, such as `u64` or `float`.

## E0313: Default is not a literal

A parameter default must be a literal value, so that a signature tells everything.

```lucid
task f(a: int = 1 + 1)
    return
```

Fix: write the value as one literal, as in `a: int = 2`.

## E0314: Empty list or map has no type to take

`[]` and `{}` need a type from an annotation or from where they go.

```lucid
let a = []
```

Fix: annotate it, as in `let a: list[int] = []`.

## E0315: Alias is defined by itself, or a type holds itself with no way to end

A recursive type needs a variant or an optional field that ends it.

```lucid
type Loop
    next: Loop
```

Fix: make the field optional, as in `next: Loop?`, or add a `choice` variant with no fields.

## E0316: `match` does not cover every case

Every variant or value must have an arm, so that adding a variant shows each place to update.

```lucid
choice Light
    Red
    Green

let light = Red
match light
    Red -> print "stop"
```

Fix: add the missing arms, or a final `_` arm.

## E0317: `match` arm can never run

An earlier arm already covers everything this arm would.

```lucid
let a = 1
match a
    _ -> print "any"
    1 -> print "one"
```

Fix: remove the arm, or move it above the arm that covers it.

## E0318: Pattern does not fit its value

The pattern names an unknown variant, has the wrong number of names, binds a type or variant name, holds `{}` in text, or names a type that the value never has.

```lucid
choice Shape
    Circle(radius: float)
    Square(side: float)

let shape = Square(side: 1.0)
match shape
    Circl(r) -> print r
    _ -> print "other"
```

Fix: spell the variant as declared, with one name per field.

## E0319: Type argument cannot be told from the values

A generic task must find each type parameter from its arguments, or be given it.

```lucid
task make[T]() -> list[T]
    return []

let a = make()
```

Fix: give the type in brackets, as in `make[int]()`.

## E0320: Type does not have a trait that a bound or operator needs

An operator or a bound needs a trait, such as `Equal` for `==` and `in`, or `Order` for `<`.

```lucid
task has[T](value: T, items: list[T]) -> bool
    return value in items
```

Fix: add the bound, as in `[T: Equal]`, or use a type that has the trait.

## E0321: Wrong number of type arguments

A generic name takes a fixed number of type arguments.

```lucid
task plain(a: int)
    return

plain[int](1)
```

Fix: give as many type arguments as the declaration has parameters.

## E0322: Type form where a value belongs, or a value where a type belongs

A type such as `int?` was used as a value, or a value was used in type brackets.

```lucid
let a = 1
print a?
```

Fix: put a type only in an annotation or in brackets, and a value only in an expression.

## E0323: Task named after a derived task or operator has the wrong form

A hand-written `show`, `equal`, `compare`, or `hash` replaces a derived one, so it must have that task's parameters and result.

```lucid
type Point
    x: int

task show(self: Point) -> int
    return self.x
```

Fix: use the form of the derived task, here `task show(self: Point) -> text`, or pick another name.

## E0324: Lambda has no task type to take from its context, or has `try` and no result type from it

A lambda takes its parameter types from where it goes, such as a parameter of task type.

```lucid
let double = n => n * 2
```

Fix: annotate the name, as in `let double: task(int) -> int = n => n * 2`, or write a nested task.

## E0325: Lambda or nested task captures a name that can change

A capture copies a value, so capturing a `var` would hide later changes.

```lucid
var n = 1
let add: task(int) -> int = m => m + n
```

Fix: copy the value into a `let` first, and capture that.

## E0326: A value is used while something changes it

Two `change` arguments overlap, a statement uses what it changes, a `for` body changes what it visits, or a value is read while a `for change` loop holds it.

```lucid
task swap(change a: int, change b: int)
    let c = a
    a = b
    b = c

var x = 1
swap(change x, change x)
```

Fix: pass separate values, or copy one into a new name first.

## E0327: `try` or `else` on a value with no error type or `none` to act on, or with nothing left after it

`try` and `else` act on a value whose type has an error or `none` in it.

```lucid
let a = 1 else 2
```

Fix: remove the `try` or `else`, since the value cannot fail.

## E0328: An error that nothing uses

A result that may be an error must be handled, so a failure is never lost.

```lucid
use std.fs

fs.write_text("out.txt", "hi")
```

Fix: handle it with `try`, `else`, or a `match`.

## E0329: A `test` name holds `{}`, or repeats

A test name is plain text, and new in its file, so that a report names one test.

```lucid
test "adds"
    expect 1 + 1 == 2

test "adds"
    expect 2 + 2 == 4
```

Fix: give each test its own name, with no `{}`.

## E0330: A call to an `extern` task or `.pointer()` is outside an `unsafe` block

Foreign code can break every guarantee, so each use is marked.

```lucid
unsafe extern task strlen(text: pointer[byte]) -> int

let bytes = "abc".bytes()
print strlen(bytes.pointer())
```

Fix: put the call in an `unsafe` block with a comment that says why it is safe.

## E0331: An `extern` task has a form that C cannot take

An `extern` task takes and gives only numbers, `bool`, and `pointer[byte]`, sits at top level, is not used as a value, and has one form in the whole program. `.pointer()` works only on a `list[byte]` name.

```lucid
unsafe extern task strlen(text: text) -> int
```

Fix: use a type that C can take, such as `pointer[byte]` from `.pointer()`.

## E0332: An `unsafe` block has no comment

The comment on the line before an `unsafe` block says why the code in it is safe.

```lucid
unsafe extern task abs(n: i32) -> i32

unsafe
    print abs(-1)
```

Fix: add a `#` comment on the line before `unsafe`.

## E0333: A private name or task with `self` is used from another module

A module shares only what is marked `public`.

Fix: mark the declaration `public`, or use what the module already shares.

## E0334: The file of a `use` does not exist, or is spelled with other letter case

A `use ./a.b` reads `a/b.lcd` from the directory of the file. Letter case must match the disk, so the program works on every system.

Fix: correct the path, or match the letter case of the file on disk.

## E0335: Modules import each other in a cycle

Imports must form a tree, so a module can load with no partial state.

Fix: move what both modules need into a third module that both import.

## E0336: An imported module holds something other than declarations and literal `let` values

Only the entry file runs code, so importing a module never has an effect.

Fix: move the statements to the entry file, or into a task.

## E0337: A `use` with no `./` names something that is not a dependency of the file's package

`use name` means `std` or a dependency in the package's own `lucid.toml`.

Fix: add the dependency to `lucid.toml`, or write `./` for a local file.

## E0338: `lucid.toml` is not valid

The manifest has bad syntax, a form outside the supported TOML, an unknown key, or a bad name or version.

Fix: correct the line the message names. A manifest holds `[package]` with `name` and `version`, and `[dependencies]`.

## E0339: A package has no entry file or two, or a dependency is wrong

A package has `src/main.lcd` or `src/lib.lcd`, not both. A dependency must exist, have the name it is given, be a library, and not share a name with another package. `lucid run` cannot run a library.

Fix: follow the message: add or remove an entry file, or correct the dependency's `path` or name.

## E0901: Parses, but cannot run at this milestone

The feature is in the language, but not built yet. `lucid check` does not report this code.

```lucid
let a = [1, 2]
let b = {1: 2}[0..1]
```

Fix: see `docs/roadmap.md` for when it arrives, and use a built feature until then.

## E0902: Task frame is too large for the stack check

A task's frame is over 512 KiB, or over half the stack for top-level code. Only code generation finds this, so `lucid check` does not report it.

Fix: split the task into smaller tasks.

## E0903: Too many generic instances

Over 20,000 instances, or a type argument of over 256 parts, usually from a generic task that calls itself with a larger type.

```lucid
task grow[T](value: T) -> int
    return grow([value])

print grow(1)
```

Fix: call the task with the same type it was given, or remove the type growth.

## E0904: No library has this foreign task

An `extern` task must be in the C runtime or `kernel32`.

```lucid
unsafe extern task no_such_symbol_anywhere(x: int) -> int
```

Fix: correct the name, or use a task that one of those libraries has.
