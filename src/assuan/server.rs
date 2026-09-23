//! The command loop. Generic over reader and writer so tests can drive a full
//! session through in-memory buffers.

use std::io::{BufRead, Write};

use crate::assuan::codec;
use crate::assuan::error::AssuanError;
use crate::assuan::state::{Owner, State};
use crate::backend::{Backend, ConfirmOptions, ConfirmOutcome};

/// Assuan caps a line at 1000 bytes and escaping can triple a byte, so chunk
/// well below that.
const MAX_DATA_CHUNK: usize = 256;

const GREETING: &str = "OK Pleased to meet you";

pub struct Server<B: Backend> {
    state: State,
    backend: B,
    finished: bool,
}

impl<B: Backend> Server<B> {
    pub fn new(backend: B) -> Self {
        Self {
            state: State::new(),
            backend,
            finished: false,
        }
    }

    pub fn with_state(backend: B, state: State) -> Self {
        Self {
            state,
            backend,
            finished: false,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn run<R: BufRead, W: Write>(&mut self, reader: R, mut writer: W) -> std::io::Result<()> {
        writeln!(writer, "{GREETING}")?;
        writer.flush()?;

        for line in reader.lines() {
            let line = line?;
            self.handle_line(&line, &mut writer)?;
            writer.flush()?;
            if self.finished {
                break;
            }
        }
        Ok(())
    }

    fn handle_line<W: Write>(&mut self, line: &str, writer: &mut W) -> std::io::Result<()> {
        let line = line.trim_end_matches(['\r', '\n']);

        if line.is_empty() || line.starts_with('#') {
            return Ok(());
        }

        let (command, rest) = match line.split_once(' ') {
            Some((c, r)) => (c, r.trim_start()),
            None => (line, ""),
        };

        // Case-insensitive, as in libassuan.
        let upper = command.to_ascii_uppercase();

        match upper.as_str() {
            "BYE" => {
                self.finished = true;
                writeln!(writer, "OK closing connection")
            }
            "RESET" => {
                self.state.reset();
                writeln!(writer, "OK")
            }
            "NOP" => writeln!(writer, "OK"),
            "CANCEL" => write_err(writer, AssuanError::canceled()),
            "END" => writeln!(writer, "OK"),
            "OPTION" => self.cmd_option(rest, writer),
            "GETINFO" => self.cmd_getinfo(rest, writer),
            "GETPIN" => self.cmd_getpin(writer),
            "CONFIRM" => self.cmd_confirm(rest, writer),
            "MESSAGE" => self.cmd_message(writer),
            "SETDESC" => self.set_field(rest, writer, |r, v| r.description = v),
            "SETPROMPT" => self.set_field(rest, writer, |r, v| r.prompt = v),
            "SETTITLE" => self.set_field(rest, writer, |r, v| r.title = v),
            "SETERROR" => self.set_field(rest, writer, |r, v| r.error = v),
            "SETOK" => self.set_field(rest, writer, |r, v| r.ok = v),
            "SETNOTOK" => self.set_field(rest, writer, |r, v| r.notok = v),
            "SETCANCEL" => self.set_field(rest, writer, |r, v| r.cancel = v),
            "SETREPEAT" => self.set_field(rest, writer, |r, v| r.repeat_passphrase = v),
            "SETREPEATOK" => self.set_field(rest, writer, |r, v| r.repeat_ok = v),
            "SETREPEATERROR" => self.set_field(rest, writer, |r, v| r.repeat_error = v),
            "SETGENPIN" => self.set_field(rest, writer, |r, v| r.genpin_label = v),
            "SETGENPIN_TT" => self.set_field(rest, writer, |r, v| r.genpin_tt = v),
            "SETQUALITYBAR_TT" => self.set_field(rest, writer, |r, v| r.quality_bar_tt = v),
            "SETKEYINFO" => self.cmd_setkeyinfo(rest, writer),
            "SETQUALITYBAR" => self.cmd_setqualitybar(rest, writer),
            "SETTIMEOUT" => self.cmd_settimeout(rest, writer),
            // No external password cache, so nothing to clear; OK keeps
            // gpg-agent happy.
            "CLEARPASSPHRASE" => writeln!(writer, "OK"),
            _ => write_err(writer, AssuanError::unknown_command()),
        }
    }

    fn set_field<W: Write>(
        &mut self,
        rest: &str,
        writer: &mut W,
        assign: impl Fn(&mut crate::assuan::state::Request, Option<String>),
    ) -> std::io::Result<()> {
        let value = codec::decode_argument(rest);
        assign(
            &mut self.state.request,
            if value.is_empty() { None } else { Some(value) },
        );
        writeln!(writer, "OK")
    }

    fn cmd_setkeyinfo<W: Write>(&mut self, rest: &str, writer: &mut W) -> std::io::Result<()> {
        // Empty or `--clear` means "no stable identifier".
        self.state.request.keyinfo = if rest.is_empty() || rest == "--clear" {
            None
        } else {
            Some(rest.to_string())
        };
        writeln!(writer, "OK")
    }

    fn cmd_setqualitybar<W: Write>(&mut self, rest: &str, writer: &mut W) -> std::io::Result<()> {
        let label = if rest.is_empty() {
            "Quality:".to_string()
        } else {
            codec::decode_argument(rest)
        };
        self.state.request.quality_bar = Some(label);
        writeln!(writer, "OK")
    }

    fn cmd_settimeout<W: Write>(&mut self, rest: &str, writer: &mut W) -> std::io::Result<()> {
        if let Ok(secs) = rest.trim().parse::<u32>() {
            self.state.options.timeout_secs = secs;
        }
        writeln!(writer, "OK")
    }

    fn cmd_option<W: Write>(&mut self, rest: &str, writer: &mut W) -> std::io::Result<()> {
        // `key=value`, `key value`, or a bare flag, optionally `--` prefixed.
        let rest = rest.trim_start_matches("--");
        let (key, value) = match rest.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => match rest.split_once(' ') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => (rest.trim(), ""),
            },
        };

        let opts = &mut self.state.options;
        let owned = || value.to_string();
        let some = || Some(value.to_string());

        match key {
            "grab" => opts.grab = true,
            "no-grab" => opts.grab = false,
            "ttyname" => opts.ttyname = some(),
            "ttytype" => opts.ttytype = some(),
            "ttyalert" => opts.ttyalert = some(),
            "lc-ctype" => opts.lc_ctype = some(),
            "lc-messages" => opts.lc_messages = some(),
            "display" => opts.display = some(),
            "owner" => opts.owner = Some(Owner::parse(value)),
            "parent-wid" => opts.parent_wid = value.trim().parse().ok(),
            "touch-file" => opts.touch_file = some(),
            "default-ok" => opts.default_ok = some(),
            "default-cancel" => opts.default_cancel = some(),
            "default-prompt" => opts.default_prompt = some(),
            "default-pwmngr" => opts.default_pwmngr = some(),
            "default-cf-visi" => opts.default_cf_visi = some(),
            "default-tt-visi" => opts.default_tt_visi = some(),
            "default-tt-hide" => opts.default_tt_hide = some(),
            "default-capshint" => opts.default_capshint = some(),
            "allow-external-password-cache" => opts.allow_external_password_cache = true,
            "invisible-char" => opts.invisible_char = some(),
            "formatted-passphrase" => opts.formatted_passphrase = true,
            "formatted-passphrase-hint" => {
                opts.formatted_passphrase_hint = Some(codec::decode_argument(value))
            }
            "constraints-enforce" => opts.constraints_enforce = true,
            "constraints-hint-short" => {
                opts.constraints_hint_short = Some(codec::decode_argument(value))
            }
            "constraints-hint-long" => {
                opts.constraints_hint_long = Some(codec::decode_argument(value))
            }
            "constraints-error-title" => {
                opts.constraints_error_title = Some(codec::decode_argument(value))
            }
            // Tolerated rather than rejected, as upstream does.
            "debug-wait" | "allow-emacs-prompt" => {
                let _ = owned();
            }
            _ => return write_err(writer, AssuanError::parameter()),
        }

        writeln!(writer, "OK")
    }

