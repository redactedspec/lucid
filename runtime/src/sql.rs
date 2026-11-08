//! `std.sql`, by decisions 194 and 195: SQLite through `libsqlite3-sys`,
//! built from its bundled source in serialized mode, so a connection may go
//! to another thread. Native targets only. A connection and a statement are
//! both a `Sql` handle, which the last count closes.

use super::{
    AtomicI64, Head, Kind, List, OUTSIDE_FILES, Text, byte_list, check_live, failed, file_allowed,
    list_bytes, object, system_text, text_from, trap,
};
use libsqlite3_sys as ffi;
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::null_mut;

// The crate's bundled bindings leave this out, though the bundled build
// has it. It waits for the last statement before it closes.
unsafe extern "C" {
    fn sqlite3_close_v2(db: *mut ffi::sqlite3) -> c_int;
}

/// A connection when `statement` is null, or else a statement on `db`.
/// `sqlite3_close_v2` waits for the last statement, so a statement needs no
/// count on its connection.
#[repr(C)]
pub struct Sql {
    head: Head,
    /// `release`, reached through this field so that a program which never
    /// opens a database does not link SQLite.
    pub(super) release: unsafe fn(*mut Head),
    db: *mut ffi::sqlite3,
    statement: *mut ffi::sqlite3_stmt,
}

std::thread_local! {
    /// How the last `std.sql` task failed: 0 if it did not, 1 for a broken
    /// constraint, 2 for a busy database, or 3 for anything else, with the
    /// words of SQLite or the runtime.
    static SQL_FAILURE: RefCell<(i64, String)> = const { RefCell::new((0, String::new())) };
}

fn problem(detail: &str) {
    SQL_FAILURE.set((3, detail.to_string()));
}

/// Keeps what `std.sql` asks for next. `db` must be live, and this thread
/// must hold its mutex, so no other thread changes the message first.
unsafe fn result(db: *mut ffi::sqlite3, code: c_int) -> c_int {
    let failure = match code & 0xff {
        ffi::SQLITE_OK | ffi::SQLITE_ROW | ffi::SQLITE_DONE => (0, String::new()),
        kind => {
            // SAFETY: the caller guarantees a live connection.
            let message = unsafe { CStr::from_ptr(ffi::sqlite3_errmsg(db)) };
            let message = message.to_string_lossy().into_owned();
            match kind {
                ffi::SQLITE_CONSTRAINT => (1, message),
                ffi::SQLITE_BUSY | ffi::SQLITE_LOCKED => (2, message),
                _ => (3, message),
            }
        }
    };
    SQL_FAILURE.set(failure);
    code
}

/// Runs `call` with the connection's mutex held, and keeps its result.
unsafe fn locked(db: *mut ffi::sqlite3, call: impl FnOnce() -> c_int) -> c_int {
    // SAFETY: the caller guarantees a live connection. Its mutex is
    // recursive, so `call` may take it again.
    unsafe {
        let mutex = ffi::sqlite3_db_mutex(db);
        ffi::sqlite3_mutex_enter(mutex);
        let code = result(db, call());
        ffi::sqlite3_mutex_leave(mutex);
        code
    }
}

fn handle(db: *mut ffi::sqlite3, statement: *mut ffi::sqlite3_stmt) -> *mut Sql {
    let head = Head { count: AtomicI64::new(1), kind: Kind::Sql };
    object(size_of::<Sql>(), Sql { head, release, db, statement })
}

/// Closes a connection, or finalizes a statement, at its last count.
///
/// # Safety
///
/// `value` must point to a `Sql` with no count left.
unsafe fn release(value: *mut Head) {
    // SAFETY: the caller guarantees this.
    unsafe {
        let sql = &*value.cast::<Sql>();
        if sql.statement.is_null() {
            sqlite3_close_v2(sql.db);
        } else {
            ffi::sqlite3_finalize(sql.statement);
        }
    }
}

/// The handle of a live `Sql`.
///
/// # Safety
///
/// `sql` must point to a live `Sql`.
unsafe fn live<'a>(sql: *const Sql) -> &'a Sql {
    // SAFETY: the caller guarantees this.
    unsafe {
        check_live(sql.cast());
        &*sql
    }
}

/// The statement of a live handle. Only a `Db` built by hand reaches the trap.
unsafe fn statement(sql: *const Sql) -> (*mut ffi::sqlite3, *mut ffi::sqlite3_stmt) {
    // SAFETY: the caller guarantees a live handle.
    let sql = unsafe { live(sql) };
    if sql.statement.is_null() {
        trap(b"trap: a connection used as a statement");
    }
    (sql.db, sql.statement)
}

