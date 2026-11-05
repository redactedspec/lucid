//! Lucid runtime. Compiled code calls these tasks through the C ABI.
//!
//! Debug builds, which run the golden tests, keep count of live values and
//! never reuse their memory. So a leak or double free stops the program.

use std::alloc::{Layout, alloc, dealloc};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

const STDOUT_FAILED: &[u8] = b"trap: cannot write to stdout";
const OUT_OF_MEMORY: &[u8] = b"trap: out of memory";

/// A counted, immutable UTF-8 text. Its bytes follow this header.
#[repr(C)]
pub struct Text {
    count: usize,
    len: usize,
}

const HEADER: usize = size_of::<Text>();

/// Debug builds only: the count of a released value.
const DEAD: usize = usize::MAX;

/// Debug builds only: the number of values not yet released.
static LIVE: AtomicUsize = AtomicUsize::new(0);

fn layout(len: usize) -> Layout {
    let size = len.saturating_add(HEADER);
    Layout::from_size_align(size, align_of::<Text>()).unwrap_or_else(|_| trap(OUT_OF_MEMORY))
}

/// Allocates a text with a count of one. The caller fills its `len` bytes.
fn text_alloc(len: usize) -> *mut Text {
    // SAFETY: the layout is never zero sized, because of the header.
    let text = unsafe { alloc(layout(len)) }.cast::<Text>();
    if text.is_null() {
        trap(OUT_OF_MEMORY);
    }
    if cfg!(debug_assertions) {
        LIVE.fetch_add(1, Ordering::Relaxed);
    }
    // SAFETY: the allocation holds a header, and is aligned for it.
    unsafe { text.write(Text { count: 1, len }) };
    text
}

/// # Safety
///
/// `text` must point to a live text. The slice must not outlive it.
unsafe fn bytes<'a>(text: *const Text) -> &'a [u8] {
    // SAFETY: the caller guarantees a live text, whose bytes follow the header.
    unsafe {
        if cfg!(debug_assertions) && (*text).count == DEAD {
            trap(b"trap: runtime bug: use after free");
        }
        std::slice::from_raw_parts(text.add(1).cast::<u8>(), (*text).len)
    }
}

fn text_from(bytes: &[u8]) -> *mut Text {
    let text = text_alloc(bytes.len());
    // SAFETY: `text_alloc` made room for exactly these bytes after the header.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), text.add(1).cast(), bytes.len()) };
    text
}

/// Makes a text from UTF-8 bytes.
///
/// # Safety
///
/// `bytes` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_new(bytes: *const u8, len: usize) -> *mut Text {
    // SAFETY: the caller guarantees the range.
    text_from(unsafe { std::slice::from_raw_parts(bytes, len) })
}

/// Makes a text from `count` texts, in order.
///
/// # Safety
///
/// `parts` must point to `count` pointers to live texts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_join(parts: *const *const Text, count: usize) -> *mut Text {
    // SAFETY: the caller guarantees the range.
    let parts = unsafe { std::slice::from_raw_parts(parts, count) };
    // SAFETY: the caller guarantees live texts.
    let part = |text: &*const Text| unsafe { bytes(*text) };
    // Saturates, so a length too large to allocate traps. One text can repeat.
    let len = parts.iter().fold(0, |len: usize, text| len.saturating_add(part(text).len()));
    let joined = text_alloc(len);
    // SAFETY: `text_alloc` made room for `len` bytes after the header.
    let mut next = unsafe { joined.add(1) }.cast::<u8>();
    for text in parts {
        let part = part(text);
        // SAFETY: the parts add up to `len`, so each one fits.
        unsafe {
            std::ptr::copy_nonoverlapping(part.as_ptr(), next, part.len());
            next = next.add(part.len());
        }
    }
    joined
}

/// # Safety
///
/// `value` must point to a live value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_retain(value: *mut Text) {
    // SAFETY: the caller guarantees a live value.
    unsafe {
        if cfg!(debug_assertions) && (*value).count == DEAD {
            trap(b"trap: runtime bug: use after free");
        }
        (*value).count += 1;
    }
}

/// Gives up one count of `value`, which may be null: a local starts that way.
///
/// # Safety
///
/// `value` must be null, or point to a live value that this caller has a count of.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_release(value: *mut Text) {
    if value.is_null() {
        return;
    }
    // SAFETY: the caller guarantees a live value, and the last count frees it.
    unsafe {
        if cfg!(debug_assertions) && (*value).count == DEAD {
            trap(b"trap: runtime bug: double free");
        }
        (*value).count -= 1;
        if (*value).count > 0 {
            return;
        }
        if cfg!(debug_assertions) {
            LIVE.fetch_sub(1, Ordering::Relaxed);
            (*value).count = DEAD;
        } else {
            dealloc(value.cast(), layout((*value).len));
        }
    }
}

/// Debug builds only: reports values that the program never released.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_check_leaks() {
    if cfg!(debug_assertions) {
        let live = LIVE.load(Ordering::Relaxed);
        if live > 0 {
            trap(format!("trap: runtime bug: {live} values leaked").as_bytes());
        }
    }
}

