//! Lucid runtime. Compiled code calls these tasks through the C ABI.
//!
//! Debug builds, which run the golden tests, keep count of live values and
//! never reuse their memory. So a leak or double free stops the program.

use std::alloc::{Layout, alloc, dealloc};
use std::hash::{BuildHasher, RandomState};
use std::io::Write;
use std::mem::offset_of;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

const STDOUT_FAILED: &[u8] = b"trap: cannot write to stdout";
const OUT_OF_MEMORY: &[u8] = b"trap: out of memory";

/// Starts every heap value, so one `retain` and `release` serve them all.
#[repr(C)]
pub struct Head {
    count: usize,
    kind: Kind,
}

/// What a value is, and which of its slots hold counted values.
#[repr(u8)]
#[derive(Clone, Copy)]
enum Kind {
    Text,
    List { counted: bool },
    Map { key_counted: bool, value_counted: bool },
    Record,
}

/// A counted, immutable UTF-8 text. Its bytes follow this header.
#[repr(C)]
pub struct Text {
    head: Head,
    len: usize,
}

/// A counted list. `cap` 8-byte slots follow this header.
#[repr(C)]
pub struct List {
    head: Head,
    len: usize,
    cap: usize,
}

/// A counted map, or a set, whose values are unused. `cap` entries follow this
/// header in insertion order, then a table of `2 * cap` indexes, the layout of
/// Python's compact dict. `len` counts live entries, and `used` counts those
/// and the removed ones. `cap` is a power of two. `keys` is set if a key is a
/// value made of other values.
#[repr(C)]
pub struct Map {
    head: Head,
    len: usize,
    used: usize,
    cap: usize,
    keys: Option<KeyTasks>,
}

/// A compiled task that hashes a key, given a starting state.
pub type HashTask = extern "C" fn(i64, i64) -> i64;

/// A compiled task that gives 1 if two keys are equal, and otherwise 0.
pub type EqualTask = extern "C" fn(i64, i64) -> u8;

/// What the runtime cannot do alone for a compound key. Keys are borrowed, and
/// these tasks only read them, so they may run during a search of a map.
#[derive(Clone, Copy)]
struct KeyTasks {
    hash: HashTask,
    equal: EqualTask,
}

/// A counted `type` or `choice` value. `tag` is its variant. `len` 8-byte slots
/// follow this header, and the first `counted` of them hold counted values.
#[repr(C)]
pub struct Record {
    head: Head,
    tag: usize,
    len: usize,
    counted: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    /// `REMOVED`, or a hash with its top bit set.
    hash: u64,
    key: i64,
    value: i64,
}

const REMOVED: u64 = 0;
const LIVE_HASH: u64 = 1 << 63;

/// In the table of a map: no entry has been here. Any other value but
/// `TABLE_REMOVED` is an entry's position plus one.
const TABLE_EMPTY: usize = 0;
const TABLE_REMOVED: usize = usize::MAX;

const SMALLEST_LIST: usize = 4;
const SMALLEST_MAP: usize = 8;

const HEADER: usize = size_of::<Text>();

/// Compiled code reads a list's or a map's length here, and a list's slots
/// from `SLOTS_OFFSET`, so that a read needs no call.
pub const LEN_OFFSET: i32 = offset_of!(List, len) as i32;
pub const SLOTS_OFFSET: i32 = size_of::<List>() as i32;
const _: () = assert!(offset_of!(Map, len) == offset_of!(List, len));

/// Compiled code reads a record's variant and slots here.
pub const TAG_OFFSET: i32 = offset_of!(Record, tag) as i32;
pub const RECORD_SLOTS_OFFSET: i32 = size_of::<Record>() as i32;

/// Debug builds only: the count of a released value.
const DEAD: usize = usize::MAX;

/// Debug builds only: the number of values not yet released.
static LIVE: AtomicUsize = AtomicUsize::new(0);

fn text_size(len: usize) -> usize {
    len.saturating_add(HEADER)
}

fn list_size(cap: usize) -> usize {
    cap.saturating_mul(size_of::<i64>()).saturating_add(size_of::<List>())
}

fn map_size(cap: usize) -> usize {
    let per_entry = size_of::<Entry>() + 2 * size_of::<usize>();
    cap.saturating_mul(per_entry).saturating_add(size_of::<Map>())
}

fn record_size(len: usize) -> usize {
    len.saturating_mul(size_of::<i64>()).saturating_add(size_of::<Record>())
}

fn layout(size: usize) -> Layout {
    Layout::from_size_align(size, align_of::<Head>()).unwrap_or_else(|_| trap(OUT_OF_MEMORY))
}

