//! The backends of Lucid, and what they need: MIR and its text form, native
//! code through Cranelift, WebAssembly, and linking. Both front ends feed it,
//! the Rust one by calls and the Lucid one by the `owned` dump, by decisions
//! 216 and 217. See "Backend" under "Self-hosting" in docs/compiler.md.

pub mod diag;
pub mod dump;
pub mod mir;
pub mod native;
pub mod wasm;
/// The stack of the thread that runs a program, and of the Rust compiler's
/// own. Debug builds use large stack frames, and `MAX_DEPTH` levels of nesting
/// must fit.
pub const STACK_BYTES: usize = 256 * 1024 * 1024;

/// The C ABI that the Lucid front end calls, by decision 217. A name that
/// starts with `lucid_` is kept for the runtime, which only `std` may
/// declare, so these start with `backend_`. `native::find_symbol` finds them,
/// so the Lucid compiler runs under `lucid run`, and a built program that
/// names one links `LUCID_BACKEND_LIB`.
pub const EXPORTS: [&str; 3] = ["backend_run", "backend_build", "backend_has_symbol"];

/// The address of a task of `EXPORTS`.
pub fn export(name: &str) -> Option<*const u8> {
    match name {
        "backend_run" => Some(backend_run as *const u8),
        "backend_build" => Some(backend_build as *const u8),
        "backend_has_symbol" => Some(backend_has_symbol as *const u8),
        _ => None,
    }
}

/// The bytes of a `list[byte]` that Lucid passes as `.pointer()` and length.
///
/// # Safety
///
/// `at` must point to `len` bytes that stay unchanged for the call.
unsafe fn given<'a>(at: *const u8, len: i64) -> &'a [u8] {
    if at.is_null() || len <= 0 {
        return &[];
    }
    // SAFETY: the caller guarantees the range.
    unsafe { std::slice::from_raw_parts(at, len as usize) }
}

/// 1 if `native::find_symbol` finds this name, for E0904, or else 0.
///
/// # Safety
///
/// As `given`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn backend_has_symbol(name: *const u8, len: i64) -> i64 {
    // SAFETY: as this task's own contract.
    let name = unsafe { given(name, len) };
    std::str::from_utf8(name).ok().and_then(native::find_symbol).is_some() as i64
}

/// Runs a program: `JSON ARGS "ARG"...`, then the `owned` dump, where `JSON`
/// is `true` or `false` and `ARGS` counts the arguments that the program
/// reads. Gives 0, or 1 after it reports errors. A trap or `exit` in the
/// program ends the process, as under `lucid run`.
///
/// # Safety
///
/// As `given`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn backend_run(input: *const u8, len: i64) -> i64 {
    // SAFETY: as this task's own contract.
    let input = unsafe { given(input, len) };
    let header = |atoms: &mut dump::Atoms| {
        let json = atoms.flag()?;
        let count: usize = atoms.number()?;
        let args = (0..count).map(|_| atoms.text()).collect::<Result<Vec<_>, _>>()?;
        Ok((json, args))
    };
    let Some(((json, args), program, source)) = order(input, header) else { return 1 };
    lucid_rt::set_args(args);
    // A thread of its own, so the program has the whole stack that
    // `native::run` expects.
    let thread = std::thread::Builder::new().stack_size(STACK_BYTES);
    let run = || native::run(&program, &source, STACK_BYTES);
    let ran = std::thread::scope(|scope| thread.spawn_scoped(scope, run).map(|t| t.join()));
    match ran {
        Ok(Ok(Ok(()))) => 0,
        Ok(Ok(Err(diags))) => report(&source, &diags, json),
        Ok(Err(_)) => 101,
        Err(e) => fail(&format!("cannot start a thread: {e}")),
    }
}

