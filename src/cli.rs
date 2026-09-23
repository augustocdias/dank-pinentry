//! The flags upstream pinentry accepts are the argv equivalents of the
//! corresponding `OPTION`s, so they seed initial state and are overridden by
//! whatever the agent sends later.
//!
//! Parsing is permissive: a pinentry that refuses to start over an unknown
//! flag locks the user out of their keys.

use clap::Parser;

use crate::assuan::state::{Options, Owner};
use crate::config::UiPreference;

#[derive(Debug, Parser)]
#[command(
    name = "dank-pinentry",
    version,
    about = "Ask securely for a secret, speaking the Assuan protocol used by gpg-agent",
    long_about = "Ask securely for a secret and print it to stdout, speaking the Assuan \
protocol used by gpg-agent.\n\n\
Frontends:\n  \
tty    Inline masked prompt on the terminal named by OPTION ttyname\n  \
dms    A dialog rendered by the DankMaterialShell plugin\n\n\
By default the terminal is used whenever one is usable, and the shell plugin \
otherwise.",
    // Tolerate flags we do not model rather than exiting non-zero.
    disable_help_flag = false,
    arg_required_else_help = false
)]
pub struct Cli {
    /// Turn on debugging output.
    #[arg(short = 'd', long = "debug")]
    pub debug: bool,

    /// Set the X display.
    #[arg(short = 'D', long = "display", value_name = "DISPLAY")]
    pub display: Option<String>,

    /// Set the tty terminal node name.
    #[arg(short = 'T', long = "ttyname", value_name = "FILE")]
    pub ttyname: Option<String>,

    /// Set the tty terminal type.
    #[arg(short = 'N', long = "ttytype", value_name = "NAME")]
    pub ttytype: Option<String>,

    /// Set the tty LC_CTYPE value.
    #[arg(short = 'C', long = "lc-ctype", value_name = "STRING")]
    pub lc_ctype: Option<String>,

    /// Set the tty LC_MESSAGES value.
    #[arg(short = 'M', long = "lc-messages", value_name = "STRING")]
    pub lc_messages: Option<String>,

    /// Timeout waiting for input, in seconds.
    #[arg(short = 'o', long = "timeout", value_name = "SECS")]
    pub timeout: Option<u32>,

    /// Grab the keyboard only while the window is focused.
    #[arg(short = 'g', long = "no-global-grab")]
    pub no_global_grab: bool,

    /// Parent window ID, used for positioning.
    #[arg(short = 'W', long = "parent-wid", value_name = "ID")]
    pub parent_wid: Option<i64>,

    /// Accepted for compatibility; both frontends take their own theming.
    #[arg(short = 'c', long = "colors", value_name = "STRING")]
    pub colors: Option<String>,

    /// Set the alert mode (none, beep or flash).
    #[arg(short = 'a', long = "ttyalert", value_name = "STRING")]
    pub ttyalert: Option<String>,

    /// Owner of the request, as `PID/UID HOST`.
    #[arg(long = "owner", value_name = "SPEC")]
    pub owner: Option<String>,

    /// Force a particular frontend, overriding the config file.
    #[arg(long = "ui", value_name = "auto|tty|dms")]
    pub ui: Option<String>,
}

impl Cli {
    /// `--help` and `--version` behave normally; unrecognised arguments are
    /// dropped with a note on stderr.
    pub fn parse_lenient() -> Self {
        match Self::try_parse() {
            Ok(cli) => cli,
            Err(err) => {
                use clap::error::ErrorKind;
                match err.kind() {
                    // Requests for output, not failures.
                    ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => err.exit(),
                    _ => {
                        eprintln!("dank-pinentry: {}", err.kind());
                        eprintln!("dank-pinentry: continuing with defaults");
                        Self::parse_from(["dank-pinentry"])
                    }
                }
            }
        }
    }