/// Allocates `size` bytes, which start with `value`. Its count must be one.
fn object<T>(size: usize, value: T) -> *mut T {
    // SAFETY: the layout is never zero sized, because of the header.
    let object = unsafe { alloc(layout(size)) }.cast::<T>();
    if object.is_null() {
        trap(OUT_OF_MEMORY);
    }
    if cfg!(debug_assertions) {
        LIVE.fetch_add(1, Ordering::Relaxed);
    }
    // SAFETY: the allocation holds a `T`, and all `T` here align like `Head`.
    unsafe { object.write(value) };
    object
}

/// Frees the memory of a value, without touching what its slots hold.
///
/// # Safety
///
/// `value` must be a live value of `size` bytes, and nothing may use it again.
unsafe fn free(value: *mut Head, size: usize) {
    if cfg!(debug_assertions) {
        LIVE.fetch_sub(1, Ordering::Relaxed);
        // SAFETY: the caller guarantees a live value.
        unsafe { (*value).count = DEAD };
    } else {
        // SAFETY: the caller guarantees the size, and `object` used this layout.
        unsafe { dealloc(value.cast(), layout(size)) };
    }
}

/// Traps on a released value, in debug builds.
///
/// # Safety
///
/// `value` must be readable, which a debug build keeps true after release.
unsafe fn check_live(value: *const Head) {
    // SAFETY: the caller guarantees this.
    if cfg!(debug_assertions) && unsafe { (*value).count } == DEAD {
        trap(b"trap: runtime bug: use after free");
    }
}

/// Allocates a text with a count of one. The caller fills its `len` bytes.
fn text_alloc(len: usize) -> *mut Text {
    object(text_size(len), Text { head: Head { count: 1, kind: Kind::Text }, len })
}

/// # Safety
///
/// `text` must point to a live text. The slice must not outlive it.
unsafe fn bytes<'a>(text: *const Text) -> &'a [u8] {
    // SAFETY: the caller guarantees a live text, whose bytes follow the header.
    unsafe {
        check_live(text.cast());
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
pub unsafe extern "C" fn lucid_retain(value: *mut Head) {
    // SAFETY: the caller guarantees a live value.
    unsafe {
        check_live(value);
        (*value).count += 1;
    }
}

/// Gives up one count of `value`, which may be null: a local starts that way.
/// The last count releases what the value holds, and frees it.
///
/// A recursive `choice` can nest as deep as memory allows, so this never
/// recurses. A value waiting here keeps the next one in its `count`, which is
/// unused once it reaches zero. So this needs no stack and no allocation.
///
/// # Safety
///
/// `value` must be null, or point to a live value that this caller has a count of.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_release(value: *mut Head) {
    let mut pending = std::ptr::null_mut();
    // SAFETY: the caller guarantees a live value, `kind` tells each value's
    // layout, and a value on `pending` has no other holder.
    unsafe {
        drop_count(value, &mut pending);
        while !pending.is_null() {
            let value = pending;
            pending = (*value).count as *mut Head;
            match (*value).kind {
                Kind::Text => free(value, text_size((*value.cast::<Text>()).len)),
                Kind::List { counted } => {
                    let list = value.cast::<List>();
                    if counted {
                        for &slot in slots(list).iter() {
                            drop_count(slot as *mut Head, &mut pending);
                        }
                    }
                    free(value, list_size((*list).cap));
                }
                Kind::Map { key_counted, value_counted } => {
                    let map = value.cast::<Map>();
                    for entry in entries(map).iter().filter(|entry| entry.hash != REMOVED) {
                        if key_counted {
                            drop_count(entry.key as *mut Head, &mut pending);
                        }
                        if value_counted {
                            drop_count(entry.value as *mut Head, &mut pending);
                        }
                    }
                    free(value, map_size((*map).cap));
                }
                Kind::Record => {
                    let record = value.cast::<Record>();
                    for &slot in &record_slots(record)[..(*record).counted] {
                        drop_count(slot as *mut Head, &mut pending);
                    }
                    free(value, record_size((*record).len));
                }
            }
        }
    }
}

/// Gives up one count of `value`. The last count adds it to `pending`.
///
/// # Safety
///
/// `value` must be null, or point to a live value that this caller has a count of.
unsafe fn drop_count(value: *mut Head, pending: &mut *mut Head) {
    if value.is_null() {
        return;
    }
    // SAFETY: the caller guarantees a live value.
    unsafe {
        if cfg!(debug_assertions) && (*value).count == DEAD {
            trap(b"trap: runtime bug: double free");
        }
        (*value).count -= 1;
        if (*value).count == 0 {
            (*value).count = *pending as usize;
            *pending = value;
        }
    }
}

/// A slot of a counted collection may be null while compiled code writes
/// through it. See "Counting" in mir.rs.
///
/// # Safety
///
/// `slot` must be zero, or hold a live value.
unsafe fn retain_slot(slot: i64) {
    if slot != 0 {
        // SAFETY: the caller guarantees this.
        unsafe { lucid_retain(slot as *mut Head) };
    }
}

/// # Safety
///
/// `slot` must be zero, or hold a live value that this caller has a count of.
unsafe fn release_slot(slot: i64) {
    // SAFETY: the caller guarantees this.
    unsafe { lucid_release(slot as *mut Head) };
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

/// The bytes of a text, one to a slot.
///
/// A new text: `text` as a literal, which reads back as the same value.
///
/// # Safety
///
/// `text` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_quote(text: *const Text) -> *mut Text {
    // SAFETY: the caller guarantees a live text, whose bytes are valid UTF-8.
    let source = unsafe { std::str::from_utf8_unchecked(bytes(text)) };
    let mut out = String::with_capacity(source.len() + 2);
    out.push('"');
    for character in source.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            other => out.push(other),
        }
    }
    out.push('"');
    text_from(out.as_bytes())
}

