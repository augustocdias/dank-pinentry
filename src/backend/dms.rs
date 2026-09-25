//! The DankMaterialShell frontend: one newline-delimited JSON request/reply
//! per prompt over a Unix socket the plugin listens on. See `PROTOCOL.md`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::assuan::error::AssuanError;
use crate::assuan::state::State;
use crate::backend::{Backend, ConfirmOptions, ConfirmOutcome, PinResponse};
use crate::owner;
use crate::secret::Secret;

pub const PROTOCOL_VERSION: u32 = 1;

/// Probing happens before every prompt, so a hung shell must not stall
/// gpg-agent.
const PROBE_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Debug, Serialize)]
pub struct OwnerInfo {
    pub pid: Option<u32>,
    pub uid: Option<u32>,
    pub host: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Request {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notok: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<String>,
    #[serde(rename = "repeatError", skip_serializing_if = "Option::is_none")]
    pub repeat_error: Option<String>,
    #[serde(rename = "qualityBar", skip_serializing_if = "Option::is_none")]
    pub quality_bar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyinfo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<OwnerInfo>,
    pub timeout: u32,
}

#[derive(Debug, Deserialize)]
pub struct Reply {
    #[serde(default)]
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub pin: Option<String>,
    /// The prompt had the user type the passphrase twice.
    #[serde(default)]
    pub repeated: bool,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

pub struct DmsBackend {
    socket_path: PathBuf,
}

impl DmsBackend {
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// A socket file can outlive its process, so only a completed round trip
    /// proves the plugin is there.
    pub fn is_available(socket_path: &Path) -> bool {
        let Ok(stream) = Self::connect(socket_path) else {
            return false;
        };
        let request = serde_json::json!({ "v": PROTOCOL_VERSION, "type": "ping" });
        matches!(
            Self::exchange(stream, &request.to_string()),
            Ok(reply) if reply.kind == "pong"
        )
    }

    fn connect(socket_path: &Path) -> Result<UnixStream, AssuanError> {
        let stream = UnixStream::connect(socket_path).map_err(|_| AssuanError::no_input())?;

        stream
            .set_read_timeout(Some(PROBE_TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(PROBE_TIMEOUT)))
            .map_err(|_| AssuanError::no_input())?;

        if !peer_is_same_user(&stream) {
            eprintln!("dank-pinentry: refusing socket owned by another user");
            return Err(AssuanError::no_input());
        }

        Ok(stream)
    }

    fn exchange(mut stream: UnixStream, payload: &str) -> Result<Reply, AssuanError> {
        stream
            .write_all(payload.as_bytes())
            .and_then(|()| stream.write_all(b"\n"))
            .and_then(|()| stream.flush())
            .map_err(|_| AssuanError::no_input())?;

        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .map_err(|_| AssuanError::no_input())?;

        if line.trim().is_empty() {
            return Err(AssuanError::no_input());
        }

        let reply: Reply = serde_json::from_str(line.trim())
            .map_err(|_| AssuanError::general("malformed reply"))?;

        if reply.v != 0 && reply.v != PROTOCOL_VERSION {
            return Err(AssuanError::general("protocol version mismatch"));
        }

        Ok(reply)
    }

    /// No read timeout: the user may take minutes, and the request's own
    /// `timeout` governs expiry.
    pub fn prompt(&self, request: &Request) -> Result<Reply, AssuanError> {
        let stream = Self::connect(&self.socket_path)?;
        stream
            .set_read_timeout(None)
            .map_err(|_| AssuanError::no_input())?;

        let payload =
            serde_json::to_string(request).map_err(|_| AssuanError::general("encode failed"))?;

        let reply = Self::exchange(stream, &payload)?;

        // Discard a reply from some other, abandoned prompt.
        if let Some(id) = &reply.id
            && id != &request.id
        {
            return Err(AssuanError::general("reply id mismatch"));
        }

        Ok(reply)
    }