    fn cmd_getinfo<W: Write>(&mut self, rest: &str, writer: &mut W) -> std::io::Result<()> {
        match rest.trim() {
            "version" => {
                write_data(writer, env!("CARGO_PKG_VERSION").as_bytes())?;
                writeln!(writer, "OK")
            }
            "pid" => {
                let pid = std::process::id().to_string();
                write_data(writer, pid.as_bytes())?;
                writeln!(writer, "OK")
            }
            "flavor" => {
                write_data(writer, self.backend.flavor().as_bytes())?;
                writeln!(writer, "OK")
            }
            "ttyinfo" => {
                let opts = &self.state.options;
                let info = format!(
                    "{} {} {} - {}/{} -",
                    opts.ttyname.as_deref().unwrap_or("-"),
                    opts.ttytype.as_deref().unwrap_or("-"),
                    opts.display.as_deref().unwrap_or("-"),
                    unsafe { libc::geteuid() },
                    unsafe { libc::getegid() },
                );
                write_data(writer, info.as_bytes())?;
                writeln!(writer, "OK")
            }
            _ => write_err(writer, AssuanError::parameter()),
        }
    }

    fn cmd_getpin<W: Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        let result = self.backend.get_pin(&self.state);
        self.state.request.clear_after_getpin();

        match result {
            Ok(pin) => {
                // Must precede the data: gpg-agent reads it as a status line
                // of this same transaction.
                if pin.repeated {
                    writeln!(writer, "S PIN_REPEATED")?;
                }
                write_data(writer, pin.secret.as_bytes())?;
                writeln!(writer, "OK")
            }
            Err(err) => write_err(writer, err),
        }
    }

    fn cmd_confirm<W: Write>(&mut self, rest: &str, writer: &mut W) -> std::io::Result<()> {
        let options = ConfirmOptions::parse(rest);
        let result = self.backend.confirm(&self.state, options);

        self.state.request.error = None;

        match result {
            // One-button prompts are informational: there is no way to say no.
            Ok(_) if options.one_button => writeln!(writer, "OK"),
            Ok(ConfirmOutcome::Confirmed) => writeln!(writer, "OK"),
            Ok(ConfirmOutcome::Declined) => write_err(writer, AssuanError::not_confirmed()),
            Ok(ConfirmOutcome::Cancelled) => write_err(writer, AssuanError::canceled()),
            Err(err) => write_err(writer, err),
        }
    }

    /// MESSAGE is CONFIRM with a single button.
    fn cmd_message<W: Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.cmd_confirm("--one-button", writer)
    }
}

