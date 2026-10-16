//! Codex lifecycle context adapter, isolated from execution authorization.
//! Only its command preview is generated; user/global host config is never edited.
pub mod artifacts;
mod cli;
pub mod protocol;
pub(crate) mod state;
pub(crate) mod transport;

pub use cli::try_run_cli;
