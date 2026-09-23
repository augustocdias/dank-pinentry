//! Replays realistic gpg-agent sessions against the protocol layer.
//!
//! The command sequences here are modelled on what gpg-agent actually sends
//! for signing, key generation and smartcard prompts. Unit tests cover
//! individual commands; these guard the ordering and the interactions between
//! them.

use dank_pinentry::assuan::Server;
use dank_pinentry::assuan::error::AssuanError;
use dank_pinentry::assuan::state::State;
use dank_pinentry::backend::{Backend, ConfirmOptions, ConfirmOutcome, NullBackend, PinResponse};
use dank_pinentry::secret::Secret;

/// Records what the frontend was asked, and replies with a script.
struct ScriptedBackend {
    pin: Option<&'static str>,
    confirm: ConfirmOutcome,
    /// Descriptions observed, so tests can assert on what the UI would show.
    seen_descriptions: Vec<Option<String>>,
    seen_errors: Vec<Option<String>>,
}

impl ScriptedBackend {
    fn returning_pin(pin: &'static str) -> Self {
        Self {
            pin: Some(pin),
            confirm: ConfirmOutcome::Confirmed,
            seen_descriptions: Vec::new(),
            seen_errors: Vec::new(),
        }
    }

    fn confirming(outcome: ConfirmOutcome) -> Self {
        Self {
            pin: None,
            confirm: outcome,
            seen_descriptions: Vec::new(),
            seen_errors: Vec::new(),
        }
    }
}

impl Backend for ScriptedBackend {
    fn get_pin(&mut self, state: &State) -> Result<PinResponse, AssuanError> {
        self.seen_descriptions
            .push(state.request.description.clone());
        self.seen_errors.push(state.request.error.clone());

        match self.pin {
            Some(p) => {
                let mut s = Secret::new();
                s.push_str(p).unwrap();
                Ok(s.into())
            }
            None => Err(AssuanError::canceled()),
        }
    }

    fn confirm(
        &mut self,
        state: &State,
        _options: ConfirmOptions,
    ) -> Result<ConfirmOutcome, AssuanError> {
        self.seen_descriptions
            .push(state.request.description.clone());
        Ok(self.confirm)
    }

    fn flavor(&self) -> &'static str {
        "scripted"
    }
}

fn replay<B: Backend>(backend: B, script: &str) -> (String, Server<B>) {
    let mut server = Server::new(backend);
    let mut out = Vec::new();
    server
        .run(std::io::Cursor::new(script.as_bytes()), &mut out)
        .expect("replay should not fail");
    (String::from_utf8(out).expect("utf-8 output"), server)
}

/// The sequence gpg-agent sends when signing a commit.
const SIGNING_SESSION: &str = "\
OPTION grab
OPTION ttyname=/dev/pts/17
OPTION ttytype=xterm-256color
OPTION lc-messages=en_GB.UTF-8
OPTION owner=44212/1000 nixos
OPTION default-ok=_OK
OPTION default-cancel=_Cancel
OPTION default-prompt=PIN:
OPTION allow-external-password-cache
SETKEYINFO n/9A1B2C3D4E5F
SETDESC Please%20enter%20the%20passphrase%20to%20unlock%20the%20OpenPGP%20secret%20key:%0A%22Augusto%20Dias%20%3Ca@example.com%3E%22%0A2048-bit%20RSA%20key
SETPROMPT Passphrase:
GETPIN
BYE
";

#[test]
fn signing_session_returns_the_passphrase() {
    let (out, _) = replay(
        ScriptedBackend::returning_pin("correct horse"),
        SIGNING_SESSION,
    );

    assert!(out.starts_with("OK Pleased to meet you\n"), "{out}");
    assert!(out.contains("D correct horse\n"), "{out}");
    assert!(out.ends_with("OK closing connection\n"), "{out}");
    assert!(!out.contains("ERR"), "{out}");
}

#[test]
fn signing_session_exposes_a_decoded_multiline_description() {
    let mut server = Server::new(ScriptedBackend::returning_pin("x"));
    let mut out = Vec::new();
    server
        .run(std::io::Cursor::new(SIGNING_SESSION.as_bytes()), &mut out)
        .unwrap();

    // The UI must receive real newlines and a decoded angle-bracket address,
    // not the percent-escaped form.
    let desc = server
        .state()
        .request
        .description
        .clone()
        .expect("description was set");
    assert!(
        desc.contains('\n'),
        "expected multi-line description: {desc:?}"
    );
    assert!(desc.contains("<a@example.com>"), "{desc:?}");
    assert!(!desc.contains('%'), "still escaped: {desc:?}");
}

#[test]
fn owner_is_available_for_attributing_the_request() {
    let (_, server) = replay(ScriptedBackend::returning_pin("x"), SIGNING_SESSION);
    let owner = server.state().options.owner.clone().expect("owner parsed");
    assert_eq!(owner.pid, Some(44212));
    assert_eq!(owner.uid, Some(1000));
    assert_eq!(owner.host.as_deref(), Some("nixos"));
}

#[test]
fn cancelling_a_signing_session_reports_canceled_once() {
    let (out, _) = replay(
        ScriptedBackend::confirming(ConfirmOutcome::Cancelled),
        SIGNING_SESSION,
    );
    assert_eq!(out.matches("ERR 83886179").count(), 1, "{out}");
}

