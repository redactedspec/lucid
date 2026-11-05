//! Lucid runtime. Compiled code calls these tasks through the C ABI.

use std::io::Write;

/// Prints UTF-8 text and a newline. If stdout fails, the program traps.
///
/// # Safety
///
/// `text` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_print(text: *const u8, len: usize) {
    // SAFETY: the caller guarantees the range, and compiled code passes static data.
    let bytes = unsafe { std::slice::from_raw_parts(text, len) };
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(bytes).and_then(|()| stdout.write_all(b"\n")).is_err() {
        trap("cannot write to stdout");
    }
}

/// Ends the program with the exit code that language.md gives to traps.
fn trap(message: &str) -> ! {
    // Must not panic, since a caller is an `extern "C"` task. The exit code still reports the trap.
    let _ = writeln!(std::io::stderr(), "trap: {message}");
    std::process::exit(101)
}