/// Prints a text and a newline.
///
/// # Safety
///
/// `text` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_print(text: *const Text) {
    // SAFETY: the caller guarantees a live text.
    let bytes = unsafe { bytes(text) };
    // SAFETY: the range comes from a live slice.
    unsafe { lucid_print(bytes.as_ptr(), bytes.len()) };
}

/// Orders by Unicode code point, which is the byte order of UTF-8.
/// Returns a value below, at, or above zero.
///
/// # Safety
///
/// `a` and `b` must point to live texts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_compare(a: *const Text, b: *const Text) -> i64 {
    // SAFETY: the caller guarantees live texts.
    unsafe { bytes(a).cmp(bytes(b)) as i64 }
}

/// True if this byte starts a Unicode character, and does not continue one.
fn starts_character(byte: u8) -> bool {
    byte & 0xC0 != 0x80
}

/// Counts Unicode characters.
///
/// # Safety
///
/// `text` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_len(text: *const Text) -> i64 {
    // SAFETY: the caller guarantees a live text.
    let bytes = unsafe { bytes(text) };
    bytes.iter().filter(|&&byte| starts_character(byte)).count() as i64
}

/// # Safety
///
/// `text` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_is_empty(text: *const Text) -> u8 {
    // SAFETY: the caller guarantees a live text.
    u8::from(unsafe { bytes(text) }.is_empty())
}

/// Copies characters `start` up to but not including `end`. Returns null if
/// the range is not `0 <= start <= end <= len`, and compiled code then traps.
///
/// # Safety
///
/// `text` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_slice(text: *const Text, start: i64, end: i64) -> *mut Text {
    // SAFETY: the caller guarantees a live text.
    let bytes = unsafe { bytes(text) };
    if start < 0 || end < start {
        return std::ptr::null_mut();
    }
    // ponytail: a scan and a copy per slice, so a loop of slices is O(n^2).
    // A view or an ASCII flag can fix this if M4 benchmarks need it.
    let offset = |character: i64| {
        let starts = (0..bytes.len()).filter(|&at| starts_character(bytes[at]));
        starts.chain([bytes.len()]).nth(character as usize)
    };
    match (offset(start), offset(end)) {
        (Some(first), Some(last)) => text_from(&bytes[first..last]),
        _ => std::ptr::null_mut(),
    }
}

/// # Safety
///
/// `text` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_byte_len(text: *const Text) -> i64 {
    // SAFETY: the caller guarantees a live text.
    unsafe { bytes(text) }.len() as i64
}

/// Copies the character that starts at byte `offset`.
///
/// # Safety
///
/// `text` must point to a live text, and `offset` must start a character.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_char_at(text: *const Text, offset: i64) -> *mut Text {
    // SAFETY: the caller guarantees a live text.
    let rest = &unsafe { bytes(text) }[offset as usize..];
    let width = 1 + rest[1..].iter().take_while(|&&byte| !starts_character(byte)).count();
    text_from(&rest[..width])
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_int_text(value: i64) -> *mut Text {
    text_from(value.to_string().as_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_float_text(value: f64) -> *mut Text {
    text_from(float_text(value).as_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_bool_text(value: u8) -> *mut Text {
    text_from(if value != 0 { b"true" } else { b"false" })
}

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
fn float_text(value: f64) -> String {
    let text = format!("{value:?}");
    match text.split_once('e') {
        Some((digits, exponent)) if !digits.contains('.') => format!("{digits}.0e{exponent}"),
        _ => text,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_print_float(value: f64) {
    print_line(float_text(value));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(debug_assertions)]
    #[test]
    fn counts_reach_the_leak_check() {
        let start = LIVE.load(Ordering::Relaxed);
        let text = text_from("héllo".as_bytes());
        // SAFETY: `text` stays live until the last release.
        unsafe {
            assert_eq!(lucid_text_len(text), 5);
            lucid_retain(text);
            lucid_release(text);
            assert_eq!(LIVE.load(Ordering::Relaxed), start + 1);
            lucid_release(text);
            lucid_release(std::ptr::null_mut());
        }
        assert_eq!(LIVE.load(Ordering::Relaxed), start);
    }

    #[test]
    fn slices_count_characters_and_reject_bad_ranges() {
        let text = text_from("a€😀".as_bytes());
        // SAFETY: `text` stays live, and each slice is read before its release.
        unsafe {
            for (start, end, expected) in [(0, 3, "a€😀"), (1, 2, "€"), (2, 3, "😀"), (3, 3, "")]
            {
                let slice = lucid_text_slice(text, start, end);
                assert_eq!(bytes(slice), expected.as_bytes());
                lucid_release(slice);
            }
            for (start, end) in [(-1, 1), (2, 1), (0, 4), (4, 4), (0, i64::MAX), (i64::MIN, 0)] {
                assert!(lucid_text_slice(text, start, end).is_null());
            }
            lucid_release(text);
        }
    }

    #[test]
    fn characters_are_read_by_byte_offset() {
        let text = text_from("a€😀".as_bytes());
        // SAFETY: `text` stays live, and each offset starts a character.
        unsafe {
            assert_eq!(lucid_text_byte_len(text), 8);
            for (offset, expected) in [(0, "a"), (1, "€"), (4, "😀")] {
                let character = lucid_text_char_at(text, offset);
                assert_eq!(bytes(character), expected.as_bytes());
                lucid_release(character);
            }
            lucid_release(text);
        }
    }
}
