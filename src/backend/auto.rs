//! Picks a frontend at prompt time.
//!
//! Selection cannot happen at startup: `OPTION ttyname` arrives *after* the
//! process is running. The choice is cached so a session cannot flip frontends
//! midway.

use crate::assuan::error::AssuanError;
use crate::assuan::state::State;
use crate::backend::dms::DmsBackend;
use crate::backend::tty::TtyBackend;
use crate::backend::{Backend, ConfirmOptions, ConfirmOutcome, PinResponse};
use crate::config::{Config, UiPreference};

enum Chosen {
    Tty(TtyBackend),
    Dms(DmsBackend),
}

impl Chosen {
    fn as_backend(&mut self) -> &mut dyn Backend {
        match self {
            Chosen::Tty(b) => b,
            Chosen::Dms(b) => b,
        }
    }

    fn flavor(&self) -> &'static str {
        match self {
            Chosen::Tty(_) => "dank:tty",
            Chosen::Dms(_) => "dank:dms",
        }
    }
}

pub struct AutoBackend {
    config: Config,
    chosen: Option<Chosen>,
}

impl AutoBackend {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            chosen: None,
        }
    }

    /// An explicit preference is never silently overridden: asking for `tty`
    /// and getting a dialog is a surprising place to type a passphrase.
    fn resolve(&mut self, state: &State) -> Result<&mut Chosen, AssuanError> {
        if self.chosen.is_none() {
            let chosen = match self.config.ui {
                UiPreference::Tty => {
                    if TtyBackend::is_available(state) {
                        Chosen::Tty(Self::tty(&self.config))
                    } else {
                        eprintln!(
                            "dank-pinentry: ui=tty but no usable terminal ({})",
                            TtyBackend::device_path(state)
                        );
                        return Err(AssuanError::no_input());
                    }
                }
                UiPreference::Dms => {
                    let path = self.config.socket_path();
                    if DmsBackend::is_available(&path) {
                        Chosen::Dms(DmsBackend::new(path))
                    } else {
                        eprintln!(
                            "dank-pinentry: ui=dms but the plugin is not listening at {}",
                            path.display()
                        );
                        return Err(AssuanError::no_input());
                    }
                }
                UiPreference::Auto => Self::resolve_auto(state, &self.config)?,
            };
            self.chosen = Some(chosen);
        }

        Ok(self.chosen.as_mut().expect("just set"))
    }

    fn tty(config: &Config) -> TtyBackend {
        TtyBackend::with_mask(config.mask_char.unwrap_or('*'))
    }

    /// Terminal first, even in a graphical session: if the request came from a
    /// command you just typed, answering it there is least surprising.
    fn resolve_auto(state: &State, config: &Config) -> Result<Chosen, AssuanError> {
        if TtyBackend::is_available(state) {
            return Ok(Chosen::Tty(Self::tty(config)));
        }

        let path = config.socket_path();
        if DmsBackend::is_available(&path) {
            return Ok(Chosen::Dms(DmsBackend::new(path)));
        }

        // Report rather than hang; gpg-agent surfaces the error.
        eprintln!(
            "dank-pinentry: no usable frontend (tty={}, socket={})",
            TtyBackend::device_path(state),
            path.display()
        );
        Err(AssuanError::no_input())
    }
}

impl Backend for AutoBackend {
    fn get_pin(&mut self, state: &State) -> Result<PinResponse, AssuanError> {
        self.resolve(state)?.as_backend().get_pin(state)
    }

    fn confirm(
        &mut self,
        state: &State,
        options: ConfirmOptions,
    ) -> Result<ConfirmOutcome, AssuanError> {
        self.resolve(state)?.as_backend().confirm(state, options)
    }

    fn flavor(&self) -> &'static str {
        // No choice made yet before the first prompt.
        match &self.chosen {
            Some(chosen) => chosen.flavor(),
            None => "dank",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(ui: UiPreference) -> Config {
        Config {
            ui,
            socket_path: Some("/nonexistent/dank-pinentry-test.sock".into()),
            ..Config::default()
        }
    }

    #[test]
    fn flavor_before_selection_is_generic() {
        let backend = AutoBackend::new(config_with(UiPreference::Auto));
        assert_eq!(backend.flavor(), "dank");
    }

    #[test]
    fn explicit_tty_does_not_fall_through_to_a_dialog() {
        let mut backend = AutoBackend::new(config_with(UiPreference::Tty));
        let mut state = State::new();
        state.options.ttyname = Some("/dev/definitely-not-a-tty".into());
        assert!(backend.get_pin(&state).is_err());
    }

    #[test]
    fn explicit_dms_fails_when_the_plugin_is_absent() {
        let mut backend = AutoBackend::new(config_with(UiPreference::Dms));
        let state = State::new();
        assert!(backend.get_pin(&state).is_err());
    }

    #[test]
    fn auto_errors_when_nothing_is_usable() {
        let mut backend = AutoBackend::new(config_with(UiPreference::Auto));
        let mut state = State::new();
        state.options.ttyname = Some("/dev/definitely-not-a-tty".into());
        assert!(backend.get_pin(&state).is_err());
    }
}
