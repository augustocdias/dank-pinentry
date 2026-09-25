//! `dank-askpass`: a `SUDO_ASKPASS` / `SSH_ASKPASS` program that draws its
//! prompt through the DMS plugin.
//!
//! The caller passes the prompt as argv[1] and reads the answer from stdout;
//! a non-zero exit means cancelled. There is deliberately no terminal
//! fallback: sudo and ssh only use an askpass when they cannot, or were told
//! not to, prompt on a terminal themselves.

use std::io::Write;
use std::process::ExitCode;

use zeroize::Zeroize;

use dank_pinentry::assuan::state::Options;
use dank_pinentry::backend::DmsBackend;
use dank_pinentry::backend::dms::{OwnerInfo, PROTOCOL_VERSION, Request, request_id};
use dank_pinentry::config::Config;
use dank_pinentry::owner;

/// What the caller wants, from ssh's `SSH_ASKPASS_PROMPT`. sudo never sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Passphrase,
    Confirm,
    Notify,
}

impl Mode {
    fn from_env(value: Option<&str>) -> Self {
        match value {
            Some("confirm") => Self::Confirm,
            Some("none") => Self::Notify,
            _ => Self::Passphrase,
        }
    }

    fn request_kind(self) -> &'static str {
        match self {
            Self::Passphrase => "getpin",
            Self::Confirm => "confirm",
            Self::Notify => "message",
        }
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(prompt) = args.next() else {
        eprintln!("usage: dank-askpass PROMPT");
        eprintln!("Set SUDO_ASKPASS or SSH_ASKPASS to this program.");
        return ExitCode::FAILURE;
    };
    let prompt = prompt.to_string_lossy().trim_end().to_string();
    let mode = Mode::from_env(std::env::var("SSH_ASKPASS_PROMPT").ok().as_deref());

    let config = Config::load();
    let socket = config.socket_path();
    if !DmsBackend::is_available(&socket) {
        eprintln!(
            "dank-askpass: the DMS plugin is not listening on {}",
            socket.display()
        );
        return ExitCode::FAILURE;
    }

    // The caller is the parent: sudo or ssh forks and execs us directly.
    let parent = std::os::unix::process::parent_id();
    let request = Request {
        v: PROTOCOL_VERSION,
        kind: mode.request_kind(),
        id: request_id(),
        description: (!prompt.is_empty()).then_some(prompt),
        owner: Some(OwnerInfo {
            pid: Some(parent),
            uid: Some(unsafe { libc::getuid() }),
            host: None,
            command: owner::command_line(parent),
        }),
        timeout: config.timeout.unwrap_or(Options::DEFAULT_TIMEOUT_SECS),
        ..Request::default()
    };

    let reply = match DmsBackend::new(socket).prompt(&request) {
        Ok(reply) => reply,
        Err(err) => {
            eprintln!("dank-askpass: {err}");
            return ExitCode::FAILURE;
        }
    };

    match (mode, reply.kind.as_str()) {
        (Mode::Passphrase, "pin") => {
            let Some(mut pin) = reply.pin else {
                eprintln!("dank-askpass: reply carried no passphrase");
                return ExitCode::FAILURE;
            };
            let mut stdout = std::io::stdout().lock();
            let written = stdout
                .write_all(pin.as_bytes())
                .and_then(|()| stdout.write_all(b"\n"))
                .and_then(|()| stdout.flush());
            pin.zeroize();
            if written.is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        (Mode::Confirm, "confirm") if reply.outcome.as_deref() == Some("confirmed") => {
            ExitCode::SUCCESS
        }
        // Nothing to answer; ssh discards stdout for notifications.
        (Mode::Notify, _) => ExitCode::SUCCESS,
        (_, "error") => {
            if let Some(msg) = reply.message {
                eprintln!("dank-askpass: plugin error: {msg}");
            }
            ExitCode::FAILURE
        }
        _ => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_follows_ssh_askpass_prompt() {
        assert_eq!(Mode::from_env(None), Mode::Passphrase);
        assert_eq!(Mode::from_env(Some("")), Mode::Passphrase);
        assert_eq!(Mode::from_env(Some("confirm")), Mode::Confirm);
        assert_eq!(Mode::from_env(Some("none")), Mode::Notify);
        assert_eq!(Mode::request_kind(Mode::Confirm), "confirm");
        assert_eq!(Mode::request_kind(Mode::Notify), "message");
    }
}
