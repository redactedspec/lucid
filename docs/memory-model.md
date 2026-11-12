# The Lucid Memory Model

Lucid has no garbage collector and no lifetime annotations, and safe code
cannot touch freed memory. This document explains how. It is the part of
the design most likely to be wrong, so it also says where the model is
weak and what has been measured.

The model takes three ideas that have each worked elsewhere:

* mutable value semantics, from Hylo and Swift's value types;
* reference counting placed by the compiler, from Perceus in Koka and
  from Lean 4;
* the law of exclusivity, from Swift and Rust.

No production language has shipped this exact combination.

## What a programmer needs to know

Every value is independent. After `var b = a`, `b` holds its own logical
value, and changing `b` never changes `a`.

There are no stored references. A field, a list element, or a variable
holds a value, never a pointer to one. With nothing pointing, nothing can
dangle, and so there is nothing to annotate.

A parameter is a read-only view unless it says otherwise. Passing a value
never copies it.

`change` gives a task exclusive access to the caller's variable. The task
may change it in place, and nothing else may touch it for the length of
the call. The caller writes `change` too, so mutation is visible where it
happens.

`take` hands the value over. The task owns it and may change it without
affecting the caller.

That is all most programs need. The rest of this document is about what
the compiler does to make it fast, and the rules it enforces to keep it
safe.

## What the compiler does

A logical copy is almost never a real one.

| In the source | What runs |
| --- | --- |
| A value used for the last time | A move. Nothing is counted |
| A heap value used again later | Its count goes up, and the buffer is shared |
| A write to a buffer held once | In place |
| A write to a shared buffer | One copy, then in place |
| A small plain value such as an `int` | A machine copy |

Every `type` and `choice` value is a counted record on the heap, copied
on write like a list. A variant with no fields is the exception: it is
static data. A union is a record too, so each `int?` that holds a number
costs an allocation. Storing small types inline is planned, with no
milestone (decision 101).

```lucid
task add_all(take items: list[int], extra: int) -> list[int]
    items.push(extra)
    return items

let numbers = [1, 2, 3]
let more = add_all(numbers, 4)   # last use of numbers: moved, in place
```

If `numbers` were used after the call, the list would be shared at the
call and `push` would copy it once. The program gives the same answer
either way; only the cost differs, and `lucid check --costs` reports the
copy.

## The law of exclusivity

While a value is passed as `change`, no other access to it may happen.
The compiler rejects each of these:

```lucid
swap(change a, change a)      # error: two exclusive uses of a
f(change a, a)                # error: a is read while it changes

for item in items
    items.push(item)          # error: items changes while being iterated

for change item in items
    print items.len()         # error: items is out of its place
```

Variables, fields, and list elements on a `change` path are checked at
compile time, as error E0326. Two list indexes that cannot be proven
different, as in `swap(change x[i], change x[j])`, are checked at run
time, and the program traps if they are equal. `x.swap(i, j)` needs no
check. Map keys work the same way, and so do paths such as `x[i].a` and
`x[j].b`: equal indexes trap even though the fields differ.

The checks are stricter than the law, in two ways.

A counted value that a statement reads stays borrowed until the
statement ends, so a read and a change of the same value cannot share a
statement, even in an order that would be safe. A plain number or `bool`
holds no borrow, so another argument of the same call may read one.

A write reads its way down its path before the right-hand side runs, so a
statement that writes to a variable cannot also pass another part of that
variable as `change`.

Loosening either rule later would break no program.

`for` holds a count on what it walks, so changing the collection in the
body would be safe: it would change a copy. It is an error anyway,
because such a loop rarely means what it says, and the copy can cost
O(n) with nothing on the page to show it.

## Inference

`change` and `take` always appear in signatures. The signature is the
contract, and agents read it. Inference happens only inside bodies.

Between tasks of the same package, the compiler may pass ownership where
that removes a count operation.

Every lambda and nested task holds a count on each value it captures,
like a field of a record. Letting a lambda that does not escape borrow
its captures instead is planned, with no milestone.

## Task values

A task value is a counted record of what it captured. It holds no
reference to any variable. Only values that cannot change are captured,
so a task value is an ordinary immutable value. A nested task reads its
own name as its own record and keeps no count on it, so task values
cannot reach themselves and form no cycles.

## Destruction

A value is destroyed when its last owner ends, at a point the program can
predict. Destruction never recurses, so a list of a million nested
records is as safe to drop as one (decision 54).

A name owns its value until the end of its block, newest name first, or
until the value moves out, or until the name is assigned again. `break`,
`continue`, and `return` end the blocks they leave. A value the program
never named ends with its statement.

Resources ride on this. A socket from `std.net` closes when it is freed,
and a `std.sql` database closes once its last statement is done, so
neither needs `defer` or `with`. Calling a user type's `close` from a
`Close` trait at that point is planned.

## Reference counts

Counts are not atomic unless they have to be. As in Lean 4, the sign of
a count says who holds it (decision 170):

* a positive count belongs to one thread and changes with plain
  instructions;
* a negative count is shared between threads and changes atomically;
* a count of zero marks static data.

