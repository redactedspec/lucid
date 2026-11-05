#!/bin/sh
# Prints the best of 5 wall-clock runs of each program, in milliseconds. Run from
# the repository root. Compares Rust release, Lucid, and Lucid without overflow
# checks. The Rust programs read their size at run time, so nothing folds away.
set -e
exe=$(rustc --print cfg | grep -q 'target_os="windows"' && echo .exe || true)
mkdir -p target/bench
cargo build -q --release -p lucid --features bench-unchecked
cp "target/release/lucid$exe" "target/bench/lucid-unchecked$exe"
cargo build -q --release -p lucid

best() {
    python -c '
import os, subprocess, sys, time
command = [os.path.abspath(sys.argv[1])] + sys.argv[2:]
times = []
for _ in range(5):
    start = time.perf_counter()
    subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
    times.append(time.perf_counter() - start)
print(round(min(times) * 1000))
' "$@"
}

# `set -e` ignores a failure inside `echo $(...)`, so results go through a name.
report() {
    label=$1
    shift
    time=$(best "$@")
    echo "$label $time"
}

for name in fib collatz; do
    rustc -O --edition 2024 -o "target/bench/$name$exe" "benchmarks/$name.rs"
    report "$name rust" "target/bench/$name$exe"
    report "$name lucid" "target/release/lucid$exe" run "benchmarks/$name.lcd"
    report "$name unchecked" "target/bench/lucid-unchecked$exe" run "benchmarks/$name.lcd"
done

# Rust folds this loop away, so only the cost of the checks is meaningful.
report "sum lucid" "target/release/lucid$exe" run benchmarks/sum.lcd
report "sum unchecked" "target/bench/lucid-unchecked$exe" run benchmarks/sum.lcd
