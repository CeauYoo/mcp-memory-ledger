# Deployment-path preview (T24 partial)

`codex-actions deployment-path-preview` reports five proposed **directory** placements: configuration, dependencies, database, logs and build output. It prints JSON to stdout and does not open, scan, create or change any target path. The target platform is mandatory (`macos`, `windows`, `linux`) and is not inferred from the process host. A Linux run can lexically review Windows paths; this is not native Windows validation.

Example, with synthetic paths only:

```text
agent_llm_mm codex-actions deployment-path-preview --target-platform macos --config-dir /local/ledger/config --dependencies-dir /local/ledger/dependencies --database-dir /local/ledger/database --logs-dir /local/ledger/logs --build-dir /local/ledger/build
```

The flags carry literal path strings, not shell commands. Quote spaces/backslashes according to the invoking shell. Environment variables, tilde expansion and unexpanded placeholders are not resolved by the preview. Supply all five flags and no extras. Missing/duplicate flags and unsupported target platforms fail the CLI; well-formed requests return a JSON report with exit status zero, including when placements are rejected. Consumers must inspect the report; successful command execution is not deployment approval.

## Results and limits

- Each placement has the original string (null when over the 4096-byte limit), original byte length, optional lexical normalization, `rejected` or `unknown` status and a reason. Aggregate status is `rejected` if any placement is rejected, otherwise `unknown`. There is deliberately no `safe`, `approved` or `verified` state.
- `ready_to_deploy`, `filesystem_verified`, `sync_location_verified`, and `installation_performed` are always false. A path without known sync markers is still unknown. A custom/renamed sync root, redirected home directory, symlink, junction or mount can defeat name-based heuristics.
- Conservative case-insensitive component markers reject Dropbox, Dropbox team directories, OneDrive and `OneDrive - ...`, iCloud Drive, `com~apple~CloudDocs`, `CloudStorage` and `Mobile Documents`. These are lexical heuristics, not proof of provider ownership; ordinary directories with these names can be rejected too. No exhaustive sync-provider detection is claimed.
- Relative paths, root-only paths, empty path segments, dot/traversal segments, controls and paths over 4096 UTF-8 bytes are rejected. Oversized strings are omitted, never silently truncated, from the report to bound output; accepted strings remain exact. Windows uses either slash spelling and requires an absolute drive path. Reserved Windows device names, trailing dots/spaces, alternate streams and forbidden characters are rejected. UNC, device namespaces and double-slash roots remain unknown with no normalization. POSIX case is preserved because the target volume's case behavior is not established.
- Unknown expansion-like components remain unknown rather than being evaluated. No path equivalence, collision, existence, permission or free-space claims are made. The report lists the missing target checks.

## Relationship to existing preview and rollback

This command lives beside the existing `codex-actions preview` / `restore-preview` commands and uses their existing CLI flag/JSON handling, but is deliberately independent of their host-specific checked writes. Existing configuration snapshots and CAS restore candidates remain the rollback mechanism; this new command does not generate an installer, run hooks, modify permissions, create deployment artifacts, or restore files. Windows hook-command preview and checked writes remain unsupported.

T24 is **partial**: deterministic read-only lexical review exists; verified non-synchronized placement on a real target, deployment/rollback execution and actual user-machine acceptance do not. Do not use this report as an install authorization or a security boundary. The project remains a technical demo/MVP.

## Verification

`cargo test --all-features --test deployment_path_preview` covers all five roles, explicit target semantics, Windows case/slashes/UNC/relative/traversal/reserved names, macOS provider markers and unknown custom roots, mandatory target selection, and a synthetic temporary-directory CLI run proving that proposed directories were not created. These tests do not inspect a user's filesystem or prove native target behavior.

Local validation on the assistant's Linux environment (2026-10-20): 11 focused deployment-path tests and 10 existing configuration preview/restore unit tests passed. All paths and data were synthetic. No native macOS/Windows filesystem inspection, deployment or full-suite result is claimed for this slice.