/// # Safety
///
/// `text` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_text_bytes(text: *const Text) -> *mut List {
    // SAFETY: the caller guarantees a live text.
    let bytes = unsafe { bytes(text) };
    // ponytail: 8 bytes per byte, since every slot has one size. Per-type
    // layouts arrive with monomorphization, if M4 measurements ask for them.
    let list = list_alloc(bytes.len(), false);
    // SAFETY: `list_alloc` made room for `len` slots.
    unsafe {
        (*list).len = bytes.len();
        for (slot, &byte) in slots(list).iter_mut().zip(bytes) {
            *slot = i64::from(byte);
        }
    }
    list
}

fn list_alloc(cap: usize, counted: bool) -> *mut List {
    let head = Head { count: 1, kind: Kind::List { counted } };
    object(list_size(cap), List { head, len: 0, cap })
}

/// # Safety
///
/// `list` must point to a live list, and no other slice of it may be in use.
/// The slice must not outlive it.
unsafe fn slots<'a>(list: *mut List) -> &'a mut [i64] {
    // SAFETY: the caller guarantees a live list, whose slots follow the header.
    unsafe {
        check_live(list.cast());
        std::slice::from_raw_parts_mut(list.add(1).cast(), (*list).len)
    }
}

fn is_counted(list: *const List) -> bool {
    // SAFETY: every caller holds a live list.
    matches!(unsafe { (*list).head.kind }, Kind::List { counted: true })
}

/// Returns a list that only this caller holds, with room for `cap` slots. This
/// is where a logical copy becomes a real one: `list` itself if the caller has
/// the only count, else a copy, which gives up the caller's count of `list`.
///
/// # Safety
///
/// `list` must point to a live list that this caller has a count of.
unsafe fn unique_list(list: *mut List, cap: usize) -> *mut List {
    // SAFETY: the caller guarantees a live list, and each path below keeps its
    // one count: moved to the copy, or dropped because the copy retains.
    unsafe {
        let shared = (*list).head.count > 1;
        if !shared && (*list).cap >= cap {
            return list;
        }
        let counted = is_counted(list);
        let copy = list_alloc(cap, counted);
        (*copy).len = (*list).len;
        let old = slots(list);
        slots(copy).copy_from_slice(old);
        if shared {
            if counted {
                old.iter().for_each(|&slot| retain_slot(slot));
            }
            (*list).head.count -= 1;
        } else {
            free(list.cast(), list_size((*list).cap));
        }
        copy
    }
}

/// An empty list with room for `cap` slots, which it holds counts of if
/// `counted` is true.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_list_new(cap: usize, counted: bool) -> *mut List {
    list_alloc(cap, counted)
}

/// Adds a value at the end, and returns the list that replaces `list`.
///
/// # Safety
///
/// `list` must point to a live list that this caller gives one count of.
/// `value` must fit the list's slots, and is borrowed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_list_push(list: *mut List, value: i64) -> *mut List {
    // SAFETY: the caller guarantees a live list, and `cap` makes room.
    unsafe {
        let (len, cap) = ((*list).len, (*list).cap);
        let cap = if len == cap { cap.saturating_mul(2).max(SMALLEST_LIST) } else { cap };
        if is_counted(list) {
            retain_slot(value);
        }
        let list = unique_list(list, cap);
        list.add(1).cast::<i64>().add(len).write(value);
        (*list).len = len + 1;
        list
    }
}

/// True if `0 <= index < len`.
fn in_bounds(list: *const List, index: i64) -> bool {
    // SAFETY: every caller holds a live list.
    (index as u64) < unsafe { (*list).len } as u64
}

