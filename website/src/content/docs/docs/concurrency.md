---
title: Concurrency
description: Threads with spawn, channels, locks, and the deadlock trap.
---

Lucid uses operating system threads. There is no `async` or `await`, and tasks have no colors: any task can block, and any task can be spawned.

## spawn

`spawn f(x)` runs the call on a new thread, and gives a `spawned[T]`, where `T` is what `f` returns. `.wait()` blocks until the thread ends, and gives its result.

```lucid
task square(n: int) -> int
    return n * n

var jobs: list[spawned[int]] = []
for n in [1, 2, 3]
    jobs.push(spawn square(n))
for job in jobs
    print job.wait()
```

The arguments run first, on the current thread, and the new thread gets copies. Every spawn ends before the task that started it returns: that task waits for each thread it did not wait for. So no thread outlives its parent, and none leaks.

`spawn` takes one call of a task, with no `change` argument.

## No data races

A value that goes to another thread is a value like any other: changing it there never changes it here. Threads share state only through a `channel` or a `lock`, so safe code cannot have a data race.

## Channels and locks

```lucid
task worker(jobs: channel[int], total: lock[int])
    while true
        let job = jobs.receive()       # int?, none once closed and empty
        if job is none
            break
        total.update(n => n + job)    # runs while it holds the lock

task sum(count: int) -> int
    let jobs = channel[int](16)       # send blocks while 16 values wait
    let total = lock(0)
    var workers: list[spawned[none]] = []
    for i in 0..4
        workers.push(spawn worker(jobs, total))
    for n in 1..count + 1
        jobs.send(n)
    jobs.close()
    for w in workers
        w.wait()                      # every job is counted
    return total.read()

print sum(100)
```

- Copying a channel or a lock shares it. A value sent through a channel, or kept in a lock, is a copy.
- `receive()` gives the next value, or `none` once the channel is closed and empty.
- `update(f)` runs `f` while it holds the lock, and keeps what `f` gives. `read()` gives a copy.
- Sending after `close`, closing twice, or reading a lock inside its own `update` traps.

`sum` waits for its workers before it reads the total. Even without those waits, it could not return before they end, since a task waits for every thread it spawned.

## Deadlocks and traps

When every thread waits, in `wait`, `send`, `receive`, a lock, or the end of a task, the program traps with `deadlock: every thread waits`, rather than hanging.

A trap in any thread ends the whole program. Run a server under a process supervisor that restarts it.

## On the web

Under `lucid build --target web`, `spawn` is error E0907, since a page has one thread. Channels and locks still work there, with that one thread.
