//! The terminal frontend.
//!
//! Renders on the terminal named by `OPTION ttyname`. It must **not** use
//! stdin/stdout: those are the Assuan pipes, and writing to them corrupts the
//! protocol stream.

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;

use crate::assuan::error::AssuanError;
use crate::assuan::state::State;
use crate::backend::term::{self, RawMode, SignalGuard};
use crate::backend::{Backend, ConfirmOptions, ConfirmOutcome, PinResponse};
use crate::secret::Secret;

const FALLBACK_TTY: &str = "/dev/tty";

pub struct TtyBackend {
    /// Overridden by `OPTION invisible-char`.
    mask: char,
}

impl TtyBackend {
    pub fn new() -> Self {
        Self::with_mask('*')
    }

    pub fn with_mask(mask: char) -> Self {
        Self { mask }
    }

    pub fn device_path(state: &State) -> &str {
        state
            .options
            .ttyname
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(FALLBACK_TTY)
    }

    /// Opening is the only reliable test: the path may exist but belong to
    /// another session, or not be a terminal at all.
    pub fn is_available(state: &State) -> bool {
        let path = Self::device_path(state);
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => unsafe { libc::isatty(file.as_raw_fd()) == 1 },
            Err(_) => false,
        }
    }

    fn mask_char(&self, state: &State) -> char {
        state
            .options
            .invisible_char
            .as_deref()
            .and_then(|s| s.chars().next())
            .unwrap_or(self.mask)
    }

    fn open(state: &State) -> Result<std::fs::File, AssuanError> {
        let path = Self::device_path(state);
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|_| AssuanError::no_input())
    }
}

impl Default for TtyBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks what was drawn so it can be erased, leaving the terminal as found.
struct Screen<'a> {
    out: &'a std::fs::File,
    lines_drawn: usize,
}

impl<'a> Screen<'a> {
    fn new(out: &'a std::fs::File) -> Self {
        Self {
            out,
            lines_drawn: 0,
        }
    }

    fn write_line(&mut self, text: &str) -> io::Result<()> {
        // Clear to end of line so a redraw leaves no fragment of a longer one.
        write!(self.out, "\r\x1b[K{text}\r\n")?;
        self.lines_drawn += 1;
        Ok(())
    }

    /// No trailing newline: the cursor stays put so the next redraw overwrites.
    fn write_prompt(&mut self, text: &str) -> io::Result<()> {
        write!(self.out, "\r\x1b[K{text}")?;
        self.out.flush()
    }

    fn erase(&mut self) -> io::Result<()> {
        write!(self.out, "\r\x1b[K")?;
        for _ in 0..self.lines_drawn {
            write!(self.out, "\x1b[A\x1b[K")?;
        }
        self.lines_drawn = 0;
        self.out.flush()
    }
}

fn draw_header(screen: &mut Screen, state: &State) -> io::Result<()> {
    if let Some(title) = &state.request.title {
        screen.write_line(&format!("\x1b[1m{title}\x1b[0m"))?;
    }
    if let Some(desc) = &state.request.description {
        for line in desc.lines() {
            screen.write_line(line)?;
        }
    }
    if let Some(err) = &state.request.error {
        screen.write_line(&format!("\x1b[1;31m{err}\x1b[0m"))?;
    }
    Ok(())
}

/// Milliseconds remaining, or -1 to block indefinitely.
fn poll_timeout(state: &State, started: std::time::Instant) -> i32 {
    if state.options.timeout_secs == 0 {
        return -1;
    }
    let budget = std::time::Duration::from_secs(state.options.timeout_secs as u64);
    match budget.checked_sub(started.elapsed()) {
        Some(remaining) => remaining.as_millis().min(i32::MAX as u128) as i32,
        None => 0,
    }
}

