//! Lucid runtime. Compiled code calls these tasks through the C ABI.

use std::io::Write;

const STDOUT_FAILED: &[u8] = b"trap: cannot write to stdout";

/// Writes one line to stdout. If stdout fails, the program traps.
fn print_line(value: impl std::fmt::Display) {
    if writeln!(std::io::stdout().lock(), "{value}").is_err() {
        trap(STDOUT_FAILED);
    }
}

/// Prints UTF-8 text and a newline.
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
        trap(STDOUT_FAILED);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_print_int(value: i64) {
    print_line(value);
}

/// Debug format uses the fewest digits that read back as the same float. It
/// writes `1e21`, but a Lucid float literal needs digits around a point.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_print_float(value: f64) {
    let text = format!("{value:?}");
    match text.split_once('e') {
        Some((digits, exponent)) if !digits.contains('.') => {
            print_line(format_args!("{digits}.0e{exponent}"));
        }
        _ => print_line(text),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_print_bool(value: u8) {
    print_line(value != 0);
}

/// Reports a trap. The compiler builds the message, with its source position.
///
/// # Safety
///
/// `message` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_trap(message: *const u8, len: usize) -> ! {
    // SAFETY: the caller guarantees the range, and compiled code passes static data.
    trap(unsafe { std::slice::from_raw_parts(message, len) })
}

/// Ends the program with the exit code that language.md gives to traps.
fn trap(message: &[u8]) -> ! {
    // Must not panic, since a caller is an `extern "C"` task. The exit code still reports the trap.
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(message).and_then(|()| stderr.write_all(b"\n"));
    std::process::exit(101)
}
