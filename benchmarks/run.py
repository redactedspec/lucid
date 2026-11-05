"""Paired benchmark rounds. Run from the repository root, after run.sh built
everything. Each round runs every build once, so machine drift hits all equally.

Prints the best and median milliseconds for each build. Ratios are against
`rust`, and `lucid/checked` compares builds with the same overflow behavior.
"""

import os
import statistics
import subprocess
import sys
import time

ROUNDS = 15
exe = ".exe" if os.name == "nt" else ""


def path(*parts):
    return os.path.abspath(os.path.join(*parts))


def once(command):
    start = time.perf_counter()
    subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
    return (time.perf_counter() - start) * 1000


def builds(name):
    source = f"benchmarks/{name}.lcd"
    found = {
        "rust": [path("target/bench", name + exe)],
        "checked": [path("target/bench", f"{name}-checked{exe}")],
        "lucid": [path("target/release", "lucid" + exe), "run", source],
        "unchecked": [path("target/bench", "lucid-unchecked" + exe), "run", source],
    }
    return {label: command for label, command in found.items() if os.path.exists(command[0])}


for name in sys.argv[1:] or ["fib", "collatz", "sum"]:
    commands = builds(name)
    times = {label: [] for label in commands}
    for _ in range(ROUNDS):
        for label, command in commands.items():
            times[label].append(once(command))
    best = {label: min(values) for label, values in times.items()}
    median = {label: statistics.median(values) for label, values in times.items()}
    for label in commands:
        print(f"{name} {label}: best {best[label]:.0f} ms, median {median[label]:.0f} ms")
    for table, title in ((best, "best"), (median, "median")):
        for base in ("rust", "checked"):
            if base in table:
                print(f"{name} lucid/{base} {title}: {table['lucid'] / table[base]:.2f}")
