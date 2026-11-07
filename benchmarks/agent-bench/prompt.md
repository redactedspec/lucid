# Subject Prompt

Each subject is a new Claude Code subagent, given this prompt with `ARM`, `FROM`, `TO`, and `WORKSPACE` filled in. One subject does one block of 10 tasks in one arm.

---

You are a subject in a programming benchmark. Work only in `WORKSPACE`. Do not read any file outside it, and do not search the web.

Write tasks `FROM` to `TO` of `tasks.md` in ARM_LANGUAGE, in that order. GUIDE_LINE

For every attempt at a task, run `sh try.sh ARM NN`, with `NN` as two digits. It checks and runs your program, logs the attempt, and shows any error or difference in the output. Never run your program another way, and never change `try.sh`, `expected/`, or `attempts/`. Do not read `expected/`.

If an attempt fails, fix the program and run `try.sh` again. Stop after 5 attempts at one task, and go on to the next.

When you finish, reply with: each task that never passed, and why. Then list each thing you had to guess about the language, and each error message that did not help you.

---

| Arm | `ARM` | `ARM_LANGUAGE` | `GUIDE_LINE` |
| --- | --- | --- | --- |
| Lucid | `lucid` | Lucid | Learn Lucid only from `guide.md`. You may run `./lucid` for more, such as `./lucid explain E0303`. |
| Lucid, bare guide | `lucid` | Lucid | The same, but `guide.md` comes from `strip_guide.py`. |
| Python | `python` | Python 3 | (empty) |
| TypeScript | `ts` | TypeScript | `try.sh` checks it with `tsc --strict` and runs it with Node. |

A workspace holds `tasks.md`, `try.sh`, `expected/`, and, for a Lucid arm, `guide.md` and the debug `lucid` binary. Each block gets a new workspace outside the repository, so no subject sees another's work.
