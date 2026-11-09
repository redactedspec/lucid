# Memory Model

Inferred ownership is the core technical bet of Lucid. Programmers write no lifetimes and run no garbage collector. Programs stay memory safe, and most of them stay fast.

The model combines three proven ideas:

- **Mutable value semantics**, as in Hylo and Swift value types.
- **Compile-time reference counting with reuse**, as in Perceus (Koka) and Lean 4.
- **The law of exclusivity**, as in Swift and Rust.

## The rules a programmer needs

1. **Every value is independent.** `var b = a` gives `b` its own logical value. Changing `b` never changes `a`.
2. **There are no stored references.** A field, list element, or variable holds a value, not a pointer to one. Without stored references, nothing can dangle and no lifetime annotation is needed.
3. **Parameters are read-only views by default.** Passing a value is free. It never copies.
4. **`change` gives exclusive access.** The caller's variable may be changed in place, and nothing else can touch it during that call.
5. **`shared` opts into reference semantics.** It is explicit, visible, and the only way to build a graph.

## What the compiler does

Logical copies are almost never real copies.

| Source | Compiler output |
| --- | --- |
| Value used for the last time | Move. No cost. |
| Value used again later, heap type | Reference count increment. Buffer is shared. |
| Write to a uniquely held buffer | In place. |
| Write to a shared buffer | Copy once (copy-on-write), then in place. |
| Small `Copy` value | Plain machine copy. |

Every `type` and `choice` value is a counted heap record, with copy-on-write like a list (decision 49), except a variant with no fields, which is static data. A union value is a record too, so each `int?` that holds a value costs an allocation (decision 57). Storing plain types inline has no milestone yet (decision 101).

### Example

```lucid
task add_all(take items: list[int], extra: int) -> list[int]
    items.push(extra)
    return items

let numbers = [1, 2, 3]
let more = add_all(numbers, 4)   # last use of numbers: moved, in place
```

If `numbers` is used after the call, the list is shared at the call and `push` copies once. The program stays correct either way. Only its cost changes, and `lucid check --costs` reports the copy.

## The exclusivity law

While a value has `change` access, no other access to it may happen. The compiler rejects these cases:

```lucid
swap(change a, change a)      # error: two exclusive uses of a
f(change a, a)                # error: a is read while it changes

for item in items
    items.push(item)          # error: items changes while being iterated

for change item in items
    print items.len()         # error: items is out of its place
```

Variables, fields, and list elements in `change` paths are checked statically, by error E0326. Overlap on list indexes that cannot be proven distinct, such as `swap(change x[i], change x[j])`, is checked at run time and traps. Use `x.swap(i, j)` to avoid the check. The same goes for map keys, and for paths such as `x[i].a` and `x[j].b`, where two equal indexes trap even though the fields differ.

The checks are stricter than the law, for two reasons. A counted value that a statement reads stays borrowed until that statement ends, so a read and a change of one value cannot share a statement, even in an order that would be safe. A plain number or `bool` holds no borrow, so another argument of the same call may read one. And a write reads its way down its path before the value runs, so a statement that writes to a variable cannot also pass another part of it as `change`. Loosening either rule later breaks no program.

`for` keeps a count of what it visits, so a change in its body would be safe and would change a copy. It is an error anyway, because that loop rarely means what it says, and the copy can cost O(n) with no sign on the page.

## Inference

`change` and `take` always appear in signatures. Signatures are the contract, and agents rely on them. Inference happens inside bodies:

- Tasks in the same package: the compiler may pass ownership when that removes a count operation.
- Lambdas: every lambda and nested task holds a count on each capture, like a field of a record. Non-escaping lambdas borrowing captures is planned, with no milestone yet.

## Shared references

Planned, not built: `shared` and `weak` report E0901, and stay outside the version 1.0 promise.

`shared T` will be a reference-counted heap cell. Copying it copies the reference, so writes are visible through all copies.

- **Creation:** `shared value` allocates a cell. `weak cell` makes a weak reference to it.
- **Mutation:** each write takes a short dynamic borrow. Conflicting access traps, like `RefCell` in Rust.
- **Cycles:** use `weak T` for back-references. Reading a `weak T` gives `(shared T)?`. Strong cycles leak memory, and the debug runtime reports them at exit.
- **Threads:** `shared` values cannot be sent to another thread. Use a `lock` or a `channel`.

## Task values

A task value is a counted record of what it captured, and holds no reference to any variable. Only a value that cannot change is captured, so a task value is an ordinary immutable value. It cannot reach itself, since a nested task reads its own name as its own record, and it stores no count on that. So task values form no cycles.

## Destruction

Values are destroyed deterministically when their last owner ends. Destroying never recurses, so a recursive value of any depth is safe to drop (decision 54). A name owns its value until the end of its block, newest name first, or until the value moves on, or until the name is assigned. `break`, `continue`, and `return` end the blocks they leave. A hidden value ends with its statement. A socket of `std.net` closes at that moment, since the runtime closes it when it frees it, by decision 177, so it needs no `defer` or `with`. A `std.sql` database closes the same way, once its last statement is done, by decision 194. Planned: a user type that has `task close(change self: T)` from a `Close` trait has it called then too.

