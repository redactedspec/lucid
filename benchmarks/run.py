"""Paired benchmark rounds. Run from the repository root, after run.sh built
everything. Each round runs every build once, so machine drift hits all equally.

Prints the best and median milliseconds for each build, and peak memory. Ratios
are against `rust`, and `lucid/checked` compares builds with the same overflow
behavior. `lucid run` holds the compiler in memory, so the Lucid figure has the
peak of an empty program taken off. That figure is close to what `lucid build`
will use, and it needs replacing once that command exists at M5.
"""

import ctypes
import ctypes.wintypes
import os
import statistics
import subprocess
import sys
import time

ROUNDS = int(os.environ.get("ROUNDS", 15))
exe = ".exe" if os.name == "nt" else ""


def path(*parts):
    return os.path.abspath(os.path.join(*parts))


class Memory(ctypes.Structure):
    _fields_ = [
        ("cb", ctypes.wintypes.DWORD),
        ("PageFaultCount", ctypes.wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("rest", ctypes.c_size_t * 7),
    ]


def peak(process):
    """Peak bytes of a finished process.

    ponytail: Windows only, the one development machine. Add `os.wait4` at M8.
    """
    memory = Memory(cb=ctypes.sizeof(Memory))
    handle = ctypes.wintypes.HANDLE(process._handle)
    ctypes.windll.psapi.GetProcessMemoryInfo(handle, ctypes.byref(memory), memory.cb)
    return memory.PeakWorkingSetSize


def once(command):
    start = time.perf_counter()
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL)
    if process.wait() != 0:
        raise SystemExit(f"{command} failed")
    return (time.perf_counter() - start) * 1000, peak(process)


def builds(name):
    source = f"benchmarks/{name}.lcd"
    found = {
        "rust": [path("target/bench", name + exe)],
        "checked": [path("target/bench", f"{name}-checked{exe}")],
        "lucid": [path("target/release", "lucid" + exe), "run", source],
        "unchecked": [path("target/bench", "lucid-unchecked" + exe), "run", source],
    }
    return {label: command for label, command in found.items() if os.path.exists(command[0])}


NAMES = ["fib", "collatz", "sum", "nbody", "trees", "json", "hashmap", "strings"]
compiler = once([path("target/release", "lucid" + exe), "run", "tests/run/empty.lcd"])[1]

for name in sys.argv[1:] or NAMES:
    commands = builds(name)
    times = {label: [] for label in commands}
    memory = {}
    for _ in range(ROUNDS):
        for label, command in commands.items():
            spent, used = once(command)
            times[label].append(spent)
            memory[label] = max(memory.get(label, 0), used)
    for label in ("lucid", "unchecked"):
        if label in memory:
            memory[label] = max(memory[label] - compiler, 1)
    best = {label: min(values) for label, values in times.items()}
    median = {label: statistics.median(values) for label, values in times.items()}
    for label in commands:
        megabytes = memory[label] / 1024 / 1024
        print(
            f"{name} {label}: best {best[label]:.0f} ms, median {median[label]:.0f} ms, "
            f"peak {megabytes:.1f} MB"
        )
    for table, title in ((best, "best"), (median, "median")):
        for base in ("rust", "checked"):
            if base in table:
                print(f"{name} lucid/{base} {title}: {table['lucid'] / table[base]:.2f}")
    print(f"{name} lucid/rust memory: {memory['lucid'] / memory['rust']:.2f}")