/// Raw bytes so non-ASCII passphrases survive. Splitting a multi-byte
/// character across lines is harmless: the receiver concatenates first.
fn write_data<W: Write>(writer: &mut W, data: &[u8]) -> std::io::Result<()> {
    if data.is_empty() {
        writeln!(writer, "D ")?;
        return Ok(());
    }
    for chunk in data.chunks(MAX_DATA_CHUNK) {
        writer.write_all(b"D ")?;
        writer.write_all(&codec::encode_data(chunk))?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn write_err<W: Write>(writer: &mut W, err: AssuanError) -> std::io::Result<()> {
    writeln!(writer, "ERR {} {}", err.wire_value(), err.description)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{NullBackend, PinResponse};
    use crate::secret::Secret;

    fn run_session<B: Backend>(backend: B, input: &str) -> String {
        let mut server = Server::new(backend);
        let mut output = Vec::new();
        server
            .run(std::io::Cursor::new(input.as_bytes()), &mut output)
            .expect("session should not fail");
        String::from_utf8(output).expect("output should be utf-8")
    }

    struct FixedPin(&'static str);

    impl Backend for FixedPin {
        fn get_pin(&mut self, _state: &State) -> Result<PinResponse, AssuanError> {
            let mut s = Secret::new();
            s.push_str(self.0).unwrap();
            Ok(s.into())
        }
        fn confirm(
            &mut self,
            _state: &State,
            _options: ConfirmOptions,
        ) -> Result<ConfirmOutcome, AssuanError> {
            Ok(ConfirmOutcome::Confirmed)
        }
        fn flavor(&self) -> &'static str {
            "fixed"
        }
    }

    /// Answers as a frontend that had the passphrase typed twice.
    struct RepeatedPin(&'static str);

    impl Backend for RepeatedPin {
        fn get_pin(&mut self, _state: &State) -> Result<PinResponse, AssuanError> {
            let mut secret = Secret::new();
            secret.push_str(self.0).unwrap();
            Ok(PinResponse {
                secret,
                repeated: true,
            })
        }
        fn confirm(
            &mut self,
            _state: &State,
            _options: ConfirmOptions,
        ) -> Result<ConfirmOutcome, AssuanError> {
            Ok(ConfirmOutcome::Confirmed)
        }
        fn flavor(&self) -> &'static str {
            "repeated"
        }
    }

    #[test]
    fn pin_repeated_status_precedes_the_data() {
        let out = run_session(RepeatedPin("s3cret"), "SETREPEAT Repeat:\nGETPIN\n");
        assert!(
            out.contains("S PIN_REPEATED\nD s3cret\nOK\n"),
            "status must come right before the data: {out}"
        );
    }

    #[test]
    fn a_single_entry_sends_no_pin_repeated() {
        let out = run_session(FixedPin("s3cret"), "SETREPEAT Repeat:\nGETPIN\n");
        assert!(!out.contains("PIN_REPEATED"), "{out}");
    }

    #[test]
    fn greets_on_connect() {
        let out = run_session(NullBackend::new(), "");
        assert_eq!(out, "OK Pleased to meet you\n");
    }

    #[test]
    fn bye_closes_the_connection() {
        let out = run_session(NullBackend::new(), "BYE\n");
        assert!(out.ends_with("OK closing connection\n"), "{out}");
    }

    #[test]
    fn stops_reading_after_bye() {
        let out = run_session(NullBackend::new(), "BYE\nGETPIN\n");
        assert_eq!(out.matches("OK").count(), 2, "{out}");
        assert!(!out.contains("ERR"), "{out}");
    }

    #[test]
    fn unknown_command_reports_unknown_ipc_command() {
        let out = run_session(NullBackend::new(), "FLYAWAY\n");
        assert!(out.contains("ERR 83886355 Unknown IPC command"), "{out}");
    }

    #[test]
    fn commands_are_case_insensitive() {
        // The greeting also starts with OK, so match the bare acknowledgements.
        let out = run_session(NullBackend::new(), "nop\nNoP\n");
        assert_eq!(out.matches("OK\n").count(), 2, "{out}");
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let out = run_session(NullBackend::new(), "# a comment\n\nNOP\n");
        // Greeting plus a single OK.
        assert_eq!(out.matches("OK").count(), 2, "{out}");
    }

    #[test]
    fn getpin_returns_data_then_ok() {
        let out = run_session(FixedPin("hunter2"), "GETPIN\n");
        assert!(out.contains("D hunter2\n"), "{out}");
        assert!(out.trim_end().ends_with("OK"), "{out}");
    }

    #[test]
    fn getpin_escapes_percent_in_the_passphrase() {
        let out = run_session(FixedPin("100%sure"), "GETPIN\n");
        assert!(out.contains("D 100%25sure\n"), "{out}");
    }

    #[test]
    fn getpin_cancel_reports_the_canceled_code() {
        let out = run_session(NullBackend::new(), "GETPIN\n");
        assert!(out.contains("ERR 83886179 Operation cancelled"), "{out}");
    }

    #[test]
    fn confirm_cancel_maps_to_canceled() {
        let out = run_session(NullBackend::new(), "CONFIRM\n");
        assert!(out.contains("ERR 83886179"), "{out}");
    }

    #[test]
    fn message_always_succeeds_even_when_the_backend_cancels() {
        // Nothing to decline, so a cancelling backend must still yield OK.
        let out = run_session(NullBackend::new(), "MESSAGE\n");
        assert!(!out.contains("ERR"), "{out}");
    }

    #[test]
    fn setdesc_is_percent_decoded() {
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(
                std::io::Cursor::new("SETDESC Enter%20passphrase%0Afor%20key\nBYE\n"),
                &mut out,
            )
            .unwrap();
        assert_eq!(
            server.state().request.description.as_deref(),
            Some("Enter passphrase\nfor key")
        );
    }

    #[test]
    fn option_parses_key_value_form() {
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(
                std::io::Cursor::new("OPTION ttyname=/dev/pts/7\nOPTION owner=4242/1000 nixos\n"),
                &mut out,
            )
            .unwrap();
        assert_eq!(
            server.state().options.ttyname.as_deref(),
            Some("/dev/pts/7")
        );
        let owner = server.state().options.owner.clone().unwrap();
        assert_eq!(owner.pid, Some(4242));
        assert_eq!(owner.host.as_deref(), Some("nixos"));
    }

    #[test]
    fn option_accepts_leading_dashes_and_bare_flags() {
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(std::io::Cursor::new("OPTION --no-grab\n"), &mut out)
            .unwrap();
        assert!(!server.state().options.grab);
    }

    #[test]
    fn reset_clears_the_prompt_but_keeps_the_tty() {
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(
                std::io::Cursor::new("OPTION ttyname=/dev/pts/1\nSETDESC hello\nRESET\n"),
                &mut out,
            )
            .unwrap();
        assert_eq!(
            server.state().options.ttyname.as_deref(),
            Some("/dev/pts/1")
        );
        assert_eq!(server.state().request.description, None);
    }

    #[test]
    fn settimeout_is_recorded_and_garbage_is_ignored() {
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(std::io::Cursor::new("SETTIMEOUT 30\n"), &mut out)
            .unwrap();
        assert_eq!(server.state().options.timeout_secs, 30);

        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(std::io::Cursor::new("SETTIMEOUT nonsense\n"), &mut out)
            .unwrap();
        assert_eq!(server.state().options.timeout_secs, 60);
    }

    #[test]
    fn settimeout_survives_reset() {
        // gpg-agent sends SETTIMEOUT once, then RESET before prompts without
        // a description; the timeout must not snap back to the default.
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(std::io::Cursor::new("SETTIMEOUT 300\nRESET\n"), &mut out)
            .unwrap();
        assert_eq!(server.state().options.timeout_secs, 300);
    }

    #[test]
    fn setqualitybar_defaults_its_label() {
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(std::io::Cursor::new("SETQUALITYBAR\n"), &mut out)
            .unwrap();
        assert_eq!(
            server.state().request.quality_bar.as_deref(),
            Some("Quality:")
        );
    }

    #[test]
    fn setkeyinfo_clear_removes_the_identifier() {
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(
                std::io::Cursor::new("SETKEYINFO n/ABCD\nSETKEYINFO --clear\n"),
                &mut out,
            )
            .unwrap();
        assert_eq!(server.state().request.keyinfo, None);
    }

    #[test]
    fn getinfo_version_and_flavor() {
        let out = run_session(FixedPin("x"), "GETINFO version\nGETINFO flavor\n");
        assert!(
            out.contains(&format!("D {}", env!("CARGO_PKG_VERSION"))),
            "{out}"
        );
        assert!(out.contains("D fixed"), "{out}");
    }

    #[test]
    fn getinfo_rejects_unknown_keys() {
        let out = run_session(NullBackend::new(), "GETINFO wat\n");
        assert!(out.contains("ERR 83886360"), "{out}");
    }

    #[test]
    fn long_passphrase_is_split_across_data_lines() {
        let long: &'static str = Box::leak("a".repeat(700).into_boxed_str());
        let out = run_session(FixedPin(long), "GETPIN\n");
        let data_lines: Vec<_> = out.lines().filter(|l| l.starts_with("D ")).collect();
        assert!(
            data_lines.len() > 1,
            "expected chunking, got {data_lines:?}"
        );
        for line in &data_lines {
            assert!(line.len() < 1000, "line too long: {}", line.len());
        }
        let joined: String = data_lines.iter().map(|l| &l[2..]).collect();
        assert_eq!(joined, long);
    }

    #[test]
    fn error_is_cleared_between_prompts() {
        let mut server = Server::new(NullBackend::new());
        let mut out = Vec::new();
        server
            .run(
                std::io::Cursor::new("SETERROR Bad passphrase\nGETPIN\n"),
                &mut out,
            )
            .unwrap();
        assert_eq!(server.state().request.error, None);
    }
}
