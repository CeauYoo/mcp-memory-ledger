# Windows hook preview: pinned contract assessment

T11 remains partial. This slice removes a misleading installable placeholder from context preview; it does not implement a Windows command renderer, install hooks, approve trust, change permissions, or support Windows checked writes.

## Evidence and precise blocker

Reviewed on 2026-10-20:

- [Official hooks guide](https://learn.chatgpt.com/docs/hooks) documents `commandWindows` as a Windows command-string override. An override is not a native argv interface or a shell selector.
- Pinned [Codex 0.159.2 command runner](https://github.com/openai/codex/blob/rust-v0.159.2/codex-rs/hooks/src/engine/command_runner.rs), `build_command` and `default_shell_program`: Windows defaults to the captured COMSPEC environment value, falling back to cmd.exe; it passes `/C` and a raw, outer-quoted command string. Explicit shell configuration instead supplies the program/arguments. A `/c` argument selects the raw-string branch. This is not PowerShell by definition.
- Pinned [HooksConfig and runtime wiring](https://github.com/openai/codex/blob/rust-v0.159.2/codex-rs/hooks/src/registry.rs) carry `shell_program` and `shell_args` separately from hooks-file content and capture the session environment. A hooks JSON file alone cannot establish the effective shell.

The current preview input is hooks JSON plus adapter paths, not the host's resolved shell configuration and captured environment. Therefore it cannot reliably reject shell overrides or verify the cmd.exe assumption. The public hook payload does not authenticate that information. This is a missing renderer/input/evidence contract, NOT a claim that Codex lacks Windows hooks.

A bounded cmd.exe-only candidate is a possible later implementation: explicitly establish effective shell/arguments and COMSPEC, reject overrides and expansion/metacharacter ambiguity, and test exact argv round trips on native Windows using the pinned raw-argument invocation. Include spaces, quotes, percent/exclamation expansion, ampersands, pipes, redirects, carets, Unicode, empty arguments and trailing backslashes, with unsafe cases rejected. The pinned default `/C` invocation does not disable shell AutoRun via `/D`; an isolated test using extra flags must not be called identical host behavior. Do not substitute POSIX or PowerShell quoting. Rendering tests would still not prove hook delivery, native permission enforcement, or supported checked writes.

## Output and backward-compatibility correction

`codex-context preview` on Windows/unknown platforms now keeps diagnostic `argv` unchanged, sets `platform_preview_supported` and `configuration_candidate_available` false, emits `unsupported_reason: target_shell_contract_and_argument_roundtrip_unverified`, and returns `hooks_json: null`. Consumers must check support before extracting configuration; the previous placeholder must not be installed. POSIX command quoting and lifecycle handlers remain unchanged; delivery remains unverified.

`codex-actions preview` remains an error on Windows/unknown targets, including when caller metadata claims a pinned host version or successful probe. It produces no candidate/backup/rollback bundle for those targets. Existing supported-platform candidate and CAS rollback behavior is unchanged. No Windows target-file backup or replacement is claimed.

## Verification

The existing Windows CI `--lib` and `--test codex_context_adapter` selections already include the changed tests; no target-list addition is required. Pure tests exercise Windows/unknown output on Linux with adversarial argv and confirm no command handler is produced. POSIX quoting is checked byte-for-byte. Integration coverage verifies native-platform output and that preview creates no state/config file. Action-preview regression tests retain rejection despite shell-looking strings and claimed host evidence.

Local Linux verification on the unchanged frozen base `88d20c793e27b07ffff02aca68d07fe60f538f03` plus this slice: `cargo test --offline --lib` passed 115 tests; `cargo test --offline --test codex_context_adapter` passed 14 tests; `cargo clippy --offline --lib --test codex_context_adapter -- -D warnings`, formatting and the 33-item status-sync check passed; 222 relative documentation links resolved. Builds used the dedicated `build-windows-preview` target with debug information and incremental compilation disabled. Native Windows execution and real Windows host acceptance remain unverified. No user configuration, trust, credentials, or device was accessed or changed.