/// Replaces a value, and returns the list that replaces `list`. Returns null
/// if `index` is out of bounds, and compiled code then traps.
///
/// # Safety
///
/// As for `lucid_list_push`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_list_set(list: *mut List, index: i64, value: i64) -> *mut List {
    if !in_bounds(list, index) {
        return std::ptr::null_mut();
    }
    // SAFETY: the caller guarantees a live list, and the index is in bounds.
    // The new value is retained first, since it may be the old one.
    unsafe {
        let counted = is_counted(list);
        if counted {
            retain_slot(value);
        }
        let list = unique_list(list, (*list).cap);
        let slot = &mut slots(list)[index as usize];
        let old = std::mem::replace(slot, value);
        if counted {
            release_slot(old);
        }
        list
    }
}

/// Drops the last value, and returns the list that replaces `list`.
///
/// # Safety
///
/// `list` must point to a live list that this caller gives one count of, and
/// it must not be empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_list_pop(list: *mut List) -> *mut List {
    // SAFETY: the caller guarantees a live list with a last value, and
    // `unique_list` keeps that. A caller that wants the value retains it first.
    unsafe {
        let counted = is_counted(list);
        let list = unique_list(list, (*list).cap);
        let last = (*list).len - 1;
        (*list).len = last;
        if counted {
            release_slot(list.add(1).cast::<i64>().add(last).read());
        }
        list
    }
}

/// Exchanges two values, and returns the list that replaces `list`. Returns
/// null if an index is out of bounds, and compiled code then traps.
///
/// # Safety
///
/// `list` must point to a live list that this caller gives one count of.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_list_swap(list: *mut List, a: i64, b: i64) -> *mut List {
    if !in_bounds(list, a) || !in_bounds(list, b) {
        return std::ptr::null_mut();
    }
    // SAFETY: the caller guarantees a live list, and both indexes are in bounds.
    unsafe {
        let list = unique_list(list, (*list).cap);
        slots(list).swap(a as usize, b as usize);
        list
    }
}

/// True if a list holds this value. Slots of a counted list hold text here.
///
/// # Safety
///
/// `list` must point to a live list. `value` must fit its slots.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_list_has(list: *mut List, value: i64) -> bool {
    let counted = is_counted(list);
    // SAFETY: the caller guarantees a live list, and borrows hold for this call.
    unsafe { slots(list).iter().any(|&slot| same_key(counted, slot, value)) }
}

/// A new list of the elements from `start` up to but not including `end`.
/// Returns null if the range is not `0 <= start <= end <= len`, and compiled
/// code then traps.
///
/// # Safety
///
/// `list` must point to a live list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_list_slice(list: *mut List, start: i64, end: i64) -> *mut List {
    // SAFETY: the caller guarantees a live list, and the range is checked here.
    unsafe {
        let old = slots(list);
        if start < 0 || end < start || end as u64 > old.len() as u64 {
            return std::ptr::null_mut();
        }
        let part = &old[start as usize..end as usize];
        let counted = is_counted(list);
        let copy = list_alloc(part.len(), counted);
        (*copy).len = part.len();
        slots(copy).copy_from_slice(part);
        if counted {
            part.iter().for_each(|&slot| retain_slot(slot));
        }
        copy
    }
}

/// A record with a count of one. Compiled code fills all `len` slots at once,
/// and retains those below `counted`, before anything can read them.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_record_new(tag: usize, len: usize, counted: usize) -> *mut Record {
    let head = Head { count: 1, kind: Kind::Record };
    object(record_size(len), Record { head, tag, len, counted })
}

/// # Safety
///
/// `record` must point to a live record, and no other slice of it may be in
/// use. The slice must not outlive it.
unsafe fn record_slots<'a>(record: *mut Record) -> &'a mut [i64] {
    // SAFETY: the caller guarantees a live record, whose slots follow the header.
    unsafe {
        check_live(record.cast());
        std::slice::from_raw_parts_mut(record.add(1).cast(), (*record).len)
    }
}

/// Replaces a value, and returns the record that replaces `record`: `record`
/// itself if the caller has the only count, else a copy. A slot below `counted`
/// may hold null while compiled code writes through it. See "Counting" in mir.rs.
///
/// # Safety
///
/// `record` must point to a live record that this caller gives one count of,
/// and `slot` must be below its `len`. `value` must fit the slot, and is borrowed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_record_set(
    mut record: *mut Record,
    slot: usize,
    value: i64,
) -> *mut Record {
    // SAFETY: the caller guarantees this. The new value is retained first,
    // since it may be the old one.
    unsafe {
        let (tag, len, counted) = ((*record).tag, (*record).len, (*record).counted);
        if slot < counted {
            retain_slot(value);
        }
        if (*record).head.count > 1 {
            let copy = lucid_record_new(tag, len, counted);
            let old = record_slots(record);
            std::ptr::copy_nonoverlapping(old.as_ptr(), copy.add(1).cast(), len);
            old[..counted].iter().for_each(|&held| retain_slot(held));
            (*record).head.count -= 1;
            record = copy;
        }
        let old = std::mem::replace(&mut record_slots(record)[slot], value);
        if slot < counted {
            release_slot(old);
        }
        record
    }
}

