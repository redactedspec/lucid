//! The backends of Lucid, and what they need: MIR and its text form, native
//! code through Cranelift, WebAssembly, and linking. Both front ends feed it,
//! the Rust one by calls and the Lucid one by the `owned` dump, by decisions
//! 216 and 217. See "Backend" under "Self-hosting" in docs/compiler.md.

pub mod diag;
pub mod dump;
pub mod mir;
pub mod native;
pub mod wasm;

/// Links an object file from `native::object` with the embedded runtime, into
/// `exe`. A failure here is in the system, not the program, so it has no code,
/// and the text tells what failed.
pub fn link(object: &[u8], exe: &str) -> Result<(), String> {
    const RUNTIME: &[u8] = include_bytes!(env!("LUCID_RT_LIB"));
    // On Linux, `lucid build app` from the parent of `app/` names the
    // program after the directory it would overwrite.
    if std::path::Path::new(exe).is_dir() {
        return Err(format!("cannot write the program to `{exe}`: a directory has that name"));
    }
    let mut command = match linker() {
        Ok(command) => command,
        Err(problem) => return Err(problem.into()),
    };
    let dir = match stage() {
        Ok(dir) => dir,
        Err(e) => return Err(format!("cannot make a staging directory: {e}")),
    };
    let (object_file, runtime_file) = if cfg!(windows) {
        (dir.join("main.obj"), dir.join("lucid_rt.lib"))
    } else {
        (dir.join("main.o"), dir.join("liblucid_rt.a"))
    };
    let linked = std::fs::write(&object_file, object)
        .and_then(|()| std::fs::write(&runtime_file, RUNTIME))
        .and_then(|()| {
            if cfg!(windows) {
                command.args(["/NOLOGO", "/SUBSYSTEM:CONSOLE"]);
                command.arg(format!("/STACK:{}", lucid_rt::EXE_STACK_BYTES));
                command.arg(format!("/OUT:{exe}"));
            } else {
                // Drops the sections that the program never reaches, such as
                // SQLite in a program that never opens a database.
                command.arg("-Wl,--gc-sections").arg("-o").arg(exe);
            }
            command.arg(&object_file).arg(&runtime_file);
            command.args(env!("LUCID_RT_LIBS").split_whitespace()).output()
        });
    let _ = std::fs::remove_dir_all(&dir);
    match linked {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let said =
                String::from_utf8_lossy(&output.stdout) + String::from_utf8_lossy(&output.stderr);
            Err(format!("linking failed:\n{said}"))
        }
        Err(e) => Err(format!("cannot link: {e}")),
    }
}

#[cfg(windows)]
fn linker() -> Result<std::process::Command, &'static str> {
    find_msvc_tools::find_tool("x86_64-pc-windows-msvc", "link.exe")
        .map(|tool| tool.to_command())
        .ok_or("cannot find the MSVC linker: install Visual Studio Build Tools, with the C++ tools")
}

/// The system `cc` finds the C start files and libc, by decision 191.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linker() -> Result<std::process::Command, &'static str> {
    Ok(std::process::Command::new("cc"))
}

/// Only Windows and Linux on x86-64 are targets, by decision 205.
#[cfg(not(any(windows, all(target_os = "linux", target_arch = "x86_64"))))]
fn linker() -> Result<std::process::Command, &'static str> {
    Err("`lucid build` works only on Windows and Linux x86-64")
}

/// Makes a new directory with a random name for `link`, that only this user
/// can open, by decision 191. It must not exist yet, so no one else can have
/// planted files in it.
pub fn stage() -> std::io::Result<std::path::PathBuf> {
    use std::hash::{BuildHasher, RandomState};
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        let mut builder = builder;
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    let mut tries = 0;
    loop {
        // Each `RandomState` has new keys, seeded from the system.
        let name = format!("lucid-build-{:016x}", RandomState::new().hash_one(std::process::id()));
        let dir = std::env::temp_dir().join(name);
        match builder.create(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && tries < 8 => tries += 1,
            made => return made.map(|()| dir),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// By decision 191, each build stages in a new directory of its own.
    #[test]
    fn staging_is_new_and_private() {
        let (one, two) = (stage().unwrap(), stage().unwrap());
        assert_ne!(one, two);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&one).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        std::fs::remove_dir(one).unwrap();
        std::fs::remove_dir(two).unwrap();
    }
}
