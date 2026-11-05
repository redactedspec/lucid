#!/bin/sh
# Builds Rust release, Rust with overflow checks, Lucid, and Lucid without overflow
# checks, then times them with run.py. Run from the repository root. Rust programs
# read their size at run time, so nothing folds away.
set -e
exe=$(rustc --print cfg | grep -q 'target_os="windows"' && echo .exe || true)
mkdir -p target/bench
cargo build -q --release -p lucid --features bench-unchecked
cp "target/release/lucid$exe" "target/bench/lucid-unchecked$exe"
cargo build -q --release -p lucid

for name in fib collatz sum; do
    rustc -O --edition 2024 -o "target/bench/$name$exe" "benchmarks/$name.rs"
    rustc -O -C overflow-checks=on --edition 2024 -o "target/bench/$name-checked$exe" "benchmarks/$name.rs"
done

python benchmarks/run.py "$@"
