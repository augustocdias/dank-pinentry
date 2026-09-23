//! Settings the binary needs. How the prompt *looks* is owned by the DMS
//! plugin instead.
//!
//! Layered, later winning: defaults, `config.toml`, `DANK_PINENTRY_*`, then
//! command-line flags applied by the caller.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiPreference {
    /// Terminal when usable, otherwise the shell plugin.
    #[default]
    Auto,
    /// Always the terminal; fail if unavailable.
    #[serde(alias = "curses", alias = "terminal")]
    Tty,
    /// Always the shell plugin; fail if unavailable.
    #[serde(alias = "gui", alias = "dank")]
    Dms,
}

impl UiPreference {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "tty" | "curses" | "terminal" => Some(Self::Tty),
            "dms" | "gui" | "dank" => Some(Self::Dms),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ui: UiPreference,
    /// Defaults to `$XDG_RUNTIME_DIR/dms-pinentry.sock`.
    pub socket_path: Option<PathBuf>,
    pub mask_char: Option<char>,
    /// Seconds before an unanswered prompt cancels itself; 0 means never.
    /// gpg-agent's `pinentry-timeout`, when set, still takes precedence.
    pub timeout: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiPreference::Auto,
            socket_path: None,
            mask_char: None,
            timeout: None,
        }
    }
}

