//! The Assuan protocol server that gpg-agent talks to.
//!
//! `stdin`/`stdout` are the protocol pipes. Nothing else may be written to
//! them -- in particular the TTY frontend must open the device named by
//! `OPTION ttyname` rather than using stdout, or it will corrupt the stream.

pub mod codec;
pub mod error;
pub mod server;
pub mod state;

// Convenience re-exports for frontends. Not all are used by the binary yet,
// which is fine -- they are part of this module's surface.
#[allow(unused_imports)]
pub use error::AssuanError;
pub use server::Server;
#[allow(unused_imports)]
pub use state::{Options, Owner, Request, State};
