//! Candidate-only, knowledge-only Codex result feedback. No execution permissions.
mod cli;
mod protocol;
mod runtime;
mod state;
#[cfg(test)]
mod tests;
mod transport;
pub use cli::try_run_cli;