/// gpg-agent retries after a wrong passphrase by setting SETERROR and asking
/// again on the same connection.
#[test]
fn retry_after_bad_passphrase_does_not_leak_the_error_into_the_retry() {
    let script = "\
SETDESC First%20attempt
GETPIN
SETERROR Bad%20Passphrase
SETDESC Second%20attempt
GETPIN
BYE
";
    let mut server = Server::new(ScriptedBackend::returning_pin("pw"));
    let mut out = Vec::new();
    server
        .run(std::io::Cursor::new(script.as_bytes()), &mut out)
        .unwrap();
    let out = String::from_utf8(out).unwrap();

    assert_eq!(out.matches("D pw\n").count(), 2, "{out}");

    // The frontend should have seen the error on the second prompt only.
    let backend_errors = {
        // Re-run capturing the backend so we can inspect what it observed.
        let mut b = ScriptedBackend::returning_pin("pw");
        {
            let mut s = Server::new(&mut b);
            let mut o = Vec::new();
            s.run(std::io::Cursor::new(script.as_bytes()), &mut o)
                .unwrap();
        }
        b.seen_errors
    };
    assert_eq!(backend_errors.len(), 2);
    assert_eq!(backend_errors[0], None, "first prompt must have no error");
    assert_eq!(
        backend_errors[1].as_deref(),
        Some("Bad Passphrase"),
        "retry should show the agent's error"
    );

    // And it must be cleared afterwards so a third prompt starts clean.
    assert_eq!(server.state().request.error, None);
}

/// Key generation asks for the passphrase twice and shows a quality bar.
#[test]
fn key_generation_session_sets_repeat_and_quality_bar() {
    let script = "\
SETDESC Please%20enter%20the%20passphrase%20for%20the%20new%20key
SETPROMPT Passphrase:
SETREPEAT Repeat:
SETREPEATERROR Passphrases%20do%20not%20match
SETQUALITYBAR
SETQUALITYBAR_TT Quality%20of%20the%20passphrase
GETPIN
BYE
";
    let mut server = Server::new(ScriptedBackend::returning_pin("s3cret"));
    let mut out = Vec::new();
    server
        .run(std::io::Cursor::new(script.as_bytes()), &mut out)
        .unwrap();
    let out = String::from_utf8(out).unwrap();

    assert!(out.contains("D s3cret\n"), "{out}");

    // Both are one-shot and must not survive into the next prompt.
    assert_eq!(server.state().request.repeat_passphrase, None);
    assert_eq!(server.state().request.quality_bar, None);
}

/// A CONFIRM prompt, e.g. "really delete this key?".
#[test]
fn confirm_declined_maps_to_not_confirmed() {
    let script = "SETDESC Really%20delete%3F\nCONFIRM\nBYE\n";
    let (out, _) = replay(
        ScriptedBackend::confirming(ConfirmOutcome::Declined),
        script,
    );
    assert!(out.contains("ERR 83886194 Not confirmed"), "{out}");
}

#[test]
fn confirm_accepted_maps_to_ok() {
    let script = "SETDESC Really%20delete%3F\nCONFIRM\nBYE\n";
    let (out, _) = replay(
        ScriptedBackend::confirming(ConfirmOutcome::Confirmed),
        script,
    );
    assert!(!out.contains("ERR"), "{out}");
}

/// A smartcard prompt: "insert card", informational only.
#[test]
fn message_is_informational_and_cannot_fail() {
    let script = "SETDESC Please%20insert%20the%20card\nMESSAGE\nBYE\n";
    let (out, _) = replay(
        ScriptedBackend::confirming(ConfirmOutcome::Cancelled),
        script,
    );
    assert!(
        !out.contains("ERR"),
        "one-button prompts must not error: {out}"
    );
}

/// A connection may be reused for several unrelated prompts with RESET between.
#[test]
fn reset_between_prompts_isolates_them() {
    let script = "\
OPTION ttyname=/dev/pts/9
SETDESC First%20key
GETPIN
RESET
GETPIN
BYE
";
    let mut backend = ScriptedBackend::returning_pin("pw");
    {
        let mut server = Server::new(&mut backend);
        let mut out = Vec::new();
        server
            .run(std::io::Cursor::new(script.as_bytes()), &mut out)
            .unwrap();
        // ttyname is an agent-level option and must survive RESET.
        assert_eq!(
            server.state().options.ttyname.as_deref(),
            Some("/dev/pts/9")
        );
    }

    assert_eq!(backend.seen_descriptions.len(), 2);
    assert_eq!(backend.seen_descriptions[0].as_deref(), Some("First key"));
    assert_eq!(
        backend.seen_descriptions[1], None,
        "RESET must clear the description"
    );
}

#[test]
fn unknown_commands_do_not_abort_the_session() {
    let script = "SOMETHINGNEW foo\nNOP\nBYE\n";
    let (out, _) = replay(NullBackend::new(), script);
    assert!(out.contains("ERR 83886355"), "{out}");
    assert!(out.ends_with("OK closing connection\n"), "{out}");
}