    fn build_request(&self, state: &State, kind: &'static str) -> Request {
        let owner = state.options.owner.as_ref().map(|o| OwnerInfo {
            pid: o.pid,
            uid: o.uid,
            host: o.host.clone(),
            command: o.pid.and_then(owner::command_line),
        });

        Request {
            v: PROTOCOL_VERSION,
            kind,
            id: request_id(),
            title: state.request.title.clone(),
            description: state.request.description.clone(),
            prompt: Some(state.effective_prompt().to_string()),
            error: state.request.error.clone(),
            ok: Some(state.effective_ok().to_string()),
            cancel: Some(state.effective_cancel().to_string()),
            notok: state.request.notok.clone(),
            repeat: state.request.repeat_passphrase.clone(),
            repeat_error: state.request.repeat_error.clone(),
            quality_bar: state.request.quality_bar.clone(),
            keyinfo: state.request.keyinfo.clone(),
            owner,
            timeout: state.options.timeout_secs,
        }
    }
}

impl Backend for DmsBackend {
    fn get_pin(&mut self, state: &State) -> Result<PinResponse, AssuanError> {
        let request = self.build_request(state, "getpin");
        let reply = self.prompt(&request)?;

        match reply.kind.as_str() {
            "pin" => pin_from_reply(reply, state),
            "cancel" => Err(AssuanError::canceled()),
            "error" => {
                if let Some(msg) = reply.message {
                    eprintln!("dank-pinentry: plugin error: {msg}");
                }
                Err(AssuanError::general("plugin reported an error"))
            }
            other => {
                eprintln!("dank-pinentry: unexpected reply type {other:?}");
                Err(AssuanError::general("unexpected reply"))
            }
        }
    }

    fn confirm(
        &mut self,
        state: &State,
        options: ConfirmOptions,
    ) -> Result<ConfirmOutcome, AssuanError> {
        let kind = if options.one_button {
            "message"
        } else {
            "confirm"
        };
        let request = self.build_request(state, kind);
        let reply = self.prompt(&request)?;

        match reply.kind.as_str() {
            "confirm" => match reply.outcome.as_deref() {
                Some("confirmed") => Ok(ConfirmOutcome::Confirmed),
                Some("declined") => Ok(ConfirmOutcome::Declined),
                Some("cancelled") | Some("canceled") => Ok(ConfirmOutcome::Cancelled),
                _ => Err(AssuanError::general("missing outcome")),
            },
            "cancel" => Ok(ConfirmOutcome::Cancelled),
            "error" => {
                if let Some(msg) = reply.message {
                    eprintln!("dank-pinentry: plugin error: {msg}");
                }
                Err(AssuanError::general("plugin reported an error"))
            }
            other => {
                eprintln!("dank-pinentry: unexpected reply type {other:?}");
                Err(AssuanError::general("unexpected reply"))
            }
        }
    }

    fn flavor(&self) -> &'static str {
        "dank:dms"
    }
}

fn peer_is_same_user(stream: &UnixStream) -> bool {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };

    if rc != 0 {
        // Cannot verify: fail closed.
        return false;
    }

    cred.uid == unsafe { libc::getuid() }
}

fn pin_from_reply(reply: Reply, state: &State) -> Result<PinResponse, AssuanError> {
    let pin = reply
        .pin
        .ok_or_else(|| AssuanError::general("no pin field"))?;
    let mut secret = Secret::new();
    secret
        .push_str(&pin)
        .map_err(|_| AssuanError::general("passphrase too long"))?;
    // A plain String cannot be reliably wiped; drop it promptly to keep the
    // window small. See PROTOCOL.md.
    drop(pin);
    Ok(PinResponse {
        secret,
        // Only meaningful when gpg-agent asked for a repeat; claiming it
        // otherwise would suppress a confirmation gpg-agent never delegated.
        repeated: reply.repeated && state.request.repeat_passphrase.is_some(),
    })
}

