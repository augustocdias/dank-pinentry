//! Accumulated pinentry state.
//!
//! The [`Options`]/[`Request`] split mirrors upstream's `pinentry_reset()`:
//! `OPTION` lines survive a `RESET`, per-prompt `SET*` values do not. Getting
//! this backwards leaks stale descriptions into the next prompt.

/// Preserved across `RESET`.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub grab: bool,
    pub ttyname: Option<String>,
    pub ttytype: Option<String>,
    pub ttyalert: Option<String>,
    pub lc_ctype: Option<String>,
    pub lc_messages: Option<String>,
    pub display: Option<String>,
    pub owner: Option<Owner>,
    pub parent_wid: Option<i64>,
    pub touch_file: Option<String>,
    pub default_ok: Option<String>,
    pub default_cancel: Option<String>,
    pub default_prompt: Option<String>,
    pub default_pwmngr: Option<String>,
    pub default_cf_visi: Option<String>,
    pub default_tt_visi: Option<String>,
    pub default_tt_hide: Option<String>,
    pub default_capshint: Option<String>,
    pub allow_external_password_cache: bool,
    pub invisible_char: Option<String>,
    pub formatted_passphrase: bool,
    pub formatted_passphrase_hint: Option<String>,
    pub constraints_enforce: bool,
    pub constraints_hint_short: Option<String>,
    pub constraints_hint_long: Option<String>,
    pub constraints_error_title: Option<String>,
    /// Seconds before a prompt gives up; 0 means never. Kept here rather than
    /// in [`Request`] because gpg-agent sends `SETTIMEOUT` only once per
    /// process, then `RESET` before prompts that lack a description.
    pub timeout_secs: u32,
}

impl Options {
    /// Upstream's default when neither the command line nor gpg-agent set one.
    pub const DEFAULT_TIMEOUT_SECS: u32 = 60;

    pub fn new() -> Self {
        Self {
            // Upstream defaults to grabbing the keyboard.
            grab: true,
            timeout_secs: Self::DEFAULT_TIMEOUT_SECS,
            ..Default::default()
        }
    }
}

/// From `OPTION owner=`; lets the UI name who is asking.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Owner {
    pub pid: Option<u32>,
    pub uid: Option<u32>,
    pub host: Option<String>,
}

impl Owner {
    /// `<pid>/<uid> <host>`. Every part is optional; malformed input yields an
    /// empty `Owner` rather than an error, matching upstream's permissiveness.
    pub fn parse(value: &str) -> Self {
        let mut owner = Owner::default();
        let (ids, host) = match value.split_once(' ') {
            Some((ids, host)) => (ids, Some(host.trim())),
            None => (value, None),
        };

        let (pid, uid) = match ids.split_once('/') {
            Some((pid, uid)) => (pid, Some(uid)),
            None => (ids, None),
        };

        owner.pid = pid.trim().parse().ok();
        owner.uid = uid.and_then(|u| u.trim().parse().ok());
        owner.host = host
            .map(|h| h.split_whitespace().next().unwrap_or("").to_string())
            .filter(|h| !h.is_empty());
        owner
    }
}

/// Cleared by `RESET`, and partly after each `GETPIN`.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub title: Option<String>,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub error: Option<String>,
    pub ok: Option<String>,
    pub notok: Option<String>,
    pub cancel: Option<String>,
    pub keyinfo: Option<String>,
    pub repeat_passphrase: Option<String>,
    pub repeat_ok: Option<String>,
    pub repeat_error: Option<String>,
    pub quality_bar: Option<String>,
    pub quality_bar_tt: Option<String>,
    pub genpin_label: Option<String>,
    pub genpin_tt: Option<String>,
}

impl Request {
    pub fn new() -> Self {
        Self::default()
    }

    /// These are one-shot: a stale `error` would re-display "Bad passphrase"
    /// on an unrelated prompt, and a stale `repeat_passphrase` would wrongly
    /// ask for confirmation again.
    pub fn clear_after_getpin(&mut self) {
        self.error = None;
        self.repeat_passphrase = None;
        self.quality_bar = None;
    }
}

#[derive(Debug, Clone)]
pub struct State {
    pub options: Options,
    pub request: Request,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            options: Options::new(),
            request: Request::new(),
        }
    }

    /// Later `OPTION` lines overwrite these, matching upstream precedence.
    pub fn with_options(options: Options) -> Self {
        Self {
            options,
            request: Request::new(),
        }
    }

    pub fn reset(&mut self) {
        self.request = Request::new();
    }

    pub fn effective_prompt(&self) -> &str {
        self.request
            .prompt
            .as_deref()
            .or(self.options.default_prompt.as_deref())
            .unwrap_or("PIN:")
    }

    pub fn effective_ok(&self) -> &str {
        self.request
            .ok
            .as_deref()
            .or(self.options.default_ok.as_deref())
            .unwrap_or("OK")
    }

    pub fn effective_cancel(&self) -> &str {
        self.request
            .cancel
            .as_deref()
            .or(self.options.default_cancel.as_deref())
            .unwrap_or("Cancel")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_parses_full_form() {
        let owner = Owner::parse("12345/1000 nixos");
        assert_eq!(owner.pid, Some(12345));
        assert_eq!(owner.uid, Some(1000));
        assert_eq!(owner.host.as_deref(), Some("nixos"));
    }

    #[test]
    fn owner_parses_pid_only() {
        let owner = Owner::parse("12345");
        assert_eq!(owner.pid, Some(12345));
        assert_eq!(owner.uid, None);
        assert_eq!(owner.host, None);
    }

    #[test]
    fn owner_parses_pid_and_uid_without_host() {
        let owner = Owner::parse("42/0");
        assert_eq!(owner.pid, Some(42));
        assert_eq!(owner.uid, Some(0));
        assert_eq!(owner.host, None);
    }

    #[test]
    fn owner_tolerates_garbage() {
        let owner = Owner::parse("not-a-pid");
        assert_eq!(owner.pid, None);
    }

    #[test]
    fn reset_keeps_options_but_clears_request() {
        let mut state = State::new();
        state.options.ttyname = Some("/dev/pts/3".into());
        state.request.description = Some("secret thing".into());

        state.reset();

        assert_eq!(state.options.ttyname.as_deref(), Some("/dev/pts/3"));
        assert_eq!(state.request.description, None);
    }

    #[test]
    fn timeout_defaults_to_upstreams_value() {
        assert_eq!(State::new().options.timeout_secs, 60);
    }

    #[test]
    fn error_does_not_leak_into_the_next_prompt() {
        let mut state = State::new();
        state.request.error = Some("Bad passphrase".into());
        state.request.repeat_passphrase = Some("Repeat:".into());

        state.request.clear_after_getpin();

        assert_eq!(state.request.error, None);
        assert_eq!(state.request.repeat_passphrase, None);
    }

    #[test]
    fn prompt_falls_back_through_default_then_builtin() {
        let mut state = State::new();
        assert_eq!(state.effective_prompt(), "PIN:");

        state.options.default_prompt = Some("Passphrase:".into());
        assert_eq!(state.effective_prompt(), "Passphrase:");

        state.request.prompt = Some("Key passphrase:".into());
        assert_eq!(state.effective_prompt(), "Key passphrase:");
    }
}
