//! A passphrase buffer locked into RAM and wiped on drop.
//!
//! Capacity is fixed and the buffer never grows: growing a `Vec` reallocates,
//! and the old allocation would be freed before it could be wiped, leaking the
//! passphrase onto the heap.

use zeroize::Zeroize;

/// Upstream pinentry uses 2 KiB; double it for long diceware passphrases.
pub const DEFAULT_CAPACITY: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub enum SecretError {
    Full,
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::Full => write!(f, "passphrase exceeds buffer capacity"),
        }
    }
}

impl std::error::Error for SecretError {}

pub struct Secret {
    buf: Vec<u8>,
    /// `mlock` can fail under a low `RLIMIT_MEMLOCK`; that degrades secrecy
    /// but must not break the prompt.
    locked: bool,
}

impl Secret {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        // Force the allocation up front so the pointer is stable and lockable.
        let mut buf = vec![0u8; capacity];
        buf.clear();

        let locked = mlock(buf.as_ptr(), capacity);
        Self { buf, locked }
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn push_str(&mut self, s: &str) -> Result<(), SecretError> {
        self.push_bytes(s.as_bytes())
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), SecretError> {
        if self.buf.len() + bytes.len() > self.buf.capacity() {
            return Err(SecretError::Full);
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    /// Removes a whole UTF-8 character, not a byte.
    pub fn pop_char(&mut self) {
        while let Some(&b) = self.buf.last() {
            self.buf.pop();
            if b & 0b1100_0000 != 0b1000_0000 {
                break;
            }
        }
    }

    pub fn clear(&mut self) {
        self.buf.zeroize();
        self.buf.clear();
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let ptr = self.buf.as_ptr();
        let capacity = self.buf.capacity();

        // Whole capacity, not just len: earlier longer contents sit past it.
        unsafe {
            std::ptr::write_bytes(ptr as *mut u8, 0, capacity);
        }

        if self.locked {
            munlock(ptr, capacity);
        }
    }
}

/// Opaque: never print a passphrase, even by accident.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secret")
            .field("len", &self.buf.len())
            .field("locked", &self.locked)
            .finish_non_exhaustive()
    }
}

fn mlock(ptr: *const u8, len: usize) -> bool {
    if len == 0 {
        return false;
    }
    unsafe { libc::mlock(ptr as *const libc::c_void, len) == 0 }
}

fn munlock(ptr: *const u8, len: usize) {
    if len == 0 {
        return;
    }
    unsafe {
        libc::munlock(ptr as *const libc::c_void, len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_bytes() {
        let mut s = Secret::new();
        s.push_str("hunter").unwrap();
        s.push_str("2").unwrap();
        assert_eq!(s.as_bytes(), b"hunter2");
        assert_eq!(s.len(), 7);
    }

    #[test]
    fn refuses_to_grow_past_capacity() {
        let mut s = Secret::with_capacity(4);
        assert!(s.push_str("abcd").is_ok());
        assert_eq!(s.push_str("e"), Err(SecretError::Full));
        // The rejected write must not have partially landed.
        assert_eq!(s.as_bytes(), b"abcd");
    }

    #[test]
    fn capacity_is_stable_so_the_locked_region_stays_valid() {
        let mut s = Secret::with_capacity(64);
        let before = s.as_bytes().as_ptr();
        s.push_str("some passphrase").unwrap();
        assert_eq!(s.as_bytes().as_ptr(), before);
        assert_eq!(s.capacity(), 64);
    }

    #[test]
    fn pop_char_removes_whole_multibyte_characters() {
        let mut s = Secret::new();
        s.push_str("aé").unwrap();
        assert_eq!(s.len(), 3);
        s.pop_char();
        assert_eq!(s.as_bytes(), b"a");
        s.pop_char();
        assert!(s.is_empty());
        // Popping an empty buffer must not panic.
        s.pop_char();
        assert!(s.is_empty());
    }

    #[test]
    fn clear_wipes_contents() {
        let mut s = Secret::new();
        s.push_str("hunter2").unwrap();
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn debug_never_reveals_the_passphrase() {
        let mut s = Secret::new();
        s.push_str("hunter2").unwrap();
        let rendered = format!("{s:?}");
        assert!(!rendered.contains("hunter2"), "leaked: {rendered}");
        assert!(rendered.contains("len"));
    }
}