/// The keys of a process. Random keys stop a remote input from choosing
/// collisions, and a map's order never shows the hash.
fn hasher() -> &'static RandomState {
    static HASHER: OnceLock<RandomState> = OnceLock::new();
    HASHER.get_or_init(RandomState::new)
}

/// Adds an `int`-sized value to a hash in progress. A compound key is a chain
/// of these, so all of it depends on the keys of the process.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_hash_int(state: i64, value: i64) -> i64 {
    hasher().hash_one((state, value)) as i64
}

/// Adds a text to a hash in progress.
///
/// # Safety
///
/// `text` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_hash_text(state: i64, text: *const Text) -> i64 {
    // SAFETY: the caller guarantees this.
    hasher().hash_one((state, unsafe { bytes(text) })) as i64
}

/// Joins the hashes of a map's or set's entries. Equal maps may differ in
/// order, so this must not depend on it.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_hash_sum(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

/// # Safety
///
/// `map` must point to a live map, and `key` must fit its keys.
unsafe fn hash_key(map: *const Map, key: i64) -> u64 {
    // SAFETY: the caller guarantees this.
    let hash = unsafe {
        match (*map).keys {
            Some(tasks) => (tasks.hash)(key, 0) as u64,
            None if key_counted(map) => hasher().hash_one(bytes(key as *const Text)),
            None => hasher().hash_one(key),
        }
    };
    hash | LIVE_HASH
}

/// # Safety
///
/// If `counted` is true, `a` and `b` must hold live texts. A compound key
/// never reaches this.
unsafe fn same_key(counted: bool, a: i64, b: i64) -> bool {
    // SAFETY: the caller guarantees this.
    a == b || (counted && unsafe { bytes(a as *const Text) == bytes(b as *const Text) })
}

fn map_alloc(
    cap: usize,
    key_counted: bool,
    value_counted: bool,
    keys: Option<KeyTasks>,
) -> *mut Map {
    let head = Head { count: 1, kind: Kind::Map { key_counted, value_counted } };
    let map = object(map_size(cap), Map { head, len: 0, used: 0, cap, keys });
    // SAFETY: `object` made room for the table, and zero is `TABLE_EMPTY`.
    unsafe { table_start(map).write_bytes(0, 2 * cap) };
    map
}

/// The entries in use, which include removed ones.
///
/// # Safety
///
/// `map` must point to a live map, and the slice must not outlive it.
unsafe fn entries<'a>(map: *mut Map) -> &'a mut [Entry] {
    // SAFETY: the caller guarantees a live map, whose entries follow the header.
    unsafe {
        check_live(map.cast());
        std::slice::from_raw_parts_mut(map.add(1).cast(), (*map).used)
    }
}

/// # Safety
///
/// `map` must point to a live map.
unsafe fn table_start(map: *mut Map) -> *mut usize {
    // SAFETY: the caller guarantees a live map, and `map_size` made this room.
    unsafe { map.add(1).cast::<Entry>().add((*map).cap).cast() }
}

/// # Safety
///
/// `map` must point to a live map, and the slice must not outlive it.
unsafe fn table<'a>(map: *mut Map) -> &'a mut [usize] {
    // SAFETY: the caller guarantees a live map, and `map_alloc` filled the table.
    unsafe { std::slice::from_raw_parts_mut(table_start(map), 2 * (*map).cap) }
}

fn key_counted(map: *const Map) -> bool {
    // SAFETY: every caller holds a live map.
    matches!(unsafe { (*map).head.kind }, Kind::Map { key_counted: true, .. })
}

fn value_counted(map: *const Map) -> bool {
    // SAFETY: every caller holds a live map.
    matches!(unsafe { (*map).head.kind }, Kind::Map { value_counted: true, .. })
}