    pub fn ui_preference(&self) -> Option<UiPreference> {
        let raw = self.ui.as_deref()?;
        match UiPreference::parse(raw) {
            Some(pref) => Some(pref),
            None => {
                eprintln!("dank-pinentry: ignoring --ui={raw:?} (expected auto, tty or dms)");
                None
            }
        }
    }

    /// Later `OPTION` lines take precedence by overwriting these.
    pub fn to_options(&self) -> Options {
        let mut options = Options::new();

        if self.no_global_grab {
            options.grab = false;
        }
        options.display = self.display.clone();
        options.ttyname = self.ttyname.clone();
        options.ttytype = self.ttytype.clone();
        options.lc_ctype = self.lc_ctype.clone();
        options.lc_messages = self.lc_messages.clone();
        options.ttyalert = self.ttyalert.clone();
        options.parent_wid = self.parent_wid;
        options.owner = self.owner.as_deref().map(Owner::parse);

        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let mut argv = vec!["dank-pinentry"];
        argv.extend_from_slice(args);
        Cli::parse_from(argv)
    }

    #[test]
    fn no_arguments_is_valid() {
        let cli = parse(&[]);
        assert!(cli.ttyname.is_none());
        assert!(!cli.debug);
    }

    #[test]
    fn parses_long_and_short_ttyname() {
        assert_eq!(
            parse(&["--ttyname", "/dev/pts/3"]).ttyname.as_deref(),
            Some("/dev/pts/3")
        );
        assert_eq!(
            parse(&["-T", "/dev/pts/4"]).ttyname.as_deref(),
            Some("/dev/pts/4")
        );
    }

    #[test]
    fn parses_the_upstream_option_set() {
        let cli = parse(&[
            "--display",
            ":0",
            "--ttytype",
            "xterm",
            "--lc-ctype",
            "en_GB.UTF-8",
            "--lc-messages",
            "en_GB.UTF-8",
            "--timeout",
            "30",
            "--parent-wid",
            "12345",
            "--colors",
            "blue,black",
            "--ttyalert",
            "beep",
            "--no-global-grab",
            "--debug",
        ]);
        assert_eq!(cli.display.as_deref(), Some(":0"));
        assert_eq!(cli.ttytype.as_deref(), Some("xterm"));
        assert_eq!(cli.timeout, Some(30));
        assert_eq!(cli.parent_wid, Some(12345));
        assert!(cli.no_global_grab);
        assert!(cli.debug);
    }

    #[test]
    fn options_are_seeded_from_argv() {
        let options = parse(&["--ttyname", "/dev/pts/9", "--no-global-grab"]).to_options();
        assert_eq!(options.ttyname.as_deref(), Some("/dev/pts/9"));
        assert!(!options.grab);
    }

    #[test]
    fn grab_defaults_to_on() {
        assert!(parse(&[]).to_options().grab);
    }

    #[test]
    fn owner_is_parsed_into_structured_form() {
        let options = parse(&["--owner", "4242/1000 nixos"]).to_options();
        let owner = options.owner.expect("owner parsed");
        assert_eq!(owner.pid, Some(4242));
        assert_eq!(owner.uid, Some(1000));
        assert_eq!(owner.host.as_deref(), Some("nixos"));
    }

    #[test]
    fn ui_preference_maps_known_values() {
        assert_eq!(
            parse(&["--ui", "tty"]).ui_preference(),
            Some(UiPreference::Tty)
        );
        assert_eq!(
            parse(&["--ui", "dms"]).ui_preference(),
            Some(UiPreference::Dms)
        );
        assert_eq!(
            parse(&["--ui", "auto"]).ui_preference(),
            Some(UiPreference::Auto)
        );
    }

    #[test]
    fn unknown_ui_value_is_ignored_rather_than_fatal() {
        assert_eq!(parse(&["--ui", "wayland"]).ui_preference(), None);
    }

    #[test]
    fn absent_ui_flag_leaves_the_choice_to_config() {
        assert_eq!(parse(&[]).ui_preference(), None);
    }
}
