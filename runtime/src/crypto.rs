//! `std.crypto`, by decision 193: digests, HMAC, and random bytes from `ring`,
//! constant-time compare from `subtle`, and passwords from `argon2`. Native
//! targets only. Every task borrows its arguments and gives a new value.

use super::{List, Text, byte_list, list_bytes, system_text, text_from};
use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use ring::rand::{SecureRandom, SystemRandom};
use ring::{digest, hmac};
use subtle::ConstantTimeEq;

/// Argon2id with OWASP's first setting: m=19456 KiB, t=2, p=1.
fn argon2() -> Argon2<'static> {
    let params = Params::new(19456, 2, 1, None).expect("a valid Argon2 setting");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn random(out: &mut [u8]) {
    SystemRandom::new().fill(out).expect("the system gives random bytes");
}

/// The SHA-256 digest of `data`, 32 bytes.
///
/// # Safety
///
/// `data` must point to a live `list[byte]`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_crypto_sha256(data: *const List) -> *mut List {
    // SAFETY: the caller guarantees a live list.
    byte_list(digest::digest(&digest::SHA256, unsafe { list_bytes(data) }).as_ref())
}

/// The SHA-512 digest of `data`, 64 bytes.
///
/// # Safety
///
/// `data` must point to a live `list[byte]`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_crypto_sha512(data: *const List) -> *mut List {
    // SAFETY: the caller guarantees a live list.
    byte_list(digest::digest(&digest::SHA512, unsafe { list_bytes(data) }).as_ref())
}

/// The HMAC-SHA256 tag of `data` under `key`, 32 bytes.
///
/// # Safety
///
/// `key` and `data` must point to live `list[byte]` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_crypto_hmac_sha256(
    key: *const List,
    data: *const List,
) -> *mut List {
    // SAFETY: the caller guarantees both lists.
    let (key, data) = unsafe { (list_bytes(key), list_bytes(data)) };
    byte_list(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data).as_ref())
}

/// True if `tag` is the HMAC-SHA256 tag of `data` under `key`, checked in
/// constant time.
///
/// # Safety
///
/// `key`, `data`, and `tag` must point to live `list[byte]` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_crypto_hmac_check(
    key: *const List,
    data: *const List,
    tag: *const List,
) -> bool {
    // SAFETY: the caller guarantees all three lists.
    let (key, data, tag) = unsafe { (list_bytes(key), list_bytes(data), list_bytes(tag)) };
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, key), data, tag).is_ok()
}

/// True if `a` and `b` hold the same bytes. The time depends only on the
/// lengths.
///
/// # Safety
///
/// `a` and `b` must point to live `list[byte]` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_crypto_same(a: *const List, b: *const List) -> bool {
    // SAFETY: the caller guarantees both lists.
    unsafe { list_bytes(a).ct_eq(list_bytes(b)) }.into()
}

/// `count` bytes from the system's cryptographic source. `std.crypto` checks
/// that `count` is not negative.
#[unsafe(no_mangle)]
pub extern "C" fn lucid_crypto_random(count: i64) -> *mut List {
    let mut out = vec![0; usize::try_from(count).unwrap_or(0)];
    random(&mut out);
    byte_list(&out)
}

/// An Argon2id hash of `password` with a new random salt, as a PHC string.
///
/// # Safety
///
/// `password` must point to a live text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_crypto_hash_password(password: *const Text) -> *mut Text {
    let mut salt = [0; 16];
    random(&mut salt);
    // SAFETY: the caller guarantees a live text.
    let password = unsafe { system_text(password) };
    let hash = argon2()
        .hash_password_with_salt(password.as_bytes(), &salt)
        .expect("Argon2 hashes with a valid setting");
    text_from(hash.to_string().as_bytes())
}

/// True if `hash`, a PHC string, is a hash of `password`. The setting comes
/// from `hash`, so hashes made at an older setting still check. A `hash` that
/// is not a PHC string of Argon2 gives false.
///
/// # Safety
///
/// `password` and `hash` must point to live texts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_crypto_check_password(
    password: *const Text,
    hash: *const Text,
) -> bool {
    // SAFETY: the caller guarantees both texts.
    let (password, hash) = unsafe { (system_text(password), system_text(hash)) };
    argon2().verify_password(password.as_bytes(), hash).is_ok()
}

/// `data` as lowercase hex, two characters a byte.
///
/// # Safety
///
/// `data` must point to a live `list[byte]`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lucid_crypto_hex(data: *const List) -> *mut Text {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    // SAFETY: the caller guarantees a live list.
    let data = unsafe { list_bytes(data) };
    let out: Vec<u8> = data
        .iter()
        .flat_map(|&b| [DIGITS[usize::from(b >> 4)], DIGITS[usize::from(b & 15)]])
        .collect();
    text_from(&out)
}