## Reference count performance

- Counts are non-atomic by default.
- By decision 170, the sign of a count tells who holds it, as in Lean 4. Built in the runtime and compiled code. `spawn` marks what it passes, and so do `send` and a lock. A positive count belongs to one thread, and changes with plain instructions. A negative count is shared by threads, and changes atomically. A count of 0 is static data. A value that goes to another thread, through `spawn`, `send`, or a lock, is marked negative with everything it reaches, which costs its size once. A marked value with one count turns positive again before a write, so a list in a lock is written in place.
- Reuse analysis could recycle a dying allocation of matching size for a new value. It is not built, see decision 100.
- Borrow elision removes count pairs around read-only uses of a read parameter.

## Limits

These are known costs of this model, not bugs:

- A task cannot return a mutable view into a value. Built-in `[]` and `for change` cover most cases. Otherwise, return an index.
- Self-referential values need `shared` or indexes.
- Copy-on-write can hide O(n) costs. `lucid check --costs` exposes them, and CI can fail on them.
- Flat, cache-friendly layouts need deliberate type design. The language permits them, but does not automate them.

## Research risk

The published results for Perceus and Lean come from functional languages, with memory-focused benchmarks. Lucid applies them to code that looks imperative, and no production language has shipped this exact model. See [research.md](research.md).

This is the hardest part of the compiler, so it comes with a staged plan:

1. **M3: naive.** Count every use. Correct and slow. Each new value goes to a hidden local, which holds it until overwritten or until the task returns. A `for` over text or a collection holds one count on it in another hidden local, so nothing in the body can free it. A store then counts again, so a value is rarely unique yet, and copy-on-write copies more than it must. `var x = [1]` holds two counts, so the first write copies once. After that `x` holds its own copy, and writes happen in place. A write through a path such as `grid[i][j] = 1` takes `grid[i]` out of its slot, writes, and puts it back, so it copies nothing unless `grid[i]` is shared. A `for` lets go of its count when the loop ends. A write to a field works like a write to a list element. A `change` argument that is a path takes its value out of its slot for the call, in the same way, and any other value of that call goes to a hidden local first. `for change` takes the list out of its place for the whole loop, and each element out of the list while the body runs, so a shared list copies once, and each write is then in place. A `take` parameter is a local that takes its own count on entry, so a write to it copies once. A `match` holds one count on its subject in a hidden local until it ends. A union holds one count on what it contains. `try` and `else` hold their value in a hidden local until the statement ends, and on an early return the exit of the task releases it and each part already built. `.pop` reads the element first, so its value outlives the list's count. `.get` keeps its receiver and key in hidden locals only if they are not plain reads, so `x.get(i)` and `g[0].get(i)` add no count to the list. With a computed path such as `g[i + 1].get(0)`, the hidden local holds a count until it is overwritten, so the next write to that element copies it once.
2. **M4: last-use moves.** Done. A value goes where it is kept without a count operation if nothing reads it again, and each count is explicit in MIR. See below.
3. **M4: borrow elision.** Done, for locals that read a read parameter. See below.
4. **M4: reuse analysis.** Not built, since measurements did not justify it (decision 100).

Stage 2 runs on MIR after monomorphization. A local moves into a place that keeps it if it is not a parameter, nothing can read it after the statement, and the statement reads it once. The last rule is what keeps a borrow valid: in `f(x.name, take x)` the first argument borrows from `x`, so `x` is retained, not moved. A `take` argument is such a place, so a `take` parameter of a top-level task arrives with a count. A written `equal`, which the runtime calls, keeps the borrowed form with a retain on entry, since the runtime cannot pass a count. A task value owns its arguments, by decision 183, so a lambda or nested task that keeps its argument, or returns it changed, writes in place when the caller is done with it. Statements that one expression holds move nothing. By decision 222, `t = "{t}x"` moves `t` into the join even when `t` is a `change` parameter, since the assignment gives the place a new count right after, and `b.t = "{b.t}x"` takes the text out of its field for the join, as a `change` argument that is a path does, after the other parts run. So both append in place when the text is unique.

Stage 3 runs first. A local needs no count if every value it ever holds is a path into a read parameter, or into another such local, and nothing changes it. So `let left = pair.left`, a `for` over a parameter, and a `match` on one and its bound names add no count operation. The caller holds a read parameter and keeps it fixed for the whole call, so such a path stays valid. Keeping one of these values anywhere still adds a count there. A local of the top level or one that holds a new value keeps its count, for now. When `shared` arrives, a value read out of a cell must hold a count for as long as a call borrows it, since a write through another copy of the cell could otherwise free it.

The benchmarks met the targets in `vision.md` on four of five programs, and decision 101 records why the model stays for the fifth.

Correctness never depends on an optimization pass. Removing all of them must give the same output, only slower. The test suite must run in that mode.
