//! The start of a program that `lucid build` made. Only the static library
//! that the compiler embeds has this, since the compiler has its own `main`.
//! See "`lucid build`" in docs/compiler.md.

use crate::{CallSite, EXE_STACK_BYTES, SITE_WORDS, STACK_MARGIN};
use crate::{set_call_sites, set_grants, set_stack_limit};
use std::collections::HashMap;

unsafe extern "C" {
    /// The program's top level, from the object file. It takes the stack
    /// limit, as every compiled task does.
    fn lucid_main(stack_limit: usize) -> u8;
    /// A count, then that many rows of `SITE_WORDS` words, from the object file.
    static lucid_call_sites: usize;
    /// The text that rows of `lucid_call_sites` point into.
    static lucid_call_names: u8;
    /// The text for `set_grants`, ended by a zero byte.
    static lucid_grants: std::ffi::c_char;
}

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    // On Windows the linker gives the main thread `EXE_STACK_BYTES`. Linux
    // takes the main thread's stack from `ulimit`, so the program runs on a
    // thread of its own there, by decision 191.
    #[cfg(windows)]
    start();
    #[cfg(not(windows))]
    {
        unsafe extern "C" {
            fn signal(signal: i32, handler: usize) -> usize;
        }
        // The values on Linux. Rust's own `main` ignores SIGPIPE, so that a
        // write to a closed socket fails with an error, and does not end the
        // process. This `main` takes its place, so it does the same.
        const SIGPIPE: i32 = 13;
        const SIG_IGN: usize = 1;
        // SAFETY: ignoring a signal runs no handler code.
        unsafe { signal(SIGPIPE, SIG_IGN) };
        let thread = std::thread::Builder::new().stack_size(EXE_STACK_BYTES).spawn(start);
        // `start` ends the process, so a join returns only on failure.
        let _ = thread.expect("cannot start the main thread").join();
    }
    std::process::exit(101)
}

/// Runs the program on a thread of `EXE_STACK_BYTES`, then ends the process.
fn start() {
    let here = 0u8;
    // Little of this thread's stack is in use yet, as in `native::run`.
    let limit = (&raw const here) as usize - (EXE_STACK_BYTES - STACK_MARGIN);
    set_stack_limit(limit);
    // SAFETY: `native.rs` writes the table in the documented shape, and both
    // symbols live as long as the program.
    set_call_sites(unsafe { call_sites() });
    // SAFETY: `native.rs` ends the text with a zero byte, and only lines of
    // UTF-8 from `lucid.toml` come before it.
    let grants = unsafe { std::ffi::CStr::from_ptr(&raw const lucid_grants) };
    set_grants(&grants.to_string_lossy());
    // SAFETY: `lucid_main` needs only the limit of this thread.
    unsafe { lucid_main(limit) };
    // Unlike a return from a C `main`, this flushes stdout.
    std::process::exit(0)
}

/// Reads the table of `lucid_call_sites`.
///
/// # Safety
///
/// The table must have the shape of `SITE_WORDS`, with names in bounds.
unsafe fn call_sites() -> HashMap<usize, Option<CallSite>> {
    let table = &raw const lucid_call_sites;
    let names = &raw const lucid_call_names;
    // SAFETY: the caller guarantees the shape.
    unsafe {
        let rows = std::slice::from_raw_parts(table.add(1), *table * SITE_WORDS);
        let name = |start: usize, len: usize| {
            let bytes = std::slice::from_raw_parts(names.add(start), len);
            std::str::from_utf8_unchecked(bytes)
        };
        let row = |r: &[usize]| {
            let site = (r[6] != 0).then(|| CallSite {
                file: name(r[2], r[3]),
                task: name(r[4], r[5]),
                line: r[6],
                column: r[7],
            });
            (r[0] + r[1], site)
        };
        rows.chunks_exact(SITE_WORDS).map(row).collect()
    }
}
