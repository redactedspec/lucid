//! `std.process`, by decision 235. A program runs directly, never through a
//! shell, by Rust's `Command`, so the search for a program and the quoting of
//! arguments on Windows are Rust's. Native targets only. A started child is a
//! socket handle whose endpoint is `Endpoint::Child`, so the compiler needs no
//! new type, and the last count closes its pipes without killing it.

use super::{
    Endpoint, List, Socket, Text, bytes, endpoint, lock, read_line, slots, socket, system_text,
    text_from,
};
use std::cell::RefCell;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::process::{ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A started child. Each part has its own lock, so one thread may read its
/// output while another writes its input.
pub(super) struct Child {
    // ponytail: `wait` holds this lock, so a `kill` from another thread waits
    // for the child to end. Poll `try_wait` if a program needs to kill a child
    // that another thread waits for.
    process: Mutex<std::process::Child>,
    input: Mutex<Option<ChildStdin>>,
    output: Mutex<BufReader<ChildStdout>>,
}

std::thread_local! {
    /// How the last `std.process` task failed: 0 if it did not, 1 for a
    /// program not found, 2 for a timeout, or 3 for anything else, with
    /// words. Then what the last `run` printed, to stdout and to stderr.
    static FAILURE: RefCell<(i64, String)> = const { RefCell::new((0, String::new())) };
    static OUTPUT: RefCell<(Vec<u8>, Vec<u8>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

fn result<T>(result: std::io::Result<T>) -> Option<T> {
    FAILURE.set(match &result {
        Ok(_) => (0, String::new()),
        Err(e) if e.kind() == ErrorKind::NotFound => (1, String::new()),
        Err(e) => (3, e.to_string()),
    });
    result.ok()
}

/// The command for `program` with each text of `args`.
///
/// # Safety
///
/// `program` must point to a live text, and `args` to a live `list[text]`.
unsafe fn command(program: *const Text, args: *const List) -> Command {
    // SAFETY: the caller guarantees both, and each slot of `args` is a live text.
    unsafe {
        let mut command = Command::new(system_text(program));
        for &arg in slots(args.cast_mut()).iter() {
            command.arg(system_text(arg as *const Text));
        }
        command
    }
}

/// The exit code, or on Unix the negated signal that ended the child, as
/// Python gives it.
fn code(status: ExitStatus) -> i64 {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return -i64::from(signal);
    }
    status.code().map_or(-1, i64::from)
}

/// Reads all of a pipe on its own thread, so that a child that fills one
/// pipe while the other waits cannot block.
fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

/// Runs a program to its end, with `input` on its stdin, and gives its exit
/// code. What it printed waits for `lucid_process_out` and `_err`. With
/// `seconds` above 0, a child that takes longer is killed, and the failure is
/// a timeout.
///
/// # Safety
///
/// `program` and `input` must point to live texts, and `args` to a live
/// `list[text]`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_process_run(
    program: *const Text,
    args: *const List,
    input: *const Text,
    seconds: i64,
) -> i64 {
    OUTPUT.set((Vec::new(), Vec::new()));
    // SAFETY: the caller guarantees all three.
    let (mut command, input) = unsafe { (command(program, args), bytes(input).to_vec()) };
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let Some(mut child) = result(command.spawn()) else { return -1 };
    let mut stdin = child.stdin.take();
    // A child that never reads its input must not block this thread.
    std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(&input);
        }
    });
    let (out, err) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let status = if seconds > 0 {
        let deadline = Instant::now() + Duration::from_secs(seconds as u64);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    // The readers may wait on a grandchild that holds a pipe,
                    // so they are left to end alone.
                    FAILURE.set((2, String::new()));
                    return -1;
                }
                // ponytail: polls every 10 ms. Wait on a thread with a
                // channel if a program needs a sharper timeout.
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => break Err(e),
            }
        }
    } else {
        child.wait()
    };
    let Some(status) = result(status) else { return -1 };
    let joined = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    OUTPUT.set(joined);
    code(status)
}

/// What the last `run` printed to stdout. Bytes that are not UTF-8 have their
/// bad parts replaced.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_process_out() -> *mut Text {
    OUTPUT.with_borrow(|(out, _)| text_from(String::from_utf8_lossy(out).as_bytes()))
}

/// What the last `run` printed to stderr.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_process_err() -> *mut Text {
    OUTPUT.with_borrow(|(_, err)| text_from(String::from_utf8_lossy(err).as_bytes()))
}

/// Starts a program with pipes for its stdin and stdout. Its stderr is the
/// program's own.
///
/// # Safety
///
/// `program` must point to a live text, and `args` to a live `list[text]`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_process_start(program: *const Text, args: *const List) -> *mut Socket {
    // SAFETY: the caller guarantees both.
    let mut command = unsafe { command(program, args) };
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    let child = result(command.spawn()).map(|mut process| {
        let input = Mutex::new(process.stdin.take());
        let output = Mutex::new(BufReader::new(process.stdout.take().expect("piped")));
        Endpoint::Child(Box::new(Child { process: Mutex::new(process), input, output }))
    });
    socket(child)
}

/// The child of a live socket.
///
/// # Safety
///
/// `child` must point to a live socket.
unsafe fn child<'a>(child: *const Socket) -> &'a Child {
    // SAFETY: the caller guarantees a live socket.
    match unsafe { endpoint(child) } {
        Endpoint::Child(child) => child,
        _ => super::trap(b"trap: a socket used as a child"),
    }
}

/// Writes all of a text to the child's stdin.
///
/// # Safety
///
/// `child` must point to a live child, and `content` to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_process_write(child: *const Socket, content: *const Text) {
    // SAFETY: the caller guarantees both.
    let (child, content) = unsafe { (self::child(child), bytes(content)) };
    let written = match lock(&child.input).as_mut() {
        Some(input) => input.write_all(content).and_then(|()| input.flush()),
        None => Err(std::io::Error::other("the input is closed")),
    };
    result(written);
}

/// Closes the child's stdin, so it reads the end of its input.
///
/// # Safety
///
/// `child` must point to a live child.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_process_close_input(child: *const Socket) {
    // SAFETY: the caller guarantees a live child.
    lock(&unsafe { self::child(child) }.input).take();
}

/// The next line of the child's stdout, as `console.read_line` reads stdin.
///
/// # Safety
///
/// `child` must point to a live child.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_process_read_line(child: *const Socket) -> *mut Text {
    // SAFETY: the caller guarantees a live child.
    read_line(&mut *lock(&unsafe { self::child(child) }.output))
}

/// Closes the child's stdin, as Rust's `wait` does, waits for the child to
/// end, and gives its exit code.
///
/// # Safety
///
/// `child` must point to a live child.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_process_wait(child: *const Socket) -> i64 {
    // SAFETY: the caller guarantees a live child.
    let child = unsafe { self::child(child) };
    lock(&child.input).take();
    result(lock(&child.process).wait()).map_or(-1, code)
}

/// Kills the child. A child that has already ended is not a failure.
///
/// # Safety
///
/// `child` must point to a live child.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_process_kill(child: *const Socket) {
    // SAFETY: the caller guarantees a live child.
    result(lock(&unsafe { self::child(child) }.process).kill());
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_process_failure() -> i64 {
    FAILURE.with_borrow(|(kind, _)| *kind)
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_process_detail() -> *mut Text {
    FAILURE.with_borrow(|(_, detail)| text_from(detail.as_bytes()))
}
