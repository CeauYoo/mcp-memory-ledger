//! Isolated protocol acceptance of the installed-style stdio bridge and local core.
//! These synthetic tests do not establish natural-language client tool selection.
//! Unix sockets and Windows named pipes exercise the same runtime contract.
#[cfg(any(unix, windows))]
#[path = "local_core/runtime.rs"]
mod runtime;

#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;
