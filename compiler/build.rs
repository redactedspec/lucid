//! Builds `lucid_rt` twice, in release. As a static library with its C `main`,
//! which `lucid build` embeds and links into each program, with the system
//! libraries it needs, on Windows only. And as a WebAssembly module for
//! `--target web`, on every host. See "`lucid build`" and "WebAssembly" in
//! docs/compiler.md.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=../runtime");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    // The table export lets the program module add its tasks for the runtime
    // to call, by decision 157.
    let wasm = ["--target", "wasm32-unknown-unknown", "--crate-type", "cdylib"];
    build(&out, &wasm, &["-C", "link-arg=--export-table", "-C", "link-arg=--growable-table"]);
    let wasm = out.join("runtime/wasm32-unknown-unknown/release/lucid_rt.wasm");
    println!("cargo::rustc-env=LUCID_RT_WASM={}", wasm.display());
    // Only the Windows `link` in main.rs uses the library.
    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    let library = ["--features", "exe", "--crate-type", "staticlib"];
    let stderr = build(&out, &library, &["--print", "native-static-libs"]);
    // Cargo replays this note when the library is already built.
    let libs = stderr.lines().find_map(|line| line.split("native-static-libs: ").nth(1));
    let libs = libs.unwrap_or_else(|| panic!("no native-static-libs in:\n{stderr}"));
    println!("cargo::rustc-env=LUCID_RT_LIBS={}", libs.trim());
    let lib = out.join("runtime/release/lucid_rt.lib");
    println!("cargo::rustc-env=LUCID_RT_LIB={}", lib.display());
}

/// Builds the runtime in release with `cargo rustc`, and gives its stderr.
fn build(out: &Path, cargo_args: &[&str], rustc_args: &[&str]) -> String {
    let cargo = env::var_os("CARGO").expect("cargo sets CARGO");
    // A target directory of its own, so this waits on no lock that the outer
    // build holds.
    let output = Command::new(cargo)
        .args(["rustc", "--manifest-path", "../runtime/Cargo.toml", "--release", "--lib"])
        .args(cargo_args)
        .arg("--target-dir")
        .arg(out.join("runtime"))
        .arg("--")
        .args(rustc_args)
        .output()
        .expect("cannot start cargo");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "building lucid_rt failed:\n{stderr}");
    stderr
}
