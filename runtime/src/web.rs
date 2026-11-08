//! What the runtime does differently on `wasm32`, by decision 159. Rust's
//! standard library imports nothing on `wasm32-unknown-unknown`, so it cannot
//! print, exit, read a clock, or find random bytes there. The loader gives
//! these as the imports of module `js`. See "WebAssembly" in docs/compiler.md.

use super::{Head, Record, Text, bytes, lucid_release, lucid_retain, text_alloc};
use std::cell::Cell;
use std::hash::BuildHasher;
#[allow(deprecated)] // Stable, and the only standard hasher that takes keys.
use std::hash::SipHasher;
use std::io::Write;
use std::sync::OnceLock;

mod js {
    #[link(wasm_import_module = "js")]
    unsafe extern "C" {
        /// Writes bytes to stdout if `stream` is 1, or to stderr if 2.
        pub fn write(stream: u32, bytes: *const u8, len: usize);
        /// Reports a trap with its message, then the trace of the stack.
        pub fn trap(message: *const u8, len: usize) -> !;
        /// Ends the program with an exit code.
        pub safe fn exit(code: i32) -> !;
        /// Milliseconds from 1970, from `Date.now`.
        pub safe fn now() -> f64;
        /// Milliseconds on a clock that only goes forward, from `performance.now`.
        pub safe fn clock() -> f64;
        /// Fills bytes from `crypto.getRandomValues`.
        pub fn random(bytes: *mut u8, len: usize);
    }
}

pub use js::exit;

/// stdout or stderr. A write cannot fail: a browser's console takes anything,
/// and Node reports a closed pipe only later.
pub struct Stream(u32);

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        // SAFETY: the loader only reads the range.
        unsafe { js::write(self.0, bytes.as_ptr(), bytes.len()) };
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn stdout() -> Stream {
    Stream(1)
}

pub fn stderr() -> Stream {
    Stream(2)
}

pub fn trap(message: &[u8]) -> ! {
    // SAFETY: the loader only reads the range.
    unsafe { js::trap(message.as_ptr(), message.len()) }
}

/// The keys of a process, as `RandomState` holds them on other targets.
pub struct Keys(u64, u64);

impl Keys {
    pub fn new() -> Keys {
        let mut bytes = [0; 16];
        // SAFETY: the loader writes only the range.
        unsafe { js::random(bytes.as_mut_ptr(), bytes.len()) };
        let (first, second) = bytes.split_at(8);
        let word = |half: &[u8]| u64::from_ne_bytes(half.try_into().expect("8 bytes"));
        Keys(word(first), word(second))
    }
}

impl BuildHasher for Keys {
    #[allow(deprecated)]
    type Hasher = SipHasher;

    #[allow(deprecated)]
    fn build_hasher(&self) -> SipHasher {
        SipHasher::new_with_keys(self.0, self.1)
    }
}

const NANOS_PER_MILLI: f64 = 1e6;

/// As on other targets. A float of milliseconds holds whole microseconds
/// exactly for centuries yet.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_time_now() -> i64 {
    (js::now() * NANOS_PER_MILLI) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_time_clock() -> i64 {
    static START: OnceLock<f64> = OnceLock::new();
    let start = *START.get_or_init(|| js::clock());
    ((js::clock() - start) * NANOS_PER_MILLI) as i64
}

/// A page cannot block, so a wait would freeze it.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_time_sleep(_nanos: i64) {
    trap(b"trap: time.sleep is not available on the web");
}

/// What `std.dom` reaches in the page, by decision 163. A node is a number that
/// the loader gives, and 0 is the place where the page is shown.
mod dom {
    #[link(wasm_import_module = "js")]
    unsafe extern "C" {
        pub fn dom_create(tag: *const u8, len: usize) -> u32;
        pub fn dom_text(content: *const u8, len: usize) -> u32;
        pub fn dom_set_text(node: u32, content: *const u8, len: usize);
        pub fn dom_set(node: u32, name: *const u8, len: usize, value: *const u8, value_len: usize);
        pub fn dom_unset(node: u32, name: *const u8, len: usize);
        pub fn dom_on(node: u32, event: *const u8, len: usize, handler: u32);
        pub fn dom_off(node: u32, event: *const u8, len: usize);
        pub safe fn dom_append(parent: u32, child: u32);
        pub safe fn dom_replace(old: u32, new: u32);
        pub safe fn dom_remove(node: u32);
        /// The length of the value of the event being handled.
        pub safe fn dom_value_len() -> usize;
        /// Writes the value of the event being handled.
        pub fn dom_value(bytes: *mut u8);
    }
}