/// Refuses what could open a file that the `files` permission does not
/// list: `ATTACH`, and the pragmas that move temporary or data files.
/// `VACUUM` attaches the literal empty name, a temporary database, so it
/// still works, and `VACUUM INTO` a file does not. SQLite gives a null name
/// for any name that is not a literal, such as a `?` mark, so null is refused.
unsafe extern "C" fn authorize(
    _: *mut c_void,
    action: c_int,
    first: *const c_char,
    _: *const c_char,
    _: *const c_char,
    _: *const c_char,
) -> c_int {
    // SAFETY: SQLite gives a live C string, or null.
    let name = || (!first.is_null()).then(|| unsafe { CStr::from_ptr(first) }.to_bytes());
    let refused = match action {
        ffi::SQLITE_ATTACH => name() != Some(b""),
        ffi::SQLITE_PRAGMA => name().is_some_and(|pragma| {
            pragma.eq_ignore_ascii_case(b"temp_store_directory")
                || pragma.eq_ignore_ascii_case(b"data_store_directory")
        }),
        _ => false,
    };
    if refused { ffi::SQLITE_DENY } else { ffi::SQLITE_OK }
}

/// Opens or creates the database at `path`, or an empty one in memory for
/// `:memory:`. Any other path must be inside the `files` permission.
///
/// # Safety
///
/// `path` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_open(path: *const Text) -> *mut Sql {
    // SAFETY: the caller guarantees a live text.
    let path = unsafe { system_text(path) };
    if path != ":memory:" && !file_allowed(path) {
        problem(OUTSIDE_FILES);
        return failed();
    }
    // The build reads `file:` names as URIs, which could name another file,
    // so such a name gets `./` to stay a plain path.
    let plain = if path.get(..5).is_some_and(|start| start.eq_ignore_ascii_case("file:")) {
        format!("./{path}")
    } else {
        path.to_string()
    };
    let Ok(name) = CString::new(plain) else {
        problem("a path cannot hold a zero byte");
        return failed();
    };
    let mut db = null_mut();
    let flags = ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE | ffi::SQLITE_OPEN_FULLMUTEX;
    // SAFETY: `name` is a C string, and `db` takes the new connection.
    let code = unsafe { ffi::sqlite3_open_v2(name.as_ptr(), &mut db, flags, std::ptr::null()) };
    if db.is_null() {
        trap(b"trap: out of memory");
    }
    // SAFETY: `db` is live, even when the open failed.
    unsafe {
        let code = locked(db, || {
            if code != ffi::SQLITE_OK {
                return code;
            }
            ffi::sqlite3_extended_result_codes(db, 1);
            // Another connection's write holds the lock only briefly, so
            // wait for it before `Busy`.
            ffi::sqlite3_busy_timeout(db, 5000);
            ffi::sqlite3_set_authorizer(db, Some(authorize), null_mut())
        });
        if code != ffi::SQLITE_OK {
            sqlite3_close_v2(db);
            return failed();
        }
    }
    handle(db, null_mut())
}

/// Compiles one statement of `query`. More than one is a failure, since
/// SQLite would skip the rest.
///
/// # Safety
///
/// `db` must point to a live connection, and `query` to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_prepare(db: *const Sql, query: *const Text) -> *mut Sql {
    // SAFETY: the caller guarantees both.
    let (db, query) = unsafe { (live(db).db, system_text(query)) };
    let (mut statement, mut tail) = (null_mut(), std::ptr::null());
    let Ok(length) = c_int::try_from(query.len()) else {
        problem("a query is at most 2 GiB");
        return failed();
    };
    let start = query.as_ptr().cast::<c_char>();
    // SAFETY: `query` holds `length` bytes, and the statement and tail come back.
    let code = unsafe {
        locked(db, || ffi::sqlite3_prepare_v2(db, start, length, &mut statement, &mut tail))
    };
    if code != ffi::SQLITE_OK {
        return failed();
    }
    // What follows must compile to no statement, as only spaces, `;`, and
    // comments do. SQLite's own parser decides, so a comment cannot fool it.
    let mut next = null_mut();
    // SAFETY: SQLite sets `tail` inside `query`, or at its end.
    let rest = length - unsafe { tail.cast::<u8>().offset_from(query.as_ptr().cast()) } as c_int;
    let code = if statement.is_null() {
        ffi::SQLITE_OK
    } else {
        // SAFETY: `tail` holds `rest` bytes of `query`.
        unsafe {
            locked(db, || ffi::sqlite3_prepare_v2(db, tail, rest, &mut next, std::ptr::null_mut()))
        }
    };
    if statement.is_null() || code != ffi::SQLITE_OK || !next.is_null() {
        // SAFETY: each is a statement or null, which is a no-op.
        unsafe {
            ffi::sqlite3_finalize(statement);
            ffi::sqlite3_finalize(next);
        }
        if code == ffi::SQLITE_OK {
            problem("a query holds exactly one statement");
        }
        return failed();
    }
    handle(db, statement)
}

