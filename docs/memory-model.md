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

Types made of plain fields are stored inline on the stack or inside their parent. Only `list`, `map`, `set`, `text`, `any`, `shared`, and recursive data use the heap.

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

for item in items
    items.push(item)          # error: items changes while being iterated
```

Variables, fields, and list elements in `change` paths are checked statically. Overlap on list indexes that cannot be proven distinct, such as `swap(change x[i], change x[j])`, is checked at run time and traps. Use `x.swap(i, j)` to avoid the check.

## Inference

`change` and `take` always appear in signatures. Signatures are the contract, and agents rely on them. Inference happens inside bodies:

- Tasks in the same package: the compiler may pass ownership when that removes a count operation.
- Lambdas: non-escaping lambdas borrow captures. Escaping lambdas capture by move or by shared buffer.

## Shared references

`shared T` is a reference-counted heap cell. Copying it copies the reference, so writes are visible through all copies.

- **Creation:** `shared value` allocates a cell. `weak cell` makes a weak reference to it.
- **Mutation:** each write takes a short dynamic borrow. Conflicting access traps, like `RefCell` in Rust.
- **Cycles:** use `weak T` for back-references. Reading a `weak T` gives `(shared T)?`. Strong cycles leak memory, and the debug runtime reports them at exit.
- **Threads:** `shared` values cannot be sent to another task. Use `lock[T]` or `channel[T]`.

## Destruction

Values are destroyed deterministically when their last owner ends. As of M3 a local's owner ends when the local is overwritten or the task returns, not at the end of its block. That must tighten before `Close` arrives at M5. A type that has `task close(change self: T)` from the `Close` trait has it called automatically at that moment. Files and sockets close without `defer` or `with`.

## Reference count performance

- Counts are non-atomic by default.
- A value crossing a task boundary is either moved, or marked thread-shared so its counts turn atomic. Lean 4 uses this technique.
- Reuse analysis recycles a dying allocation of matching size for a new value.
- Borrow inference removes count pairs around read-only uses.

## Limits

These are known costs of this model, not bugs:

- A task cannot return a mutable view into a value. Built-in `[]` and `for change` cover most cases. Otherwise, return an index.
- Self-referential values need `shared` or indexes.
- Copy-on-write can hide O(n) costs. `lucid check --costs` exposes them, and CI can fail on them.
- Flat, cache-friendly layouts need deliberate type design. The language permits them, but does not automate them.

## Research risk

The published results for Perceus and Lean come from functional languages, with memory-focused benchmarks. Lucid applies them to code that looks imperative, and no production language has shipped this exact model. See [research.md](research.md).

This is the hardest part of the compiler, so it comes with a staged plan:

1. **M3: naive.** Count every use. Correct and slow. Each new value goes to a hidden local, which holds it until overwritten or until the task returns. A `for` over text holds one count on its text in another hidden local, so assigning the variable in the body is safe. A store then counts again, so a value is rarely unique yet, and copy-on-write will copy more than it must.
2. **M4: last-use moves and borrow elision.** Measure against Rust.
3. **M4: reuse analysis.** Only after measurements justify it.

If benchmarks miss the target in `vision.md` after these stages, revisit the model in `decisions.md`.

Correctness never depends on an optimization pass. Removing all of them must give the same output, only slower. The test suite must run in that mode.
