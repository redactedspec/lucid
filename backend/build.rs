//! Builds `lucid_rt` twice, in release. As a static library with its C `main`,
//! which `lucid build` embeds and links into each program, with the system
//! libraries it needs. And as a WebAssembly module for `--target web`, on
//! every host. Then builds this crate, with the runtime and its C `main`, as
//! the static library that a program calling the backend links, by decision
//! 217. See "`lucid build`", "WebAssembly", and "Self-hosting" in
//! docs/compiler.md.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Set for the inner build of this crate, to the lines that the outer build
/// gives to `rustc-env`, so the inner build does not start a build of itself.
const INNER: &str = "LUCID_BACKEND_INNER";

fn main() {
    println!("cargo::rerun-if-env-changed={INNER}");
    if let Ok(lines) = env::var(INNER) {
        lines.lines().for_each(|line| println!("cargo::rustc-env={line}"));
        return;
    }
    println!("cargo::rerun-if-changed=../runtime");
    println!("cargo::rerun-if-changed=src");
    println!("cargo::rerun-if-changed=Cargo.toml");
    // Dependency versions and the workspace profile reach the nested builds.
    println!("cargo::rerun-if-changed=../Cargo.toml");
    println!("cargo::rerun-if-changed=../Cargo.lock");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let windows = env::var_os("CARGO_CFG_WINDOWS").is_some();
    // The table export lets the program module add its tasks for the runtime
    // to call, by decision 157.
    let wasm = ["--target", "wasm32-unknown-unknown", "--crate-type", "cdylib"];
    let runtime = out.join("runtime");
    let wasm_args = ["-C", "link-arg=--export-table", "-C", "link-arg=--growable-table"];
    build(&runtime, &wasm, &wasm_args);
    let wasm = runtime.join("wasm32-unknown-unknown/release/lucid_rt.wasm");
    let library = ["--features", "exe", "--crate-type", "staticlib"];
    let print = ["--print", "native-static-libs"];
    let libs = static_libs(&build(&runtime, &library, &print));
    let name = if windows { "lucid_rt.lib" } else { "liblucid_rt.a" };
    let lib = runtime.join("release").join(name);
    let backend = out.join("backend");
    let name = if windows { "lucid_backend.lib" } else { "liblucid_backend.a" };
    let backend_lib = backend.join("release").join(name);
    // The inner build cannot know its own system libraries, so `link` reads
    // them from this file, one argument per line, with the directories that
    // build scripts named, which hold libraries such as `windows-targets`.
    let backend_libs = backend.join("libs.txt");
    let lines = [
        format!("LUCID_RT_WASM={}", wasm.display()),
        format!("LUCID_RT_LIBS={libs}"),
        format!("LUCID_RT_LIB={}", lib.display()),
        format!("LUCID_BACKEND_LIB={}", backend_lib.display()),
        format!("LUCID_BACKEND_LIBS={}", backend_libs.display()),
    ]
    .join("\n");
    let json = ["--message-format=json-render-diagnostics"];
    let args = [&library[..], &json].concat();
    let (stderr, stdout) = build_with(".", &backend, &args, &print, &lines);
    let mut found: Vec<String> =
        static_libs(&stderr).split_whitespace().map(String::from).collect();
    for path in linked_paths(&stdout) {
        found.push(if windows { format!("/LIBPATH:{path}") } else { format!("-L{path}") });
    }
    std::fs::write(&backend_libs, found.join("\n")).expect("cannot write the backend's libraries");
    lines.lines().for_each(|line| println!("cargo::rustc-env={line}"));
}

/// The system libraries that `--print native-static-libs` names.
fn static_libs(stderr: &str) -> String {
    // Cargo replays this note when the library is already built.
    let libs = stderr.lines().find_map(|line| line.split("native-static-libs: ").nth(1));
    libs.unwrap_or_else(|| panic!("no native-static-libs in:\n{stderr}")).trim().to_string()
}

/// The `linked_paths` of every build script in cargo's JSON messages, with no
/// kind such as `native=`.
fn linked_paths(stdout: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for message in stdout.split("\"linked_paths\":[").skip(1) {
        let list = message.split(']').next().unwrap_or_default();
        for item in list.split("\",\"") {
            let item = item.trim_matches('"').replace("\\\\", "\\");
            let item = item.split_once('=').map_or(item.as_str(), |(_, path)| path);
            if !item.is_empty() && !paths.iter().any(|known| known == item) {
                paths.push(item.to_string());
            }
        }
    }
    paths
}

/// Builds the runtime's library in release with `cargo rustc`, and gives its
/// stderr.
fn build(target: &Path, cargo_args: &[&str], rustc_args: &[&str]) -> String {
    build_with("../runtime", target, cargo_args, rustc_args, "").0
}

/// Builds a crate's library in release with `cargo rustc`, and gives its
/// stderr and stdout. `inner` is for `INNER`, empty for the runtime.
fn build_with(
    dir: &str,
    target: &Path,
    cargo_args: &[&str],
    rustc_args: &[&str],
    inner: &str,
) -> (String, String) {
    let cargo = env::var_os("CARGO").expect("cargo sets CARGO");
    // A target directory of its own, so this waits on no lock that the outer
    // build holds.
    let mut command = Command::new(cargo);
    command
        .args(["rustc", "--manifest-path", &format!("{dir}/Cargo.toml"), "--release", "--lib"])
        .args(cargo_args)
        .arg("--target-dir")
        .arg(target)
        // Data in its own sections, so the linker drops SQLite's tables, and
        // with them its code, from a program that never opens a database.
        .env("CFLAGS_x86_64-pc-windows-msvc", windows_cflags())
        .arg("--")
        .args(rustc_args);
    if !inner.is_empty() {
        command.env(INNER, inner);
    }
    let output = command.output().expect("cannot start cargo");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "building {dir} failed:\n{stderr}");
    (stderr, String::from_utf8_lossy(&output.stdout).into_owned())
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