impl Backend for TtyBackend {
    fn get_pin(&mut self, state: &State) -> Result<PinResponse, AssuanError> {
        let tty = Self::open(state)?;
        let fd = tty.as_raw_fd();

        let _signals = SignalGuard::install();
        let _raw = RawMode::enable(fd).map_err(|_| AssuanError::no_input())?;

        let mut screen = Screen::new(&tty);
        let mut secret = Secret::new();
        let mask = self.mask_char(state);
        let prompt = state.effective_prompt().to_string();

        draw_header(&mut screen, state).map_err(|_| AssuanError::no_input())?;

        let started = std::time::Instant::now();
        let mut chars = 0usize;
        let mut reader = &tty;

        let outcome = loop {
            screen
                .write_prompt(&format!(
                    "{prompt} {}",
                    std::iter::repeat_n(mask, chars).collect::<String>()
                ))
                .map_err(|_| AssuanError::no_input())?;

            if term::interrupted() {
                break Err(AssuanError::canceled());
            }

            match term::wait_readable(fd, poll_timeout(state, started)) {
                Ok(true) => {}
                Ok(false) => break Err(AssuanError::timeout()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break Err(AssuanError::no_input()),
            }

            let mut byte = [0u8; 1];
            match reader.read(&mut byte) {
                Ok(0) => break Err(AssuanError::canceled()), // EOF
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break Err(AssuanError::no_input()),
            }

            match byte[0] {
                b'\r' | b'\n' => break Ok(()),
                // Ctrl-C or Escape.
                0x03 | 0x1b => break Err(AssuanError::canceled()),
                // Ctrl-D cancels only on an empty buffer, as in a shell.
                0x04 if secret.is_empty() => break Err(AssuanError::canceled()),
                0x04 => {}
                0x7f | 0x08 => {
                    secret.pop_char();
                    chars = chars.saturating_sub(1);
                }
                // Ctrl-U clears the entry.
                0x15 => {
                    secret.clear();
                    chars = 0;
                }
                lead => {
                    // Gather the whole character so it counts as one mask
                    // position and stays valid UTF-8.
                    let width = term::utf8_len(lead);
                    let mut buf = [0u8; 4];
                    buf[0] = lead;
                    let mut filled = 1;
                    while filled < width {
                        match reader.read(&mut buf[filled..filled + 1]) {
                            Ok(0) => break,
                            Ok(_) => filled += 1,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                            Err(_) => break,
                        }
                    }
                    if secret.push_bytes(&buf[..filled]).is_ok() {
                        chars += 1;
                    }
                }
            }
        };

        let _ = screen.erase();
        outcome.map(|()| secret.into())
    }

    fn confirm(
        &mut self,
        state: &State,
        options: ConfirmOptions,
    ) -> Result<ConfirmOutcome, AssuanError> {
        let tty = Self::open(state)?;
        let fd = tty.as_raw_fd();

        let _signals = SignalGuard::install();
        let _raw = RawMode::enable(fd).map_err(|_| AssuanError::no_input())?;

        let mut screen = Screen::new(&tty);
        draw_header(&mut screen, state).map_err(|_| AssuanError::no_input())?;

        let hint = if options.one_button {
            format!("[{}]", strip_accelerator(state.effective_ok()))
        } else {
            format!(
                "[{}/{}]",
                strip_accelerator(state.effective_ok()),
                strip_accelerator(state.effective_cancel())
            )
        };

        screen
            .write_prompt(&format!("{hint} "))
            .map_err(|_| AssuanError::no_input())?;

        let started = std::time::Instant::now();
        let mut reader = &tty;

        let outcome = loop {
            if term::interrupted() {
                break Ok(ConfirmOutcome::Cancelled);
            }

            match term::wait_readable(fd, poll_timeout(state, started)) {
                Ok(true) => {}
                Ok(false) => break Err(AssuanError::timeout()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break Err(AssuanError::no_input()),
            }

            let mut byte = [0u8; 1];
            match reader.read(&mut byte) {
                Ok(0) => break Ok(ConfirmOutcome::Cancelled),
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break Err(AssuanError::no_input()),
            }

            // One-button prompts are informational: any key dismisses them.
            if options.one_button {
                break Ok(ConfirmOutcome::Confirmed);
            }

            match byte[0].to_ascii_lowercase() {
                b'y' | b'\r' | b'\n' => break Ok(ConfirmOutcome::Confirmed),
                b'n' => break Ok(ConfirmOutcome::Declined),
                0x03 | 0x1b => break Ok(ConfirmOutcome::Cancelled),
                _ => continue,
            }
        };

        let _ = screen.erase();
        outcome
    }

    fn flavor(&self) -> &'static str {
        "dank:tty"
    }
}

/// gpg-agent marks accelerators with `_` (`_OK`); meaningless on a terminal.
fn strip_accelerator(label: &str) -> String {
    label.replace('_', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_path_prefers_the_agent_supplied_tty() {
        let mut state = State::new();
        assert_eq!(TtyBackend::device_path(&state), "/dev/tty");

        state.options.ttyname = Some("/dev/pts/7".into());
        assert_eq!(TtyBackend::device_path(&state), "/dev/pts/7");
    }

    #[test]
    fn empty_ttyname_falls_back() {
        let mut state = State::new();
        state.options.ttyname = Some(String::new());
        assert_eq!(TtyBackend::device_path(&state), "/dev/tty");
    }

    #[test]
    fn accelerators_are_stripped_for_the_terminal() {
        assert_eq!(strip_accelerator("_OK"), "OK");
        assert_eq!(strip_accelerator("_Cancel"), "Cancel");
        assert_eq!(strip_accelerator("Yes"), "Yes");
    }

    #[test]
    fn mask_char_honours_the_invisible_char_option() {
        let backend = TtyBackend::new();
        let mut state = State::new();
        assert_eq!(backend.mask_char(&state), '*');

        state.options.invisible_char = Some("•".into());
        assert_eq!(backend.mask_char(&state), '•');
    }

    #[test]
    fn zero_timeout_means_block_forever() {
        let mut state = State::new();
        state.options.timeout_secs = 0;
        assert_eq!(poll_timeout(&state, std::time::Instant::now()), -1);
    }

    #[test]
    fn timeout_counts_down() {
        let mut state = State::new();
        state.options.timeout_secs = 60;
        let remaining = poll_timeout(&state, std::time::Instant::now());
        assert!(remaining > 59_000 && remaining <= 60_000, "{remaining}");
    }

    #[test]
    fn expired_timeout_polls_without_blocking() {
        let mut state = State::new();
        state.options.timeout_secs = 1;
        let started = std::time::Instant::now() - std::time::Duration::from_secs(5);
        assert_eq!(poll_timeout(&state, started), 0);
    }

    #[test]
    fn a_nonexistent_tty_is_not_available() {
        let mut state = State::new();
        state.options.ttyname = Some("/dev/definitely-not-a-tty-12345".into());
        assert!(!TtyBackend::is_available(&state));
    }

    #[test]
    fn a_regular_file_is_not_a_tty() {
        let mut state = State::new();
        state.options.ttyname = Some("/dev/null".into());
        assert!(!TtyBackend::is_available(&state));
    }
}
