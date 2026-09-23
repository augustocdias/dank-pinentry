//! gpg-error codes for `ERR <code> <description>`.
//!
//! A value packs `(source << 24) | code`. Values come from libgpg-error's
//! `err-sources.h.in` and `err-codes.h.in`; the tests pin the two that matter
//! so a wrong constant fails the build rather than confusing gpg-agent.

pub const SOURCE_PINENTRY: u32 = 5;

pub const fn gpg_error(code: u32) -> u32 {
    (SOURCE_PINENTRY << 24) | code
}

pub const GENERAL: u32 = 1;
pub const BAD_PASSPHRASE: u32 = 11;
pub const TIMEOUT: u32 = 62;
pub const CANCELED: u32 = 99;
pub const NOT_CONFIRMED: u32 = 114;
pub const LOCALE_PROBLEM: u32 = 166;
pub const FULLY_CANCELED: u32 = 198;
pub const ASS_GENERAL: u32 = 257;
pub const ASS_INV_VALUE: u32 = 261;
pub const ASS_UNKNOWN_CMD: u32 = 275;
pub const ASS_NO_INPUT: u32 = 278;
pub const ASS_PARAMETER: u32 = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssuanError {
    pub code: u32,
    pub description: &'static str,
}

impl AssuanError {
    pub const fn new(code: u32, description: &'static str) -> Self {
        Self { code, description }
    }

    pub const fn wire_value(&self) -> u32 {
        gpg_error(self.code)
    }

    pub const fn canceled() -> Self {
        Self::new(CANCELED, "Operation cancelled")
    }

    pub const fn not_confirmed() -> Self {
        Self::new(NOT_CONFIRMED, "Not confirmed")
    }

    /// gpg-agent stops retrying on this, unlike [`Self::canceled`].
    pub const fn fully_canceled() -> Self {
        Self::new(FULLY_CANCELED, "Operation fully cancelled")
    }

    pub const fn timeout() -> Self {
        Self::new(TIMEOUT, "Timeout")
    }

    pub const fn unknown_command() -> Self {
        Self::new(ASS_UNKNOWN_CMD, "Unknown IPC command")
    }

    pub const fn parameter() -> Self {
        Self::new(ASS_PARAMETER, "IPC parameter error")
    }

    pub const fn no_input() -> Self {
        Self::new(ASS_NO_INPUT, "No input source for IPC")
    }

    pub const fn general(description: &'static str) -> Self {
        Self::new(GENERAL, description)
    }
}

impl std::fmt::Display for AssuanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.wire_value(), self.description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_matches_the_value_gpg_agent_logs() {
        // What real pinentry emits; a change here breaks cancellation.
        assert_eq!(AssuanError::canceled().wire_value(), 83_886_179);
    }

    #[test]
    fn source_is_pinentry_not_agent() {
        assert_eq!(gpg_error(0) >> 24, 5);
    }

    #[test]
    fn not_confirmed_wire_value() {
        assert_eq!(AssuanError::not_confirmed().wire_value(), 83_886_194);
    }
}
