//! `dank-pinentry` -- a pinentry with a TTY frontend and a DankMaterialShell
//! plugin frontend.
//!
//! Split into a library and a thin binary so the protocol layer can be driven
//! from integration tests without spawning a process.

pub mod assuan;
pub mod backend;
pub mod cli;
pub mod config;
pub mod owner;
pub mod secret;