/// Finds a key's table position. The probe always ends at an empty position:
/// the table has `2 * cap` positions, and at most `cap` are ever filled.
///
/// This holds no reference into the map across a key task, since a key may
/// hold this same map, and the task then reads it too. A task that breaks the
/// rules of `equal` finds the wrong entry, and nothing worse.
///
/// # Safety
///
/// `map` must point to a live map, and `key` must fit its keys.
unsafe fn find(map: *mut Map, hash: u64, key: i64) -> Option<usize> {
    // SAFETY: the caller guarantees this, and a table index is below `used`.
    unsafe {
        check_live(map.cast());
        let (table, first) = (table_start(map), map.add(1).cast::<Entry>());
        let counted = key_counted(map);
        let mask = 2 * (*map).cap - 1;
        let mut at = hash as usize & mask;
        loop {
            match *table.add(at) {
                TABLE_EMPTY => return None,
                TABLE_REMOVED => {}
                index => {
                    let entry = *first.add(index - 1);
                    let same = entry.hash == hash
                        && match (*map).keys {
                            Some(tasks) => (tasks.equal)(entry.key, key) != 0,
                            None => same_key(counted, entry.key, key),
                        };
                    if same {
                        return Some(at);
                    }
                }
            }
            at = (at + 1) & mask;
        }
    }
}

/// Adds an entry that is known to be absent. The caller has made room.
///
/// # Safety
///
/// `map` must be live, with `used < cap`. This does not count the entry's values.
unsafe fn append(map: *mut Map, entry: Entry) {
    // SAFETY: the caller guarantees room, and a free position exists as in `find`.
    unsafe {
        let table = table(map);
        let mask = table.len() - 1;
        let mut at = entry.hash as usize & mask;
        while table[at] != TABLE_EMPTY && table[at] != TABLE_REMOVED {
            at = (at + 1) & mask;
        }
        let index = (*map).used;
        map.add(1).cast::<Entry>().add(index).write(entry);
        (*map).used = index + 1;
        (*map).len += 1;
        table[at] = index + 1;
    }
}

/// Returns a map that only this caller holds, with room for one more entry.
/// See `unique_list`. A rebuild also drops removed entries.
///
/// # Safety
///
/// `map` must point to a live map that this caller has a count of.
unsafe fn unique_map(map: *mut Map) -> *mut Map {
    // SAFETY: the caller guarantees a live map. As in `unique_list`, each path
    // keeps its one count, and the live entries fill less than the new `cap`.
    unsafe {
        let shared = (*map).head.count > 1;
        let full = (*map).used == (*map).cap;
        if !shared && !full {
            return map;
        }
        let cap = if full {
            (*map).len.saturating_mul(2).next_power_of_two().max(SMALLEST_MAP)
        } else {
            (*map).cap
        };
        let (keys, values) = (key_counted(map), value_counted(map));
        let copy = map_alloc(cap, keys, values, (*map).keys);
        for entry in entries(map).iter().filter(|entry| entry.hash != REMOVED) {
            append(copy, *entry);
            if shared && keys {
                retain_slot(entry.key);
            }
            if shared && values {
                retain_slot(entry.value);
            }
        }
        if shared {
            (*map).head.count -= 1;
        } else {
            free(map.cast(), map_size((*map).cap));
        }
        copy
    }
}

/// An empty map, or set. The flags say which slots hold counted values. A
/// compound key needs both tasks. Any other counted key is a text.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_map_new(
    key_counted: bool,
    value_counted: bool,
    hash: Option<HashTask>,
    equal: Option<EqualTask>,
) -> *mut Map {
    let keys = hash.zip(equal).map(|(hash, equal)| KeyTasks { hash, equal });
    map_alloc(SMALLEST_MAP, key_counted, value_counted, keys)
}

/// Sets a key's value, and returns the map that replaces `map`. A key that is
/// already there keeps its place in the order.
///
/// # Safety
///
/// `map` must point to a live map that this caller gives one count of. `key`
/// and `value` must fit its slots, and are borrowed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_map_set(map: *mut Map, key: i64, value: i64) -> *mut Map {
    // SAFETY: the caller guarantees all of this, and `unique_map` makes room.
    // The new value is retained first, since it may be the old one.
    unsafe {
        let (keys, values) = (key_counted(map), value_counted(map));
        if values {
            retain_slot(value);
        }
        let map = unique_map(map);
        let hash = hash_key(map, key);
        match find(map, hash, key) {
            Some(at) => {
                let entry = &mut entries(map)[table(map)[at] - 1];
                let old = std::mem::replace(&mut entry.value, value);
                if values {
                    release_slot(old);
                }
            }
            None => {
                if keys {
                    retain_slot(key);
                }
                append(map, Entry { hash, key, value });
            }
        }
        map
    }
}

/// Removes a key if it is there, and returns the map that replaces `map`.
///
/// # Safety
///
/// `map` must point to a live map that this caller gives one count of. `key`
/// must fit its keys, and is borrowed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_map_remove(map: *mut Map, key: i64) -> *mut Map {
    // SAFETY: the caller guarantees all of this. `unique_map` may rebuild, so
    // the position is found again after it.
    unsafe {
        let (keys, values) = (key_counted(map), value_counted(map));
        let hash = hash_key(map, key);
        if find(map, hash, key).is_none() {
            return map;
        }
        let map = unique_map(map);
        let Some(at) = find(map, hash, key) else { return map };
        let table = table(map);
        let entry = std::mem::replace(
            &mut entries(map)[table[at] - 1],
            Entry { hash: REMOVED, key: 0, value: 0 },
        );
        table[at] = TABLE_REMOVED;
        (*map).len -= 1;
        if keys {
            release_slot(entry.key);
        }
        if values {
            release_slot(entry.value);
        }
        map
    }
}