/// How many `?` marks the statement has.
///
/// # Safety
///
/// `statement` must point to a live statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_marks(statement: *const Sql) -> i64 {
    // SAFETY: the caller guarantees a live statement.
    let (_, statement) = unsafe { self::statement(statement) };
    // SAFETY: `statement` is live.
    i64::from(unsafe { ffi::sqlite3_bind_parameter_count(statement) })
}

/// Binds one value, by `bind`, to the 1-based `?` mark `at`.
unsafe fn bind(
    statement: *const Sql,
    at: i64,
    bind: impl FnOnce(*mut ffi::sqlite3_stmt, c_int) -> c_int,
) {
    // SAFETY: the caller guarantees a live statement.
    let (db, statement) = unsafe { self::statement(statement) };
    let at = c_int::try_from(at).unwrap_or(0);
    // SAFETY: `db` is live.
    unsafe { locked(db, || bind(statement, at)) };
}

/// # Safety
///
/// `statement` must point to a live statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_bind_null(statement: *const Sql, at: i64) {
    // SAFETY: the caller guarantees a live statement.
    unsafe { bind(statement, at, |s, at| ffi::sqlite3_bind_null(s, at)) }
}

/// # Safety
///
/// `statement` must point to a live statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_bind_int(statement: *const Sql, at: i64, value: i64) {
    // SAFETY: the caller guarantees a live statement.
    unsafe { bind(statement, at, |s, at| ffi::sqlite3_bind_int64(s, at, value)) }
}

/// # Safety
///
/// `statement` must point to a live statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_bind_float(statement: *const Sql, at: i64, value: f64) {
    // SAFETY: the caller guarantees a live statement.
    unsafe { bind(statement, at, |s, at| ffi::sqlite3_bind_double(s, at, value)) }
}

/// # Safety
///
/// `statement` must point to a live statement, and `value` to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_bind_text(statement: *const Sql, at: i64, value: *const Text) {
    // SAFETY: the caller guarantees a live text.
    let value = unsafe { system_text(value) };
    let length = value.len() as u64;
    let start = value.as_ptr().cast();
    // SAFETY: SQLite copies the bytes, since the text may go first.
    unsafe {
        bind(statement, at, |s, at| {
            ffi::sqlite3_bind_text64(
                s,
                at,
                start,
                length,
                ffi::SQLITE_TRANSIENT(),
                ffi::SQLITE_UTF8 as u8,
            )
        })
    }
}

/// # Safety
///
/// `statement` must point to a live statement, and `value` to a live
/// `list[byte]`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_bind_bytes(statement: *const Sql, at: i64, value: *const List) {
    // SAFETY: the caller guarantees a live list.
    let value = unsafe { list_bytes(value) };
    let (start, length) = (value.as_ptr().cast(), value.len() as u64);
    // SAFETY: SQLite copies the bytes, since the list may go first.
    unsafe {
        bind(statement, at, |s, at| {
            ffi::sqlite3_bind_blob64(s, at, start, length, ffi::SQLITE_TRANSIENT())
        })
    }
}

/// Runs the statement to its next row. True if there is one, and false when
/// it is done or failed.
///
/// # Safety
///
/// `statement` must point to a live statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_step(statement: *const Sql) -> bool {
    // SAFETY: the caller guarantees a live statement.
    let (db, statement) = unsafe { self::statement(statement) };
    // SAFETY: both are live.
    unsafe { locked(db, || ffi::sqlite3_step(statement)) == ffi::SQLITE_ROW }
}

/// Runs the statement to its end, and gives how many rows it changed, with
/// those that triggers and foreign keys changed. The mutex is held, so
/// another thread's changes do not count.
///
/// # Safety
///
/// `statement` must point to a live statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_run(statement: *const Sql) -> i64 {
    // SAFETY: the caller guarantees a live statement.
    let (db, statement) = unsafe { self::statement(statement) };
    let mut changed = 0;
    // SAFETY: both are live.
    unsafe {
        locked(db, || {
            let before = ffi::sqlite3_total_changes64(db);
            loop {
                let code = ffi::sqlite3_step(statement);
                if code != ffi::SQLITE_ROW {
                    changed = ffi::sqlite3_total_changes64(db) - before;
                    return code;
                }
            }
        })
    };
    changed
}