A value that goes to another thread, by `spawn`, `send`, or a lock, is
marked negative along with everything it reaches. That costs one walk of
its size. A marked value with a count of one turns positive again before
a write, so a list kept in a lock is still written in place.

Borrow elision removes the count pairs around read-only uses of a read
parameter.

Reusing a dying allocation for a new value of the same size, as Perceus
does, is not built. The measurements did not justify it (decision 100).

## Shared references

Planned, not built. `shared` and `weak` give error E0901 today, and are
outside the version 1.0 promise.

`shared T` will be a counted heap cell. Copying it copies the reference,
so a write through one copy is seen through all of them. `shared value`
makes a cell, and `weak cell` makes a weak reference to it. Each write
takes a short dynamic borrow, and a conflicting access traps, as with
`RefCell` in Rust. A back-reference should be `weak T`, which reads as
`(shared T)?`; a strong cycle leaks, and the debug runtime reports it at
exit. A `shared` value cannot go to another thread. Use a `lock` or a
`channel`.

## Limits

These are costs of the model, not bugs.

* A task cannot return a mutable view into a value. `[]` and `for
  change` cover most needs; otherwise return an index.
* A value that refers to itself needs `shared` or indexes.
* Copy-on-write can hide an O(n) cost. `lucid check --costs` shows it,
  and a build script can fail on it.
* A flat, cache-friendly layout takes deliberate type design. The
  language allows it but does not do it for you.

## How the compiler gets there

The published results for Perceus and Lean come from functional
languages and from benchmarks chosen to stress memory. Lucid applies the
same ideas to code that looks imperative. Because this is the hardest
part of the compiler, it was built in stages, each correct on its own.

**Stage 1, at M3: count everything.** Correct and slow. Each new value
goes to a hidden local, which holds it until it is overwritten or the
task returns. A store counts again, so values are seldom unique and
copy-on-write copies more than it must. `var x = [1]` holds two counts,
so the first write copies once; after that `x` has its own copy and
writes are in place.

Some details of this stage still hold:

* A write through a path such as `grid[i][j] = 1` takes `grid[i]` out of
  its slot, writes, and puts it back, so nothing is copied unless
  `grid[i]` is shared. A write to a field works the same way.
* A `change` argument that is a path is taken out of its slot for the
  call, and the call's other arguments go to hidden locals first.
* `for change` takes the list out of its place for the whole loop, and
  each element out of the list while the body runs. A shared list is
  copied once, and every write after that is in place.
* A `for` over text or a collection holds one count on it, so nothing in
  the body can free it, and lets go when the loop ends.
* A `take` parameter takes its own count on entry, so writing to it
  copies once.
* A `match` holds one count on its subject until it ends. A union holds
  one count on what it contains.
* `try` and `else` hold their value until the statement ends. On an
  early return, the task's exit releases it and every part already
  built.
* `.pop` reads the element first, so its value outlives the list's count.
* `.get` puts its receiver and key in hidden locals only when they are
  not plain reads, so `x.get(i)` and `g[0].get(i)` add no count. With a
  computed path such as `g[i + 1].get(0)`, the hidden local keeps a count
  until it is overwritten, so the next write to that element copies once.

**Stage 2, at M4: last-use moves.** This runs on MIR after
monomorphization. A local moves, with no count operation, into a place
that keeps it if it is not a parameter, nothing reads it after the
statement, and the statement reads it once. The last condition keeps
borrows valid: in `f(x.name, take x)` the first argument borrows from
`x`, so `x` is retained, not moved.

A `take` argument is such a place, so a `take` parameter of a top-level
task arrives with a count. A user-written `equal`, which the runtime
calls, keeps the borrowed form and retains on entry, since the runtime
cannot hand over a count. A task value owns its arguments (decision 183),
so a lambda that keeps its argument, or returns it changed, writes in
place when the caller is done with it. Statements held inside one
expression move nothing.

Text appends get special care (decision 222). `t = "{t}x"` moves `t` into
the join even when `t` is a `change` parameter, since the assignment gives
the place a fresh count right after. `b.t = "{b.t}x"` takes the text out
of its field for the join, after the other parts run, the way a `change`
argument that is a path does. Both append in place when the text is
unique.

**Stage 3, at M4: borrow elision.** Though numbered last, it runs before stage 2. A local needs no
count if every value it ever holds is a path into a read parameter, or
into another such local, and nothing changes it. So `let left =
pair.left`, a `for` over a parameter, and a `match` on one and the names
it binds add no count operations. The caller holds a read parameter
fixed for the whole call, so such a path stays valid. Storing one of
these values anywhere still counts it there. Top-level locals and locals
holding a new value keep their counts, for now. When `shared` arrives, a
value read out of a cell will need a count for as long as a call borrows
it, since a write through another copy of the cell could otherwise free
it.

**Reuse analysis** was planned for M4 and not built.

The benchmarks met the targets in [vision.md](vision.md#measures) on four
programs of five; decision 101 records why the model stays for the fifth.
The numbers are in [research.md](research.md#m4-benchmarks).

Correctness never depends on an optimization. With every optimization
pass removed, a program must give the same output, only slower, and the
test suite must pass in that mode.