impl Config {
    /// Bad config degrades to defaults: a pinentry that refuses to start locks
    /// the user out of their keys.
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(config) => config,
            Err(err) => {
                eprintln!("dank-pinentry: ignoring bad configuration: {err}");
                Self::default()
            }
        }
    }

    fn try_load() -> Result<Self, config::ConfigError> {
        let mut builder = config::Config::builder();

        if let Some(path) = Self::config_path() {
            builder = builder.add_source(
                config::File::from(path)
                    .format(config::FileFormat::Toml)
                    .required(false),
            );
        }

        builder
            // No separator: the whole name after the prefix is the key, so
            // DANK_PINENTRY_SOCKET_PATH maps to `socket_path`. Setting a "_"
            // separator would instead read it as nested `socket.path`, which
            // matches no field and is silently ignored.
            .add_source(config::Environment::with_prefix("DANK_PINENTRY"))
            .build()?
            .try_deserialize()
    }

    pub fn config_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
        Some(base.join("dank-pinentry").join("config.toml"))
    }

    pub fn default_socket_path() -> PathBuf {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        base.join("dms-pinentry.sock")
    }

    pub fn socket_path(&self) -> PathBuf {
        self.socket_path
            .clone()
            .unwrap_or_else(Self::default_socket_path)
    }

    pub fn from_toml(text: &str) -> Result<Self, config::ConfigError> {
        config::Config::builder()
            .add_source(config::File::from_str(text, config::FileFormat::Toml))
            .build()?
            .try_deserialize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_auto() {
        let config = Config::default();
        assert_eq!(config.ui, UiPreference::Auto);
        assert_eq!(config.socket_path, None);
        assert_eq!(config.mask_char, None);
    }

    #[test]
    fn empty_config_is_all_defaults() {
        let config = Config::from_toml("").unwrap();
        assert_eq!(config.ui, UiPreference::Auto);
        assert_eq!(config.socket_path, None);
    }

    #[test]
    fn parses_ui_preference() {
        assert_eq!(
            Config::from_toml("ui = \"tty\"").unwrap().ui,
            UiPreference::Tty
        );
        assert_eq!(
            Config::from_toml("ui = \"dms\"").unwrap().ui,
            UiPreference::Dms
        );
        assert_eq!(
            Config::from_toml("ui = \"auto\"").unwrap().ui,
            UiPreference::Auto
        );
    }

    #[test]
    fn accepts_ui_aliases() {
        assert_eq!(
            Config::from_toml("ui = \"curses\"").unwrap().ui,
            UiPreference::Tty
        );
        assert_eq!(
            Config::from_toml("ui = \"terminal\"").unwrap().ui,
            UiPreference::Tty
        );
        assert_eq!(
            Config::from_toml("ui = \"gui\"").unwrap().ui,
            UiPreference::Dms
        );
    }

    #[test]
    fn rejects_unknown_ui_values() {
        // `load()` turns this into a warning plus defaults.
        assert!(Config::from_toml("ui = \"wayland\"").is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        // A typo would otherwise silently do nothing.
        let err = Config::from_toml("plaecment = \"top\"").unwrap_err();
        assert!(format!("{err}").contains("plaecment"), "{err}");
    }

    #[test]
    fn parses_socket_path() {
        let config = Config::from_toml("socket_path = \"/run/user/1000/custom.sock\"").unwrap();
        assert_eq!(
            config.socket_path,
            Some(PathBuf::from("/run/user/1000/custom.sock"))
        );
    }

    #[test]
    fn parses_mask_char() {
        assert_eq!(
            Config::from_toml("mask_char = \"#\"").unwrap().mask_char,
            Some('#')
        );
    }

    #[test]
    fn parses_timeout() {
        assert_eq!(
            Config::from_toml("timeout = 120").unwrap().timeout,
            Some(120)
        );
        assert_eq!(Config::from_toml("timeout = 0").unwrap().timeout, Some(0));
        assert_eq!(Config::from_toml("").unwrap().timeout, None);
    }

    #[test]
    fn comments_are_ignored() {
        let config = Config::from_toml("# a comment\nui = \"tty\" # trailing\n").unwrap();
        assert_eq!(config.ui, UiPreference::Tty);
    }

    #[test]
    fn socket_path_falls_back_to_the_runtime_dir_default() {
        let config = Config::default();
        let path = config.socket_path();
        assert!(path.ends_with("dms-pinentry.sock"), "{}", path.display());
    }

    #[test]
    fn explicit_socket_path_wins_over_the_default() {
        let config = Config::from_toml("socket_path = \"/tmp/explicit.sock\"").unwrap();
        assert_eq!(config.socket_path(), PathBuf::from("/tmp/explicit.sock"));
    }

    fn from_env(vars: &[(&str, &str)]) -> Config {
        let map: std::collections::HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();

        config::Config::builder()
            .add_source(config::Environment::with_prefix("DANK_PINENTRY").source(Some(map)))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn environment_maps_onto_snake_case_fields() {
        // Regression: a "_" separator reads SOCKET_PATH as nested
        // `socket.path`, which matches no field and is silently dropped.
        let config = from_env(&[
            ("DANK_PINENTRY_UI", "dms"),
            ("DANK_PINENTRY_SOCKET_PATH", "/tmp/from-env.sock"),
        ]);
        assert_eq!(config.ui, UiPreference::Dms);
        assert_eq!(
            config.socket_path,
            Some(PathBuf::from("/tmp/from-env.sock"))
        );
    }

    #[test]
    fn numeric_environment_values_parse() {
        // Environment values always arrive as strings.
        assert_eq!(
            from_env(&[("DANK_PINENTRY_TIMEOUT", "90")]).timeout,
            Some(90)
        );
    }

    #[test]
    fn environment_alone_yields_defaults_for_unset_keys() {
        let config = from_env(&[("DANK_PINENTRY_UI", "tty")]);
        assert_eq!(config.ui, UiPreference::Tty);
        assert_eq!(config.socket_path, None);
    }

    #[test]
    fn command_line_ui_values_map_the_same_way_as_config_values() {
        for value in ["tty", "curses", "terminal"] {
            assert_eq!(UiPreference::parse(value), Some(UiPreference::Tty));
        }
        for value in ["dms", "gui", "dank"] {
            assert_eq!(UiPreference::parse(value), Some(UiPreference::Dms));
        }
        assert_eq!(UiPreference::parse("AUTO"), Some(UiPreference::Auto));
        assert_eq!(UiPreference::parse("nonsense"), None);
    }
}
