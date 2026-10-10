# Local portable archives

The repository can build a native executable archive from an exact committed
source tree and test the unpacked executable without Rust on its child process
PATH. This is a local build and installation simulation. It does not publish a
release, prove a real fresh-machine/user-client story, or approve Local Alpha.
The repository remains a technical demo / MVP.

## Build from committed source

The builder requires Git, Python 3.9 or later, the pinned Rust toolchain, native
linker/build prerequisites, and Cargo dependencies. The consumer needs no Rust
toolchain. Python is needed only to run the optional verification script.

Run from the repository root after committing the intended source locally:

```text
python3 scripts/portable-package.py build --candidate local-mvp-ci-rc.1 --source-ref HEAD --output target/portable-package --target-dir target
```

`--offline` prevents Cargo registry access and requires an already populated
cache. The command has no upload, release, tag, installer, service registration,
credential, or model-call step. It runs `cargo build --locked --release --bin
agent_llm_mm --target <native-target>` in an isolated `git archive` snapshot.
Dirty and untracked checkout files are excluded. Compiler/wrapper overrides are
cleared, Cargo compiler/wrapper settings are pinned for the command, and toolchain
versions are measured inside that committed snapshot. The running packager must
exactly match the script in that source commit; commit a changed script before
using it to create source-specific evidence.

The output directory must be absent or empty. Never reuse a successful evidence
directory. An exclusive hidden reservation prevents concurrent attempts from
sharing an output directory, and final files are created without overwrite. A
failed command may leave diagnostic or partial output and its reservation; it
does not write successful verification evidence.

Native build/verify pairs are:

| Host | Rust target | Archive suffix |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `linux-x86_64.tar.gz` |
| macOS Apple silicon | `aarch64-apple-darwin` | `macos-aarch64.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `macos-x86_64.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `windows-x86_64.zip` |

The script only builds for its actual host and rejects a mismatched Rust host.
A code path or green test on one platform does not establish runtime evidence
for any other platform. Archives use native system libraries; Linux glibc,
macOS deployment targets, and Windows runtime availability must still be checked
on the intended consumer system. There is no universal/static-binary promise,
code signing, notarization, or security-warning bypass.

## Files and traceability

For example, Linux output contains:

- `agent-llm-mm-local-mvp-ci-rc.1-linux-x86_64.tar.gz`
- `agent-llm-mm-local-mvp-ci-rc.1-linux-x86_64.build.json`
- `agent-llm-mm-local-mvp-ci-rc.1-linux-x86_64.sha256`

Each archive contains one named top-level directory, with:

- the real native executable under `bin/`
- `LICENSE` and `NOTICE`
- the conservative mock-provider example configuration
- this guide, database lifecycle guidance, and the MCP tool reference
- `BUILD-MANIFEST.json`

The manifest records the exact Git commit and tree, source commit timestamp,
Rust target and toolchain versions, build command, packager SHA-256, and every
payload file's size, mode and SHA-256. The external build receipt pins the
archive size/hash and internal manifest hash. The archive's ordered entries,
permissions, owner fields and timestamps, JSON ordering, and checksum format
are deterministic for the same payload. This does not claim byte-identical
compiler output across machines/toolchains or signed supply-chain attestation.
A checksum received alongside a modified artifact is not independent proof of
authenticity; obtain expected source identity and receipts from a trusted build.

## Verify an unpacked archive

Record the full expected source commit and tree independently, for example with
`git rev-parse HEAD` and `git rev-parse HEAD^{tree}` in the intended source
checkout. Supply both, plus the archive and its build receipt:

```text
python3 scripts/portable-package.py verify --archive target/portable-package/agent-llm-mm-local-mvp-ci-rc.1-linux-x86_64.tar.gz --receipt target/portable-package/agent-llm-mm-local-mvp-ci-rc.1-linux-x86_64.build.json --expected-commit FULL_COMMIT --expected-tree FULL_TREE --output target/portable-package-verify
```

Use the actual native archive filename on macOS/Windows. The verifier does not
need a source checkout or Cargo; the standalone script, trusted build receipt,
archive, Python standard library, and expected identities suffice.

Before executing anything, verification checks archive/member size limits,
source identity, archive checksum, exact payload inventory, every file hash and
mode, and the executable's native format/architecture. It rejects traversal,
absolute/Windows-drive paths, links, special files, duplicate paths, unexpected
payloads, missing assets, text placeholders, corrupt/truncated archives, and
cross-platform runtime tests.

