//! Builds `lucid_rt` twice, in release. As a static library with its C `main`,
//! which `lucid build` embeds and links into each program, with the system
//! libraries it needs. And as a WebAssembly module for
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
    let library = ["--features", "exe", "--crate-type", "staticlib"];
    let stderr = build(&out, &library, &["--print", "native-static-libs"]);
    // Cargo replays this note when the library is already built.
    let libs = stderr.lines().find_map(|line| line.split("native-static-libs: ").nth(1));
    let libs = libs.unwrap_or_else(|| panic!("no native-static-libs in:\n{stderr}"));
    println!("cargo::rustc-env=LUCID_RT_LIBS={}", libs.trim());
    let name =
        if env::var_os("CARGO_CFG_WINDOWS").is_some() { "lucid_rt.lib" } else { "liblucid_rt.a" };
    let lib = out.join("runtime/release").join(name);
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
        // Data in its own sections, so the linker drops SQLite's tables, and
        // with them its code, from a program that never opens a database.
        .env("CFLAGS_x86_64-pc-windows-msvc", windows_cflags())
        .arg("--")
        .args(rustc_args)
        .output()
        .expect("cannot start cargo");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "building lucid_rt failed:\n{stderr}");
    stderr
}

/// `/Gw` added to the flags that `cc` would read for the runtime's C, so a
/// developer's own flags stay. `cc` takes the first of these that is set, so
/// `build` sets the first.
fn windows_cflags() -> String {
    let names = [
        "CFLAGS_x86_64-pc-windows-msvc",
        "CFLAGS_x86_64_pc_windows_msvc",
        "TARGET_CFLAGS",
        "CFLAGS",
    ];
    let own = names.iter().find_map(|name| env::var(name).ok()).unwrap_or_default();
    format!("{own} /Gw").trim_start().to_string()
}