/// The bytes of a text from compiled code, which lends it for the call.
fn borrowed<'a>(text: *const Text) -> &'a [u8] {
    // SAFETY: compiled code passes a live text, and keeps it for the call.
    unsafe { bytes(text) }
}

/// Traps where a page would run code that is not the program's, as Elm's
/// virtual DOM guards, by decision 168. A handler goes through `dom.On`.
fn guard(tag: &[u8], name: &[u8], value: &[u8]) {
    let name = name.to_ascii_lowercase();
    // A URL parser drops spaces and control characters, so they hide nothing.
    let url: Vec<u8> =
        value.iter().filter(|&&byte| byte > b' ').map(u8::to_ascii_lowercase).collect();
    if tag.eq_ignore_ascii_case(b"script") {
        trap(b"trap: std.dom cannot make a `script` element");
    } else if name.starts_with(b"on") || name == b"formaction" || name == b"srcdoc" {
        trap(b"trap: std.dom cannot set an attribute that runs code: use `dom.On`");
    } else if url.starts_with(b"javascript:") || url.starts_with(b"data:text/html") {
        trap(b"trap: std.dom cannot set a `javascript:` or `data:text/html` value");
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_create(tag: *const Text) -> i64 {
    let tag = borrowed(tag);
    guard(tag, b"", b"");
    // SAFETY: the loader only reads the range.
    i64::from(unsafe { dom::dom_create(tag.as_ptr(), tag.len()) })
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_text(content: *const Text) -> i64 {
    let content = borrowed(content);
    // SAFETY: the loader only reads the range.
    i64::from(unsafe { dom::dom_text(content.as_ptr(), content.len()) })
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_set_text(node: i64, content: *const Text) {
    let content = borrowed(content);
    // SAFETY: the loader only reads the range.
    unsafe { dom::dom_set_text(node as u32, content.as_ptr(), content.len()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_set(node: i64, name: *const Text, value: *const Text) {
    let (name, value) = (borrowed(name), borrowed(value));
    guard(b"", name, value);
    // SAFETY: the loader only reads the ranges.
    unsafe { dom::dom_set(node as u32, name.as_ptr(), name.len(), value.as_ptr(), value.len()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_unset(node: i64, name: *const Text) {
    let name = borrowed(name);
    // SAFETY: the loader only reads the range.
    unsafe { dom::dom_unset(node as u32, name.as_ptr(), name.len()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_on(node: i64, event: *const Text, handler: i64) {
    let event = borrowed(event);
    // SAFETY: the loader only reads the range.
    unsafe { dom::dom_on(node as u32, event.as_ptr(), event.len(), handler as u32) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_off(node: i64, event: *const Text) {
    let event = borrowed(event);
    // SAFETY: the loader only reads the range.
    unsafe { dom::dom_off(node as u32, event.as_ptr(), event.len()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_append(parent: i64, child: i64) {
    dom::dom_append(parent as u32, child as u32);
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_replace(old: i64, new: i64) {
    dom::dom_replace(old as u32, new as u32);
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_remove(node: i64) {
    dom::dom_remove(node as u32);
}

/// A compiled `task(int, text)`: its record, a handler's number, and the
/// event's value. A task that gives `none` gives an `i32`, by decision 166.
type Step = extern "C" fn(i64, i64, i64) -> i32;

std::thread_local! {
    /// The task value that handles the next event, which `std.dom` gives.
    static STEP: Cell<*mut Head> = const { Cell::new(std::ptr::null_mut()) };
}

/// Keeps `step` to handle events, in place of the one kept before.
///
/// # Safety
///
/// `step` must be a live `task(int, text)` value, which compiled code lends.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_dom_step(step: *mut Head) {
    // SAFETY: the caller guarantees a live value. The old one has the count
    // that this task added when it kept it.
    unsafe {
        lucid_retain(step);
        lucid_release(STEP.replace(step));
    }
}

/// Runs the kept step for the handler numbered `handler`. The loader calls
/// this for each event that a node listens for.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_dom_fire(handler: u32) {
    let step = STEP.get();
    if step.is_null() {
        return;
    }
    let value = text_alloc(dom::dom_value_len());
    // SAFETY: `text_alloc` made room for the value's bytes after the header.
    // `step` is a live record whose tag is its task's index in the table, and
    // a step may keep a new one, so this holds a count of its own for the call.
    // A task value takes the count of each argument, by decision 183.
    unsafe {
        dom::dom_value(value.add(1).cast());
        lucid_retain(step);
        let task: Step = std::mem::transmute::<usize, Step>((*step.cast::<Record>()).tag as usize);
        task(step as usize as i64, i64::from(handler), value as usize as i64);
        lucid_release(step);
    }
}