/// Non-cryptographic; only pairs a reply with its request.
pub fn request_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{:x}{:x}", std::process::id(), nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_socket_is_not_available() {
        assert!(!DmsBackend::is_available(Path::new(
            "/run/user/definitely-missing/nope.sock"
        )));
    }

    #[test]
    fn request_ids_differ_between_calls() {
        let a = request_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = request_id();
        assert_ne!(a, b);
    }

    #[test]
    fn request_serialises_without_null_noise() {
        let backend = DmsBackend::new(PathBuf::from("/tmp/x.sock"));
        let state = State::new();
        let request = backend.build_request(&state, "getpin");
        let json = serde_json::to_string(&request).unwrap();

        assert!(json.contains("\"type\":\"getpin\""), "{json}");
        assert!(json.contains("\"v\":1"), "{json}");
        assert!(!json.contains("\"description\""), "{json}");
    }

    #[test]
    fn request_includes_decoded_description_and_owner() {
        let backend = DmsBackend::new(PathBuf::from("/tmp/x.sock"));
        let mut state = State::new();
        state.request.description = Some("line one\nline two".into());
        state.options.owner = Some(crate::assuan::state::Owner {
            pid: Some(1234),
            uid: Some(1000),
            host: Some("nixos".into()),
        });

        let request = backend.build_request(&state, "getpin");
        let json = serde_json::to_string(&request).unwrap();

        // Escaped, not a raw byte that would break the line framing.
        assert!(json.contains("line one\\nline two"), "{json}");
        assert!(json.contains("\"pid\":1234"), "{json}");
    }

    #[test]
    fn request_carries_the_configured_timeout() {
        let backend = DmsBackend::new(PathBuf::from("/tmp/x.sock"));
        let mut state = State::new();
        state.options.timeout_secs = 120;
        let json = serde_json::to_string(&backend.build_request(&state, "getpin")).unwrap();
        assert!(json.contains("\"timeout\":120"), "{json}");
    }

    #[test]
    fn reply_parsing_accepts_each_variant() {
        let pin: Reply = serde_json::from_str(r#"{"v":1,"type":"pin","pin":"hunter2"}"#).unwrap();
        assert_eq!(pin.kind, "pin");
        assert_eq!(pin.pin.as_deref(), Some("hunter2"));

        let cancel: Reply = serde_json::from_str(r#"{"v":1,"type":"cancel"}"#).unwrap();
        assert_eq!(cancel.kind, "cancel");

        let confirm: Reply =
            serde_json::from_str(r#"{"v":1,"type":"confirm","outcome":"declined"}"#).unwrap();
        assert_eq!(confirm.outcome.as_deref(), Some("declined"));
    }

    #[test]
    fn reply_parsing_tolerates_extra_fields() {
        let reply: Reply =
            serde_json::from_str(r#"{"v":1,"type":"pin","pin":"x","futureThing":42}"#).unwrap();
        assert_eq!(reply.kind, "pin");
    }

    fn pin_reply(json: &str) -> Reply {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn repeated_is_reported_when_gpg_agent_asked_for_a_repeat() {
        let mut state = State::new();
        state.request.repeat_passphrase = Some("Repeat:".into());
        let pin = pin_from_reply(
            pin_reply(r#"{"v":1,"type":"pin","pin":"s3cret","repeated":true}"#),
            &state,
        )
        .unwrap();
        assert!(pin.repeated);
        assert_eq!(pin.secret.as_bytes(), b"s3cret");
    }

    #[test]
    fn repeated_is_ignored_when_no_repeat_was_asked_for() {
        let pin = pin_from_reply(
            pin_reply(r#"{"v":1,"type":"pin","pin":"x","repeated":true}"#),
            &State::new(),
        )
        .unwrap();
        assert!(!pin.repeated);
    }

    #[test]
    fn a_single_entry_answer_to_a_repeat_request_is_not_repeated() {
        // The in-bar prompt answers repeat requests with one field; gpg-agent
        // must then ask again itself.
        let mut state = State::new();
        state.request.repeat_passphrase = Some("Repeat:".into());
        let pin = pin_from_reply(pin_reply(r#"{"v":1,"type":"pin","pin":"x"}"#), &state).unwrap();
        assert!(!pin.repeated);
    }
}
