//! Raw-mode handling for a terminal device.
//!
//! Leaving a terminal in raw mode makes the user's shell unusable, and a
//! default `SIGINT` would kill the process without running `Drop`. Hence the
//! guards below.

use std::io;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

pub fn clear_interrupt() {
    INTERRUPTED.store(false, Ordering::SeqCst);
}

extern "C" fn handle_signal(_sig: libc::c_int) {
    // Async-signal-safe: only touches an atomic.
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Installs interrupt handlers, restoring the previous ones on drop.
pub struct SignalGuard {
    previous: Vec<(libc::c_int, libc::sigaction)>,
}

impl SignalGuard {
    pub fn install() -> Self {
        let mut previous = Vec::new();
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = handle_signal as extern "C" fn(libc::c_int) as usize;
                libc::sigemptyset(&mut action.sa_mask);
                // No SA_RESTART: read() must fail with EINTR so the input
                // loop can observe the flag and unwind cleanly.
                action.sa_flags = 0;

                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(sig, &action, &mut old) == 0 {
                    previous.push((sig, old));
                }
            }
        }
        clear_interrupt();
        Self { previous }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        for (sig, old) in &self.previous {
            unsafe {
                libc::sigaction(*sig, old, std::ptr::null_mut());
            }
        }
    }
}

/// Restores the original terminal settings on drop.
pub struct RawMode {
    fd: RawFd,
    original: libc::termios,
}

impl RawMode {
    pub fn enable(fd: RawFd) -> io::Result<Self> {
        let original = unsafe {
            let mut termios: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut termios) != 0 {
                return Err(io::Error::last_os_error());
            }
            termios
        };

        let mut raw = original;

        // ISIG off is deliberate: ^C arrives as a byte so cancelling goes
        // through our own path and the guards still run.
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHONL | libc::ISIG | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::INLCR | libc::IGNCR | libc::BRKINT);

        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        unsafe {
            if libc::tcsetattr(fd, libc::TCSAFLUSH, &raw) != 0 {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(Self { fd, original })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.original);
        }
    }
}

/// `Ok(false)` on timeout. `EINTR` surfaces as an error so the caller can
/// check the interrupt flag.
pub fn wait_readable(fd: RawFd, timeout_ms: i32) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    let rc = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
    match rc {
        -1 => Err(io::Error::last_os_error()),
        0 => Ok(false),
        _ => Ok(true),
    }
}

/// Byte length of a UTF-8 sequence from its leading byte, so a multi-byte
/// keystroke renders as one mask character rather than several.
pub fn utf8_len(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead >> 5 == 0b110 {
        2
    } else if lead >> 4 == 0b1110 {
        3
    } else if lead >> 3 == 0b11110 {
        4
    } else {
        // Continuation or invalid byte: treat as one unit rather than stalling
        // on bytes that will never arrive.
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_len_covers_all_lead_bytes() {
        assert_eq!(utf8_len(b'a'), 1);
        assert_eq!(utf8_len(0xC3), 2); // ü
        assert_eq!(utf8_len(0xE2), 3); // €
        assert_eq!(utf8_len(0xF0), 4); // emoji
        assert_eq!(utf8_len(0x80), 1); // stray continuation
    }

    #[test]
    fn interrupt_flag_round_trips() {
        clear_interrupt();
        assert!(!interrupted());
        handle_signal(libc::SIGINT);
        assert!(interrupted());
        clear_interrupt();
        assert!(!interrupted());
    }
}
