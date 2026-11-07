//! What the runtime does differently on `wasm32`, by decision 159. Rust's
//! standard library imports nothing on `wasm32-unknown-unknown`, so it cannot
//! print, exit, read a clock, or find random bytes there. The loader gives
//! these as the imports of module `js`. See "WebAssembly" in docs/compiler.md.

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
