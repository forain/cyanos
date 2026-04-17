//! C string functions: strlen, strcpy, strcmp, strcat, strdup, strtol, …

use crate::mem::{malloc, memcpy, memset};

use crate::io::{size_t, c_int};

/// Return the length of NUL-terminated string `s`.
#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const u8) -> size_t {
    let mut n = 0;
    while *s.add(n) != 0 { n += 1; }
    n
}

/// Like `strlen` but stops at `maxlen`.
#[no_mangle]
pub unsafe extern "C" fn strnlen(s: *const u8, maxlen: size_t) -> size_t {
    let mut n = 0;
    while n < maxlen && *s.add(n) != 0 { n += 1; }
    n
}

/// Copy NUL-terminated `src` into `dst` (must not overlap, dst must be large enough).
#[no_mangle]
pub unsafe extern "C" fn strcpy(dst: *mut u8, src: *const u8) -> *mut u8 {
    let mut i = 0;
    loop {
        let c = *src.add(i);
        *dst.add(i) = c;
        if c == 0 { break; }
        i += 1;
    }
    dst
}

/// Copy at most `n` bytes of `src` into `dst`; NUL-pad if shorter.
#[no_mangle]
pub unsafe extern "C" fn strncpy(dst: *mut u8, src: *const u8, n: size_t) -> *mut u8 {
    let mut i = 0;
    while i < n {
        let c = *src.add(i);
        *dst.add(i) = c;
        i += 1;
        if c == 0 {
            memset(dst.add(i), 0, n - i);
            return dst;
        }
    }
    dst
}

/// Append `src` to `dst`.
#[no_mangle]
pub unsafe extern "C" fn strcat(dst: *mut u8, src: *const u8) -> *mut u8 {
    let dlen = strlen(dst as *const u8);
    strcpy(dst.add(dlen), src);
    dst
}

/// Append at most `n` bytes of `src` to `dst`.
#[no_mangle]
pub unsafe extern "C" fn strncat(dst: *mut u8, src: *const u8, n: size_t) -> *mut u8 {
    let dlen = strlen(dst as *const u8);
    let slen = strnlen(src, n);
    memcpy(dst.add(dlen), src, slen);
    *dst.add(dlen + slen) = 0;
    dst
}

/// Compare two NUL-terminated strings. Returns <0, 0, or >0.
#[no_mangle]
pub unsafe extern "C" fn strcmp(a: *const u8, b: *const u8) -> c_int {
    let mut i = 0;
    loop {
        let ca = *a.add(i);
        let cb = *b.add(i);
        if ca != cb { return ca as c_int - cb as c_int; }
        if ca == 0  { return 0; }
        i += 1;
    }
}

/// Like `strcmp` but limited to `n` characters.
#[no_mangle]
pub unsafe extern "C" fn strncmp(a: *const u8, b: *const u8, n: size_t) -> c_int {
    for i in 0..n {
        let ca = *a.add(i);
        let cb = *b.add(i);
        if ca != cb { return ca as c_int - cb as c_int; }
        if ca == 0  { return 0; }
    }
    0
}

/// Duplicate a string on the heap.
#[no_mangle]
pub unsafe extern "C" fn strdup(s: *const u8) -> *mut u8 {
    let len = strlen(s) + 1;
    let p = malloc(len);
    if !p.is_null() { memcpy(p, s, len); }
    p
}

/// Find the first occurrence of byte `c` in `s`.
#[no_mangle]
pub unsafe extern "C" fn strchr(s: *const u8, c: c_int) -> *mut u8 {
    let mut i = 0;
    loop {
        let b = *s.add(i);
        if b == c as u8 { return s.add(i) as *mut u8; }
        if b == 0 { return core::ptr::null_mut(); }
        i += 1;
    }
}

/// Find the last occurrence of byte `c` in `s`.
#[no_mangle]
pub unsafe extern "C" fn strrchr(s: *const u8, c: c_int) -> *mut u8 {
    let mut last: *mut u8 = core::ptr::null_mut();
    let mut i = 0;
    loop {
        let b = *s.add(i);
        if b == c as u8 { last = s.add(i) as *mut u8; }
        if b == 0 { return last; }
        i += 1;
    }
}

/// Find the first occurrence of `needle` in `haystack`.
#[no_mangle]
pub unsafe extern "C" fn strstr(haystack: *const u8, needle: *const u8) -> *mut u8 {
    let nlen = strlen(needle);
    if nlen == 0 { return haystack as *mut u8; }
    let hlen = strlen(haystack);
    if nlen > hlen { return core::ptr::null_mut(); }
    for i in 0..=(hlen - nlen) {
        if strncmp(haystack.add(i), needle, nlen) == 0 {
            return haystack.add(i) as *mut u8;
        }
    }
    core::ptr::null_mut()
}

/// Convert string to long integer.
#[no_mangle]
pub unsafe extern "C" fn strtol(s: *const u8, endptr: *mut *mut u8, base: c_int) -> i64 {
    let mut i = 0usize;
    // Skip whitespace.
    while *s.add(i) == b' ' || *s.add(i) == b'\t' { i += 1; }
    let neg = if *s.add(i) == b'-' { i += 1; true } else { if *s.add(i) == b'+' { i += 1; } false };
    let radix = if base == 0 {
        if *s.add(i) == b'0' {
            i += 1;
            if *s.add(i) == b'x' || *s.add(i) == b'X' { i += 1; 16 } else { 8 }
        } else { 10 }
    } else { base as u64 };
    let mut n: i64 = 0;
    loop {
        let c = *s.add(i);
        let digit = match c {
            b'0'..=b'9' => (c - b'0') as u64,
            b'a'..=b'f' => (c - b'a' + 10) as u64,
            b'A'..=b'F' => (c - b'A' + 10) as u64,
            _ => break,
        };
        if digit >= radix { break; }
        n = n.wrapping_mul(radix as i64).wrapping_add(digit as i64);
        i += 1;
    }
    if !endptr.is_null() { *endptr = s.add(i) as *mut u8; }
    if neg { -n } else { n }
}

/// Convert string to unsigned long.
#[no_mangle]
pub unsafe extern "C" fn strtoul(s: *const u8, endptr: *mut *mut u8, base: c_int) -> u64 {
    strtol(s, endptr, base) as u64
}

/// Convert integer to string using `atoi` semantics.
#[no_mangle]
pub unsafe extern "C" fn atoi(s: *const u8) -> c_int {
    strtol(s, core::ptr::null_mut(), 10) as c_int
}