/// How many columns each row has.
///
/// # Safety
///
/// `statement` must point to a live statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_columns(statement: *const Sql) -> i64 {
    // SAFETY: the caller guarantees a live statement.
    i64::from(unsafe { ffi::sqlite3_column_count(self::statement(statement).1) })
}

/// The statement and a column that `std.sql` asks for, which is in range.
unsafe fn column(statement: *const Sql, at: i64) -> (*mut ffi::sqlite3_stmt, c_int) {
    // SAFETY: the caller guarantees a live statement.
    let (_, statement) = unsafe { self::statement(statement) };
    // SAFETY: `statement` is live.
    let count = unsafe { ffi::sqlite3_column_count(statement) };
    match c_int::try_from(at) {
        Ok(at) if (0..count).contains(&at) => (statement, at),
        _ => trap(b"trap: a column past the last"),
    }
}

/// A text from SQLite, which may not be UTF-8: bad parts are replaced.
unsafe fn sqlite_text(start: *const u8, length: c_int) -> *mut Text {
    if start.is_null() {
        return text_from(b"");
    }
    // SAFETY: SQLite gives `length` bytes at `start`.
    let bytes = unsafe { std::slice::from_raw_parts(start, length as usize) };
    text_from(String::from_utf8_lossy(bytes).as_bytes())
}

/// The name of a column, as the query gives it.
///
/// # Safety
///
/// `statement` must point to a live statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_column_name(statement: *const Sql, at: i64) -> *mut Text {
    // SAFETY: the caller guarantees a live statement, and `column` checks `at`.
    unsafe {
        let (statement, at) = column(statement, at);
        let name = ffi::sqlite3_column_name(statement, at);
        if name.is_null() {
            trap(b"trap: out of memory");
        }
        text_from(CStr::from_ptr(name).to_bytes())
    }
}

// ponytail: `query` makes two calls a cell, kind then value, each checking
// the column. Return both from one call if large results prove slow.
/// The kind of a value in the current row: 1 for an integer, 2 for a float,
/// 3 for text, 4 for bytes, and 5 for null, as SQLite numbers them.
///
/// # Safety
///
/// `statement` must point to a live statement on a row.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_column_kind(statement: *const Sql, at: i64) -> i64 {
    // SAFETY: the caller guarantees a live statement, and `column` checks `at`.
    unsafe {
        let (statement, at) = column(statement, at);
        i64::from(ffi::sqlite3_column_type(statement, at))
    }
}

/// # Safety
///
/// `statement` must point to a live statement on a row.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_column_int(statement: *const Sql, at: i64) -> i64 {
    // SAFETY: the caller guarantees a live statement, and `column` checks `at`.
    unsafe {
        let (statement, at) = column(statement, at);
        ffi::sqlite3_column_int64(statement, at)
    }
}

/// # Safety
///
/// `statement` must point to a live statement on a row.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_column_float(statement: *const Sql, at: i64) -> f64 {
    // SAFETY: the caller guarantees a live statement, and `column` checks `at`.
    unsafe {
        let (statement, at) = column(statement, at);
        ffi::sqlite3_column_double(statement, at)
    }
}

/// # Safety
///
/// `statement` must point to a live statement on a row.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_column_text(statement: *const Sql, at: i64) -> *mut Text {
    // SAFETY: the caller guarantees a live statement, and `column` checks `at`.
    // The bytes come before the length, as SQLite's docs ask.
    unsafe {
        let (statement, at) = column(statement, at);
        let start = ffi::sqlite3_column_text(statement, at);
        sqlite_text(start, ffi::sqlite3_column_bytes(statement, at))
    }
}

/// # Safety
///
/// `statement` must point to a live statement on a row.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_sql_column_bytes(statement: *const Sql, at: i64) -> *mut List {
    // SAFETY: the caller guarantees a live statement, and `column` checks `at`.
    // The bytes come before the length, as SQLite's docs ask.
    unsafe {
        let (statement, at) = column(statement, at);
        let start = ffi::sqlite3_column_blob(statement, at).cast::<u8>();
        let length = ffi::sqlite3_column_bytes(statement, at) as usize;
        if start.is_null() {
            return byte_list(&[]);
        }
        byte_list(std::slice::from_raw_parts(start, length))
    }
}

/// The kind of the last failure on this thread. See `SQL_FAILURE`.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_sql_failure() -> i64 {
    SQL_FAILURE.with_borrow(|failure| failure.0)
}

/// The words of the last failure on this thread.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_sql_detail() -> *mut Text {
    SQL_FAILURE.with_borrow(|failure| text_from(failure.1.as_bytes()))
}
