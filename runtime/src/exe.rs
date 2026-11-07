//! The start of a program that `lucid build` made. Only the static library
//! that the compiler embeds has this, since the compiler has its own `main`.
//! See "`lucid build`" in docs/compiler.md.

use crate::{CallSite, EXE_STACK_BYTES, SITE_WORDS, STACK_MARGIN, set_call_sites, set_grants};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The lowest stack address that compiled code may use. Each task reads it on
/// entry. Set once, before the program starts.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)] // The C name of the symbol.
pub static lucid_stack_limit: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    /// The program's top level, from the object file.
    fn lucid_main() -> u8;
    /// A count, then that many rows of `SITE_WORDS` words, from the object file.
    static lucid_call_sites: usize;
    /// The text that rows of `lucid_call_sites` point into.
    static lucid_call_names: u8;
    /// The text for `set_grants`, ended by a zero byte.
    static lucid_grants: std::ffi::c_char;
}

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let here = 0u8;
    // The linker gives the main thread `EXE_STACK_BYTES`, and little of it is
    // in use yet, as in `native::run`.
    let limit = (&raw const here) as usize - (EXE_STACK_BYTES - STACK_MARGIN);
    lucid_stack_limit.store(limit, Ordering::Relaxed);
    // SAFETY: `native.rs` writes the table in the documented shape, and both
    // symbols live as long as the program.
    set_call_sites(unsafe { call_sites() });
    // SAFETY: `native.rs` ends the text with a zero byte, and only lines of
    // UTF-8 from `lucid.toml` come before it.
    let grants = unsafe { std::ffi::CStr::from_ptr(&raw const lucid_grants) };
    set_grants(&grants.to_string_lossy());
    // SAFETY: `lucid_main` takes nothing, and needs only the limit above.
    unsafe { lucid_main() };
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
