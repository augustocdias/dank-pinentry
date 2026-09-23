//! The interface every frontend implements. Frontends never touch the wire
//! format; they receive the accumulated [`State`] and return a decision.

pub mod auto;
pub mod dms;
pub mod term;
pub mod tty;

pub use auto::AutoBackend;
pub use dms::DmsBackend;
pub use tty::TtyBackend;

use crate::assuan::error::AssuanError;
use crate::assuan::state::State;
use crate::secret::Secret;

/// A passphrase, and whether the frontend already had it typed twice.
#[derive(Debug)]
pub struct PinResponse {
    pub secret: Secret,
    /// Reported as `S PIN_REPEATED`, so gpg-agent skips asking again.
    pub repeated: bool,
}

impl From<Secret> for PinResponse {
    fn from(secret: Secret) -> Self {
        Self {
            secret,
            repeated: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Confirmed,
    /// Explicitly declined; maps to `NOT_CONFIRMED`, not `CANCELED`.
    Declined,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConfirmOptions {
    /// `--one-button`: informational only; there is nothing to decline.
    pub one_button: bool,
    /// `--focus=ok` / `--focus=cancel`, as 1, 0 or -1.
    pub focus: i8,
}

impl ConfirmOptions {
    pub fn parse(line: &str) -> Self {
        Self {
            one_button: line.contains("--one-button"),
            focus: if line.contains("--focus=ok") {
                1
            } else if line.contains("--focus=cancel") {
                -1
            } else {
                0
            },
        }
    }
}

pub trait Backend {
    fn get_pin(&mut self, state: &State) -> Result<PinResponse, AssuanError>;

    fn confirm(
        &mut self,
        state: &State,
        options: ConfirmOptions,
    ) -> Result<ConfirmOutcome, AssuanError>;

    /// Reported by `GETINFO flavor`.
    fn flavor(&self) -> &'static str;
}

/// Lets tests hand out a borrowed backend and inspect it afterwards. Must live
/// here rather than in the test crate because of the orphan rule.
impl<B: Backend + ?Sized> Backend for &mut B {
    fn get_pin(&mut self, state: &State) -> Result<PinResponse, AssuanError> {
        (**self).get_pin(state)
    }

    fn confirm(
        &mut self,
        state: &State,
        options: ConfirmOptions,
    ) -> Result<ConfirmOutcome, AssuanError> {
        (**self).confirm(state, options)
    }

    fn flavor(&self) -> &'static str {
        (**self).flavor()
    }
}

/// Always cancels, so gpg-agent gets a clean answer instead of a hang.
pub struct NullBackend {
    flavor: &'static str,
}

impl NullBackend {
    pub fn new() -> Self {
        Self { flavor: "null" }
    }
}

impl Default for NullBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for NullBackend {
    fn get_pin(&mut self, _state: &State) -> Result<PinResponse, AssuanError> {
        Err(AssuanError::canceled())
    }

    fn confirm(
        &mut self,
        _state: &State,
        _options: ConfirmOptions,
    ) -> Result<ConfirmOutcome, AssuanError> {
        Ok(ConfirmOutcome::Cancelled)
    }

    fn flavor(&self) -> &'static str {
        self.flavor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_button() {
        assert!(ConfirmOptions::parse("--one-button").one_button);
        assert!(!ConfirmOptions::parse("").one_button);
    }

    #[test]
    fn parses_focus() {
        assert_eq!(ConfirmOptions::parse("--focus=ok").focus, 1);
        assert_eq!(ConfirmOptions::parse("--focus=cancel").focus, -1);
        assert_eq!(ConfirmOptions::parse("").focus, 0);
    }

    #[test]
    fn parses_combined_flags() {
        let opts = ConfirmOptions::parse("--one-button --focus=cancel");
        assert!(opts.one_button);
        assert_eq!(opts.focus, -1);
    }
}