/// Builds a program: `JSON WEB "OUT"`, then the `owned` dump, where `WEB` is
/// `true` for `--target web` and `OUT` names the file to write. Gives 0, or
/// 1 after it reports errors.
///
/// # Safety
///
/// As `given`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn backend_build(input: *const u8, len: i64) -> i64 {
    // SAFETY: as this task's own contract.
    let input = unsafe { given(input, len) };
    let header = |atoms: &mut dump::Atoms| Ok((atoms.flag()?, atoms.flag()?, atoms.text()?));
    let Some(((json, web, out), program, source)) = order(input, header) else { return 1 };
    let built = if web {
        wasm::script(&program, &source).map(|script| write_script(&script, &out))
    } else {
        native::object(&program, &source).map(|(object, backend)| link(&object, &out, backend))
    };
    match built {
        Ok(Ok(())) => 0,
        Ok(Err(problem)) => fail(&problem),
        Err(diags) => report(&source, &diags, json),
    }
}

/// Reads the input of `backend_run` or `backend_build`. Input that the Lucid
/// front end cannot write is a bug in it, which this reports.
fn order<H>(
    input: &[u8],
    header: impl FnOnce(&mut dump::Atoms) -> Result<H, String>,
) -> Option<(H, mir::Program, diag::Source)> {
    let read = std::str::from_utf8(input).map_err(|e| e.to_string());
    match read.and_then(|text| dump::read_after(text, header)) {
        Ok(order) => Some(order),
        Err(problem) => {
            fail(&format!("the backend cannot read the program: {problem}"));
            None
        }
    }
}

/// Reports compile errors, as data on stdout with `json`, and gives 1.
pub fn report(source: &diag::Source, diags: &[diag::Diagnostic], json: bool) -> i64 {
    use std::io::Write;
    // If the reader is gone there is no one to tell, and the result still
    // says what happened.
    _ = if json {
        std::io::stdout().write_all(diag::render_json(source, diags).as_bytes())
    } else {
        std::io::stderr().write_all(diag::render_text(source, diags).as_bytes())
    };
    if diags.is_empty() { 0 } else { 1 }
}

/// Reports a failure in the system, not the program, which has no code.
fn fail(problem: &str) -> i64 {
    use std::io::Write;
    _ = std::io::stderr().write_all(format!("lucid: {problem}\n").as_bytes());
    1
}

/// Writes the output of `lucid build --target web`. As in `link`, a failure
/// is in the system, so it has no code.
pub fn write_script(script: &str, name: &str) -> Result<(), String> {
    std::fs::write(name, script).map_err(|e| format!("cannot write `{name}`: {e}"))
}

/// Links an object file from `native::object` with the embedded runtime, into
/// `exe`, or with `LUCID_BACKEND_LIB` if it calls a task of `EXPORTS`. A
/// failure here is in the system, not the program, so it has no code, and the
/// text tells what failed.
pub fn link(object: &[u8], exe: &str, backend: bool) -> Result<(), String> {
    const RUNTIME: &[u8] = include_bytes!(env!("LUCID_RT_LIB"));
    // ponytail: read from where the build left it, not held inside, since a
    // library cannot hold itself. A moved `lucid` cannot build the compiler.
    let read = |path: &str| std::fs::read(path).map_err(|e| format!("cannot read `{path}`: {e}"));
    // One argument per line, since a directory may hold a space.
    let (library, libs) = if backend {
        let libs = String::from_utf8_lossy(&read(env!("LUCID_BACKEND_LIBS"))?).into_owned();
        (std::borrow::Cow::Owned(read(env!("LUCID_BACKEND_LIB"))?), libs)
    } else {
        (std::borrow::Cow::Borrowed(RUNTIME), env!("LUCID_RT_LIBS").replace(' ', "\n"))
    };
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
        .and_then(|()| std::fs::write(&runtime_file, &library))
        .and_then(|()| {
            if cfg!(windows) {
                // `/Brepro` writes a hash where the PE timestamp would go,
                // so two builds match, by decision 218.
                command.args(["/NOLOGO", "/SUBSYSTEM:CONSOLE", "/Brepro"]);
                command.arg(format!("/STACK:{}", lucid_rt::EXE_STACK_BYTES));
                command.arg(format!("/OUT:{exe}"));
            } else {
                // Drops the sections that the program never reaches, such as
                // SQLite in a program that never opens a database. Debug
                // sections go too: a trace reads the call-site tables, not
                // DWARF, by decision 228.
                command.args(["-Wl,--gc-sections", "-Wl,--strip-debug", "-o"]).arg(exe);
            }
            command.arg(&object_file).arg(&runtime_file);
            command.args(libs.lines()).output()
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