/// The address of a key's value, or null if the key is not there.
///
/// # Safety
///
/// `map` must point to a live map, and `key` must fit its keys.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_map_find(map: *mut Map, key: i64) -> *const i64 {
    // SAFETY: the caller guarantees this. The address lasts as long as the map.
    unsafe {
        let hash = hash_key(map, key);
        match find(map, hash, key) {
            Some(at) => &entries(map)[table(map)[at] - 1].value,
            None => std::ptr::null(),
        }
    }
}

/// The first live position at or after `position`, or -1 at the end.
///
/// # Safety
///
/// `map` must point to a live map, and `position` must not be negative.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_map_next(map: *mut Map, position: i64) -> i64 {
    // SAFETY: the caller guarantees a live map.
    let entries = unsafe { entries(map) };
    let mut rest = entries.iter().enumerate().skip(position as usize);
    rest.find(|(_, entry)| entry.hash != REMOVED).map_or(-1, |(at, _)| at as i64)
}

/// The key at a position from `lucid_map_next`.
///
/// # Safety
///
/// `map` must point to a live map, with a live entry at `position`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_map_key(map: *mut Map, position: i64) -> i64 {
    // SAFETY: the caller guarantees this.
    unsafe { entries(map)[position as usize].key }
}

/// The value at a position from `lucid_map_next` or `lucid_map_position`.
///
/// # Safety
///
/// `map` must point to a live map, with a live entry at `position`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_map_value(map: *mut Map, position: i64) -> i64 {
    // SAFETY: the caller guarantees this.
    unsafe { entries(map)[position as usize].value }
}

/// The position of a key, or -1 if the key is not there.
///
/// # Safety
///
/// `map` must point to a live map, and `key` must fit its keys.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_map_position(map: *mut Map, key: i64) -> i64 {
    // SAFETY: the caller guarantees this.
    unsafe {
        let hash = hash_key(map, key);
        find(map, hash, key).map_or(-1, |at| table(map)[at] as i64 - 1)
    }
}