It extracts into a new system temporary directory, creates a separate temporary
home/config/database, and runs the unpacked executable directly with an empty
PATH. Neither Cargo nor rustc is available through that PATH. Inherited provider
keys, database/config overrides, proxies, and preload settings are excluded.
The deterministic mock provider is used and the dashboard and daemon stay off.
This is environment isolation, not an OS network/security sandbox.

The simulation checks:

1. `init`, then `doctor --read-only`, including unchanged database bytes.
2. MCP initialize/tool discovery and scoped ingest in two namespaces.
3. Process restart, scoped recall and Claim provenance inspection.
4. A correction, current Claim recall, and revision-history readback.
5. SQLite backup, restore to a new path, content/integrity/foreign-key checks,
   then read-only doctor and identical MCP recall from the restored database.
   Only the synthetic smoke configuration is switched, never a real data path.

Successful output includes `init.json`, `doctor.json`, `doctor-restored.json`, `mcp-transcript.json`,
`summary.json`, and `SHA256SUMS`. The summary explicitly says
`real_fresh_machine_evidence = false` and `real_user_client_evidence = false`.
Temporary state is removed after verification. Failure never writes a passed
summary. This evidence must not be copied into real fresh-machine or real-user
client evidence slots.

## Consumer first run (without Cargo)

Check the trusted archive checksum before unpacking. Do not execute an archive
whose source or checksum you cannot establish. Unpack into a new folder, copy
`examples/agent-llm-mm.example.toml` to a separate private configuration file,
and set `database_url` to a new absolute SQLite path in a writable data folder.
Keep `provider = "mock"`, dashboard disabled, and daemon disabled for the first
run. First inspect and clear inherited `AGENT_LLM_MM_*` environment overrides,
particularly `AGENT_LLM_MM_DATABASE_URL` and provider settings: they can override
the configuration file. Set only `AGENT_LLM_MM_CONFIG` for these commands. Do not
reuse a real database for a package smoke test.

On macOS/Linux, from the unpacked directory:

```bash
export AGENT_LLM_MM_CONFIG="/absolute/path/to/agent-llm-mm.local.toml"
./bin/agent_llm_mm init
./bin/agent_llm_mm doctor --read-only
./bin/agent_llm_mm serve
```

On Windows, from the unpacked directory in PowerShell:

```powershell
$env:AGENT_LLM_MM_CONFIG = 'C:\absolute\path\agent-llm-mm.local.toml'
.\bin\agent_llm_mm.exe init
.\bin\agent_llm_mm.exe doctor --read-only
.\bin\agent_llm_mm.exe serve
```

`serve` speaks MCP over standard input/output. Register the absolute executable
path, argument `serve`, and `AGENT_LLM_MM_CONFIG` environment value with your
MCP client. Waiting for client input on stdio is expected. Client-specific setup
and the actual fresh-user acceptance story remain independent evidence gates.
For existing data, follow `docs/database-operations.md`; do not rerun `init` over
an existing database or assume unpacking a newer executable migrates it.

## Regression tests and release boundary

```text
python3 -m unittest discover -s tests -p 'test_portable_package.py' -v
```

Archive regression payloads are explicitly synthetic, never executed, and never
release evidence. Separate SQLite tests execute real local synthetic backup and
restore operations and check connection closure on success and errors. They exercise archive integrity, source mismatch, native
headers, deterministic archive output, unsafe extraction, environment isolation,
existing-output protection, and fail-closed evidence generation.

The verifier explicitly closes every SQLite source/destination connection before
restored process checks and temporary-directory cleanup, including on failure.
Transaction context exit alone does not close Python SQLite connections, which
can keep files locked on Windows.

The older `packaging-archive-evidence.sh` checks a fixed four-archive inventory;
it does not build or certify the payloads. Its broad `packaging-preflight-check`
also tracks installer/service-manager/auto-updater blockers. These are separate
from this per-platform portable archive path. Local Alpha does not require
installers, services or auto-updaters, but still requires its own fresh-machine,
platform, user-client, provider and human-decision gates.

## Observed native CI coverage (2026-10-10)

[Run 38035347484](https://github.com/yooyui/mcp-memory-ledger/actions/runs/38035347484) passed for exact source
`0ca8bccc64cadb3f1eae9d1a74bad602a1c60bc9`: Linux x86_64, macOS ARM64 and
Windows x86_64 each built and verified their native archive and completed the
no-Rust unpacked-binary workflow. Windows PowerShell 7.6.6 also passed the actual
12-test wrapper harness, including 20 native argument/environment combinations.
All three ran 40 package tests and 13 evaluation fixtures. Other architectures,
OS versions and real-user machines are not certified by that configured-host run;
every later source head requires new checks. No package was published as a release.
