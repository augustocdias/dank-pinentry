//! Attributing a prompt to the process that triggered it, so the UI can name
//! it. Anti-phishing: normally any process can make an indistinguishable
//! passphrase box appear.
//!
//! All best-effort; the owner often exits before the prompt is answered.

use std::path::PathBuf;

/// Caps a pathological argv before it reaches the dialog.
const MAX_COMMAND_LEN: usize = 200;

pub fn command_line(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(format_cmdline(&raw))
}

/// Fallback when the command line is empty (kernel threads, zombies).
pub fn process_name(pid: u32) -> Option<String> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = raw.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Pids are recycled, so a stale owner pid can point at an unrelated process.
pub fn belongs_to_uid(pid: u32, uid: u32) -> bool {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse::<u32>().ok())
        .map(|real| real == uid)
        .unwrap_or(false)
}

fn format_cmdline(raw: &[u8]) -> String {
    let text: Vec<String> = raw
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();

    let joined = text.join(" ");
    truncate(&joined, MAX_COMMAND_LEN)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn proc_path(pid: u32, entry: &str) -> PathBuf {
    PathBuf::from(format!("/proc/{pid}/{entry}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_nul_separated_arguments() {
        assert_eq!(format_cmdline(b"git\0commit\0-S\0"), "git commit -S");
    }

    #[test]
    fn ignores_empty_arguments() {
        assert_eq!(format_cmdline(b"a\0\0b\0"), "a b");
    }

    #[test]
    fn empty_cmdline_yields_empty_string() {
        assert_eq!(format_cmdline(b""), "");
    }

    #[test]
    fn long_command_lines_are_truncated() {
        let raw: Vec<u8> = "x".repeat(500).into_bytes();
        let formatted = format_cmdline(&raw);
        assert!(formatted.chars().count() <= MAX_COMMAND_LEN, "{formatted}");
        assert!(formatted.ends_with('…'));
    }

    #[test]
    fn truncate_is_char_aware() {
        let text = "üüüüü";
        assert_eq!(truncate(text, 3), "üü…");
    }

    #[test]
    fn reads_our_own_command_line() {
        let pid = std::process::id();
        let cmdline = command_line(pid).expect("own cmdline is readable");
        assert!(!cmdline.is_empty());
    }

    #[test]
    fn our_own_process_belongs_to_our_uid() {
        let pid = std::process::id();
        let uid = unsafe { libc::getuid() };
        assert!(belongs_to_uid(pid, uid));
    }

    #[test]
    fn a_bogus_pid_belongs_to_nobody() {
        assert!(!belongs_to_uid(u32::MAX, 1000));
        assert_eq!(command_line(u32::MAX), None);
    }

    #[test]
    fn proc_path_builds_expected_layout() {
        assert_eq!(
            proc_path(42, "cmdline").to_str().unwrap(),
            "/proc/42/cmdline"
        );
    }
}