/// A set of a list's values, in order, with each repeat dropped.
///
/// # Safety
///
/// `list` must point to a live list, of values that can be keys. See
/// `lucid_map_new` for the tasks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_set_from(
    list: *mut List,
    hash: Option<HashTask>,
    equal: Option<EqualTask>,
) -> *mut Map {
    // SAFETY: the caller guarantees a live list, and the new map has one owner.
    unsafe {
        let mut map = lucid_map_new(is_counted(list), false, hash, equal);
        for &slot in slots(list).iter() {
            map = lucid_map_set(map, slot, 0);
        }
        map
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_int_text(value: i64) -> *mut Text {
    text_from(value.to_string().as_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_uint_text(value: u64) -> *mut Text {
    text_from(value.to_string().as_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_float_text(value: f64) -> *mut Text {
    text_from(float_text(value).as_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn lucid_f32_text(value: f32) -> *mut Text {
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

#[unsafe(no_mangle)]
pub extern "C" fn lucid_print_uint(value: u64) {
    print_line(value);
}

/// Debug format uses the fewest digits that read back as the same float. It
/// writes `1e21`, but a Lucid float literal needs digits around a point.
fn float_text(value: impl std::fmt::Debug) -> String {
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
pub extern "C" fn lucid_print_f32(value: f32) {
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
    use std::sync::{Mutex, MutexGuard};

    /// One test at a time, since `LIVE` is global.
    fn serial() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(debug_assertions)]
    #[test]
    fn counts_reach_the_leak_check() {
        let _serial = serial();
        let start = LIVE.load(Ordering::Relaxed);
        let text = text_from("héllo".as_bytes());
        // SAFETY: `text` stays live until the last release.
        unsafe {
            assert_eq!(lucid_text_len(text), 5);
            lucid_retain(text.cast());
            lucid_release(text.cast());
            assert_eq!(LIVE.load(Ordering::Relaxed), start + 1);
            lucid_release(text.cast());
            lucid_release(std::ptr::null_mut());
        }
        assert_eq!(LIVE.load(Ordering::Relaxed), start);
    }

    #[test]
    fn slices_count_characters_and_reject_bad_ranges() {
        let _serial = serial();
        let text = text_from("a€😀".as_bytes());
        // SAFETY: `text` stays live, and each slice is read before its release.
        unsafe {
            for (start, end, expected) in [(0, 3, "a€😀"), (1, 2, "€"), (2, 3, "😀"), (3, 3, "")]
            {
                let slice = lucid_text_slice(text, start, end);
                assert_eq!(bytes(slice), expected.as_bytes());
                lucid_release(slice.cast());
            }
            for (start, end) in [(-1, 1), (2, 1), (0, 4), (4, 4), (0, i64::MAX), (i64::MIN, 0)] {
                assert!(lucid_text_slice(text, start, end).is_null());
            }
            lucid_release(text.cast());
        }
    }

    #[test]
    fn characters_are_read_by_byte_offset() {
        let _serial = serial();
        let text = text_from("a€😀".as_bytes());
        // SAFETY: `text` stays live, and each offset starts a character.
        unsafe {
            assert_eq!(lucid_text_byte_len(text), 8);
            for (offset, expected) in [(0, "a"), (1, "€"), (4, "😀")] {
                let character = lucid_text_char_at(text, offset);
                assert_eq!(bytes(character), expected.as_bytes());
                lucid_release(character.cast());
            }
            lucid_release(text.cast());
        }
    }

    #[test]
    fn a_shared_list_copies_once_and_keeps_its_elements() {
        let _serial = serial();
        // SAFETY: each pointer is live for as long as this test holds a count.
        unsafe {
            let mut list = lucid_list_new(0, true);
            let text = text_from(b"x");
            for _ in 0..100 {
                list = lucid_list_push(list, text as i64);
            }
            lucid_retain(list.cast());
            let copy = lucid_list_set(list, 0, slots(list)[1]);
            assert_ne!(copy, list);
            assert_eq!(((*list).len, (*copy).len), (100, 100));
            assert!(lucid_list_set(copy, 100, 0).is_null());
            assert!(lucid_list_swap(copy, -1, 0).is_null());
            lucid_release(copy.cast());
            lucid_release(list.cast());
            assert_eq!((*text).head.count, 1);
            lucid_release(text.cast());
        }
    }

    #[test]
    fn pop_releases_the_value_and_keeps_a_shared_list() {
        let _serial = serial();
        // SAFETY: each pointer is live for as long as this test holds a count.
        unsafe {
            let text = text_from(b"x");
            let list = lucid_list_push(lucid_list_new(0, true), text as i64);
            lucid_retain(list.cast());
            let popped = lucid_list_pop(list);
            assert_ne!(popped, list);
            assert_eq!(((*list).len, (*popped).len, (*text).head.count), (1, 0, 2));
            lucid_release(list.cast());
            let list = lucid_list_push(popped, text as i64);
            let popped = lucid_list_pop(list);
            assert_eq!((popped, (*text).head.count), (list, 1));
            lucid_release(popped.cast());
            lucid_release(text.cast());
        }
    }

    #[test]
    fn a_map_keeps_order_through_growth_and_removal() {
        let _serial = serial();
        // SAFETY: each pointer is live for as long as this test holds a count.
        unsafe {
            let mut map = lucid_map_new(false, false, None, None);
            for key in 0..1000 {
                map = lucid_map_set(map, key, key * 2);
            }
            for key in (0..1000).step_by(2) {
                map = lucid_map_remove(map, key);
                map = lucid_map_remove(map, key);
            }
            for key in 0..1000 {
                map = lucid_map_set(map, key + 1000, 1);
                map = lucid_map_remove(map, key + 1000);
            }
            assert_eq!((*map).len, 500);
            let mut keys = Vec::new();
            let mut at = lucid_map_next(map, 0);
            while at >= 0 {
                keys.push(lucid_map_key(map, at));
                at = lucid_map_next(map, at + 1);
            }
            assert_eq!(keys, (0..1000).filter(|key| key % 2 == 1).collect::<Vec<_>>());
            assert_eq!(*lucid_map_find(map, 7), 14);
            assert!(lucid_map_find(map, 8).is_null());
            lucid_release(map.cast());
        }
    }

    #[test]
    fn text_keys_compare_by_content() {
        let _serial = serial();
        // SAFETY: each pointer is live for as long as this test holds a count.
        unsafe {
            let (a, b) = (text_from(b"key"), text_from(b"key"));
            let map = lucid_map_set(lucid_map_new(true, false, None, None), a as i64, 1);
            let map = lucid_map_set(map, b as i64, 2);
            assert_eq!((*map).len, 1);
            assert_eq!(*lucid_map_find(map, b as i64), 2);
            let map = lucid_map_remove(map, b as i64);
            assert_eq!(((*map).len, (*a).head.count), (0, 1));
            lucid_release(map.cast());
            lucid_release(a.cast());
            lucid_release(b.cast());
        }
    }
}
