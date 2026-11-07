//! Builds `lucid_rt` as a release static library with its C `main`, which
//! `lucid build` embeds and links into each program. Also passes on the system
//! libraries it needs. See "`lucid build`" in docs/compiler.md.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=../runtime");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let cargo = env::var_os("CARGO").expect("cargo sets CARGO");
    // A target directory of its own, so this waits on no lock that the outer
    // build holds.
    let output = Command::new(cargo)
        .args(["rustc", "--manifest-path", "../runtime/Cargo.toml", "--release", "--lib"])
        .args(["--features", "exe", "--crate-type", "staticlib", "--target-dir"])
        .arg(out.join("runtime"))
        .args(["--", "--print", "native-static-libs"])
        .output()
        .expect("cannot start cargo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "building lucid_rt failed:\n{stderr}");
    // Cargo replays this note when the library is already built.
    let libs = stderr.lines().find_map(|line| line.split("native-static-libs: ").nth(1));
    let libs = libs.unwrap_or_else(|| panic!("no native-static-libs in:\n{stderr}"));
    println!("cargo::rustc-env=LUCID_RT_LIBS={}", libs.trim());
    let lib = out.join("runtime/release/lucid_rt.lib");
    println!("cargo::rustc-env=LUCID_RT_LIB={}", lib.display());
}
