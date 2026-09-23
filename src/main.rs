//! gpg-agent speaks Assuan over stdin/stdout, so nothing may be printed there
//! outside the protocol. Diagnostics go to stderr, which gpg-agent logs.

use std::io::{self, BufReader};

use dank_pinentry::assuan::Server;
use dank_pinentry::assuan::state::State;
use dank_pinentry::backend::AutoBackend;
use dank_pinentry::cli::Cli;
use dank_pinentry::config::Config;

fn main() {
    let cli = Cli::parse_lenient();

    let mut config = Config::load();
    // --ui beats the environment, which beats the config file.
    if let Some(pref) = cli.ui_preference() {
        config.ui = pref;
    }

    let mut state = State::with_options(cli.to_options());
    // --timeout beats the config file; gpg-agent's SETTIMEOUT, which arrives
    // later over the protocol, beats both.
    if let Some(timeout) = cli.timeout.or(config.timeout) {
        state.options.timeout_secs = timeout;
    }

    let mut server = Server::with_state(AutoBackend::new(config), state);

    let stdin = io::stdin();
    let stdout = io::stdout();

    if let Err(err) = server.run(BufReader::new(stdin.lock()), stdout.lock()) {
        eprintln!("dank-pinentry: {err}");
        std::process::exit(1);
    }
}
