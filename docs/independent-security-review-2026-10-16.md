# Independent defensive security review

Reviewed release: `ef27d4358d721a1186592bdcd0ca8b2b3346c73e` in CeauYoo/mcp-memory-ledger.
Date: 2026-10-16 UTC. This is a focused engineering review, not a security certification.

## Scope and threat model

Read-only source review of workspace registration, request authorization/revocation, local IPC, owner dashboard credentials, native backup/restore, legacy adoption, file paths, diagnostic privacy, and selected dependency advisories. Tests used only synthetic local databases. No real user data, credentials, model requests, external targets, deployment or remote writes were used. Same-OS-user processes are explicitly **not** mutually isolated tenants. Privileged OS administrators are also outside the intended boundary.

## Findings against the released baseline

### F1 — High priority: database lifecycle creates broadly readable copies

Evidence: `src/adapters/sqlite/lifecycle.rs:227-235,287-296,949-987`.

- `init` creates its file without Unix mode 0600.
- `VACUUM INTO` creates migration backups without a private-mode reservation.
- Migration copies that backup into a bare file directly beneath the system temporary directory, rather than a private directory.
- With umask 022, a synthetic 0600 input produced a 0644 migration backup. A freshly initialized database was also 0644. The rehearsal copy inherits backup permissions; on a shared temporary directory this exposes content even if the original database lives in a private directory. Process termination may leave that copy behind.

Evidence logs: `project-binary-modes.txt`, `file-modes.txt`. The application binary used was prebuilt, SHA256 `86fde6cfe45272d8ecf0a9f20f71b627ae2873b95f5a9d92f11f95d70b412a00`; its depfile identifies the adoption checkout, and relevant lifecycle/config sources were byte-identical to the pinned release. This is source-correlated confirmation, not a fresh reproducible build of the entire release.

Remediation: create sensitive files privately from the first byte; use an explicitly 0700 temporary directory on Unix and a protected inheritable current-user DACL on Windows; include all SQLite sidecars; publish backups without replacing any existing path. Sync the backup and destination directory before changing the original where supported. Regressions must inspect modes under umask 022 and cover failed migration cleanup. Do not change permissions on arbitrary existing user directories.

The separate native `backup`/`restore` implementation is distinct: its snapshot is chmod 0600 before data is copied, with integrity/schema checks and no-clobber hard-link publication. Its baseline tempfile directory was not guaranteed 0700, so explicit private-directory hardening is still appropriate.

### F2 — High priority data integrity: lifecycle checks a different pathname than SQLite opens

Evidence: `src/adapters/sqlite/lifecycle.rs:207-236,1023-1037` and SQLx 0.8.6 URI parsing.

The lifecycle helper manually strips `sqlite://` and query text without percent decoding; SQLx percent-decodes the filename. For an existing synthetic `synthetic source.sqlite`, `init` with the valid encoded URL ending `synthetic%20source.sqlite` reserved a new empty literal-percent filename but migrated the existing decoded database. It returned success, no backup, and `not_applicable_new_database`. Thus the exclusive-create check does not protect the file being mutated, bypassing both the init refusal contract and migration backup/rehearsal contract.

Evidence: `encoded-path-init.txt`. Only synthetic data was used; the original database bytes changed and a zero-byte literal-percent alias was left behind.

Remediation: derive every file check/create from the same parsed `SqliteConnectOptions` filename used by the connection; reject non-file/memory modes; encode generated SQLite URLs correctly, including literal percent, question marks, hashes, Unicode and platform-specific separators. Regression: init on an existing encoded-path database must fail and leave its bytes unchanged, without creating an alias file.

### F3 — Dependency maintenance risk: pinned TLS implementation predates security fixes

`Cargo.lock:1683-1686` pins rustls 0.23.37. It falls within [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html), patched in 0.23.45. This concerns acceptance of TLS 1.3 handshake messages at an incorrect encryption level. The transcript remains authenticated; this is not evidence of arbitrary server impersonation.

The October 2026 group is patched in 0.23.46, including [RUSTSEC-2026-0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html) and [RUSTSEC-2026-0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html). Reachability differs: no custom session-store secret Debug path was established, and the pinned ring provider does not establish the post-quantum downgrade preconditions. Upgrade a compatible patch and rerun provider/TLS offline tests. Offline workspace core tools do not issue model requests; HTTPS provider modes are the relevant network surface.

A complete current cargo-audit/OSV scan was **not** performed. This selected advisory review cannot establish that every dependency is free of advisories. The rmcp 0.5 HTTP DNS-rebinding advisory [RUSTSEC-2026-0189](https://rustsec.org/advisories/RUSTSEC-2026-0189.html) explicitly excludes non-HTTP transports; the reviewed core uses local IPC, and its separate dashboard validates Host/Origin itself. It is not reported as an exploitable core finding. SQLx 0.8.6 and libsqlite3-sys 0.30.1 are above the fixed ranges of the older RustSec advisories checked.

## Controls reviewed without a confirmed baseline bypass

- Registry schema/size bounds, canonical exact-root binding, stable workspace identity, unique project scopes, private registry replacement and serialized updates.
- Request authorization is applied before legacy DTO dispatch; unknown tools/fields are denied; nested namespace/owner values cannot broaden authority. Personal writes require read and write grants. Global reflection/self-model operations are absent from the allowed inventory.
- Existing sessions reload root/identity/namespace/grants on every tool call and tool listing. Revocation rejects future admissions; already admitted work drains rather than being retroactively rolled back.
- Unix socket directory ownership/mode, socket owner and peer UID, bounded handshake/session/request/frame budgets, fail-fast admission, cancellation isolation and lifetime barriers.
- Dashboard is opt-in loopback, bearer-header only, constant-time digest comparison, Host/Origin enforcement, no-store headers, and token removal before downstream handling. Token file is bounded and checked for private ownership and aliases. Browser token storage is tab memory rather than URL/cookies/storage.
- Scoped session audits contain static operation names and binding metadata, not raw model input or error payloads. Durable receipts remain sensitive ledger data; content-free session telemetry does not imply the entire database is content-free.
- Native backup/restore uses SQLite online backup, checks integrity/current schema, returns a checksum, and publishes to a new path. Registry/configuration are expressly separate.
- Baseline adoption remains a deliberately conservative basic-fact slice: advanced histories anywhere block; supported graph edges and feedback incoming references are checked without rewriting data; execution repeats inventory inside a writer reservation and cooperating-core lock. Owner must retire old standalone writers.

## Validation limits

Existing prebuilt test executables gave policy 19, registry 14, dashboard HTTP 6, and lifecycle 20 passes. These are supplemental regression observations, not exact-commit rebuild results. All 11 local-core integration tests stopped at start_core with OS `EPERM`; they did not reach their intended assertions. No workaround or repeat attempt was used. Native Windows ACL/pipe behavior has not been exercised by this reviewer. CI success is not substituted for security review.

No fuzzing, exhaustive concurrency proof, malicious-database parser audit, full transitive dependency scan, external penetration test, or forensic analysis was performed. Existing plaintext data and backups require owner-managed retention and OS protections; the tool is not an encryption-at-rest system or a same-user tenant sandbox.

## Frozen delta review

History, Windows and lifecycle changes below are reviewed separately from the released baseline. The integrated-source conclusion at the end identifies the combined candidate; these findings do not imply that an existing installed release has been updated.

### Privacy/lifecycle repair delta — source reviewed

Reviewed `privacy-final.patch` SHA256 `8493ef534eb5659ef521b4b45bcd5e4cc276a54360a713159836e817a8b69a16`. Lifecycle source SHA256 `ee3fe5f1a6cc71e81735c25b13c66b6dc77526ce010d490516a40be9979dd10c`; lifecycle tests `153fea441e107b94638dbabcee489ff7ab1fcffd86f52de87d4084c346fde057`.

F1 and F2 root causes are corrected in this delta: private creation, explicitly private staging/rehearsal directories, no-clobber backup publication, Unix parent-directory synchronization before mutation, one decoded filename for lifecycle checks/SQLite, safe internally generated URLs, and rejection of memory/nested file-URI ambiguity. Existing source permissions are not silently changed. rustls is updated to 0.23.46 and rustls-webpki to 0.103.15, addressing the selected version findings in F3.

The author executed 22 lifecycle tests, 2 private-file unit tests, the final expanded encoded-path regression, 34 offline provider tests and strict Clippy successfully. Their logs were inspected; this reviewer did not independently rebuild that patch. Windows helpers are integrated separately and remain subject to Windows delta/native verification. Approval is limited to this source delta, not a declaration that an unpublished integrated tree or existing installed release has been repaired.

The reviewer additionally reran the frozen repair's existing test executables independently: encoded-path regression passed; permission regression passed in separate processes under umasks 000, 022 and 077; private-directory cleanup and no-clobber publication unit tests both passed. Evidence: `independent-repair-regressions.log`; executable hashes: `repair-test-binaries.sha256`. This independently re-executes the authored regressions, without claiming an independent clean build or a final integrated-tree test.

### Windows transport delta — static review, native gate still required

Reviewed `windows-core-final.patch` SHA256 `f4f3b77a95960bb0cb4f7b3e08290cd014892c77cb9b63e0c51816e3b18e0ce5`.

Shared request authorization, framing budgets and shutdown draining remain in the transport-independent runtime. Windows adds a current-user protected named-pipe DACL, remote-client rejection, first-instance exclusion, process-token SID verification in both directions, bounded connection establishment, owner/private-file checks against handles, file identity checks, non-delete-sharing directory/lock guards, and an owner-only staging-directory implementation. Existing SYSTEM/Administrators permissions are allowed explicitly because privileged administrators are outside this boundary; broad Users/Everyone allow entries, NULL DACLs, unknown ACE types and wrong owners fail closed.

Two issues identified during review were corrected before this freeze:
- Parent-directory validation now checks inherit-only broad allow ACEs as well as permissions effective on the directory. Otherwise SQLite sidecars could inherit access unavailable on the original private database.
- Drive-letter syntax alone was insufficient to establish locality. The implementation and Windows verification script now check drive type before metadata access/output creation and reject mapped remote drives as well as UNC paths.

No further blocking source defect was established in this focused delta. This is **not Windows runtime acceptance**. Author evidence consists of Linux shared-runtime tests/Clippy, Python packaging/verifier contract tests, and MSVC-target typechecking/Clippy of the Windows helper/adapter/test subset. Actual Windows named-pipe connection, ACL inheritance, second-user denial, process-exit/restart and filesystem locking behavior still require native evidence from the final integrated candidate. The new verifier is structured to use isolated synthetic state and only terminate child handles it created; typechecking or Linux verifier unit tests cannot substitute for running it on Windows.

### Advanced-history adoption delta — source review

Reviewed `history-final.patch` SHA256 `1d34b0f03b2d829da5bb77c7fe941e66ea057f0cb55425cccc1f14235b18b12d`.

The inventory now traverses undirected provenance closure seeded by the candidate namespace, including foreign incoming references, all candidate versions and normalized/payload edges. Missing endpoints, cross-scope/owner attribution, payload/row disagreement, unknown JSON fields and unsupported global self-model histories are rejected. Version continuity, status changes, revision/rollback snapshots and feedback commit references are validated. Inventory is read-only and execution still revalidates within the writer reservation.

Review feedback resulted in bounded projection rather than SELECT *, aggregate byte checks for every loaded provenance field, dynamic SQLite type/null checks (including feedback BLOB rejection), stronger feedback/candidate state invariants, and refusal of unknown views/triggers or changed known trigger definitions. These are supported-schema and provenance checks, not proof of semantic truth or trustworthiness of remembered text.

The author reports 92 focused tests, strict all-target/all-feature Clippy and formatting checks passed, plus the Windows-cfg-compatible adoption set. This reviewer inspected the source and relevant test cases; the final integrated tree remains a separate verification gate. No new blocking source defect was established in this focused delta.


## Final integrated-source conclusion

The integrated candidate was reviewed against baseline `ef27d4358d721a1186592bdcd0ca8b2b3346c73e` plus the three frozen patches identified above. The 33 changed source, test, verification-script, workflow and Cargo files are recorded in `integrated-source-manifest-pre-windows-ci-fix.sha256`; that historical manifest has SHA256 `41049a89edeb786fea1255419336a19dc58986b0a334eae93f224d30e88cde96`. Each implementation file matched its reviewed author tree. The expected Cargo union was checked separately for the URL dependency, Windows APIs and TLS patch upgrades. `git diff --check` passed. Documentation changes are outside that implementation manifest.

F1 and F2 have reviewed source fixes in this combined candidate. The selected affected dependency versions in F3 are updated. No additional blocking implementation defect was established by the final merge comparison. This conclusion is bounded by the stated coverage: it is not an exhaustive vulnerability assessment or a guarantee of isolation from processes running as the same OS user.

The integrator reports the final combined regression outcome as 834 of 846 distinct Rust tests passed, with 12 local-core tests blocked at AF_UNIX core startup by OS `EPERM`; they did not reach their intended assertions. An initial build-directory symlink fixture conflict was corrected without source changes before rerunning affected suites. The reviewer inspected `integrated-rust.log` and `integrated-rust-rerun.log`, including the startup denial. The integrator also reports 125 Python tests, 9 Node tests, 33 project-status checks, formatting, strict Clippy and diff checks passed. This is not a full Rust pass, and IPC integration remains unverified in this environment. Windows native execution remains an explicit release gate: compile/typecheck and Linux tests do not establish real Windows ACL, named-pipe, child termination/restart or filesystem-lock behavior. The claimed Windows support level must remain provisional until that evidence exists. The release owner must bind subsequent native results to the exact published candidate identity.

Recommended follow-through: preserve the encoded-path/no-clobber and multi-umask privacy regressions; retain cross-namespace incoming-edge, malformed-type and schema-object refusal tests; run a current complete dependency advisory scan; and require native Windows verification for the exact release candidate. Existing broadly readable backups or abandoned rehearsal copies created by older releases are not retroactively repaired by this patch. Owners should review their retention and permissions without exposing those contents in diagnostic logs.


## Historical addendum: Windows CI corrective-delta review

Date: 2026-10-16 UTC. Read-only review of the corrective working-tree delta against the staged integrated candidate identified by the release team as ea7dde3. The local checkout HEAD remains the earlier baseline; the staged `windows_security.rs` SHA256 is `27ba13468948b1bc1616772589ad5f96d951569a9467c445ed66562a16bfd24c`. This note does not claim a newly published commit identity or a native execution result.

## ACL failure diagnosis

The failing test expected an OA entry with no object GUIDs for the current owner to be rejected. Microsoft documents that ConvertStringSecurityDescriptorToSecurityDescriptorW normalizes this representation into ACCESS_ALLOWED_ACE_TYPE. Acceptance of the resulting ordinary current-user allow entry matches the intended policy; this failure does not establish that the validator accepts actual object ACEs.

Source: https://learn.microsoft.com/en-us/windows/win32/api/sddl/nf-sddl-convertstringsecuritydescriptortosecuritydescriptorw

The corrective test keeps a normalization positive case and adds object GUID, inherited-object GUID, and both-GUID cases. It asserts the parsed ACE type and requires the specific unsupported-entry error from the validator. Thus an SDDL conversion failure cannot satisfy the negative test accidentally. The production ACL validator is unchanged. Native execution of these tests is still required.

## SQLite filename correction

The corrective shared sqlite_file_url helper encodes OS filename bytes, including percent signs and Windows backslashes/question marks, instead of changing the canonical extended-length path prefix. Config, lifecycle and backup inspection now use the same conversion. This also removes raw-path URL construction in affected fixtures. The review found no new blocking source defect in this focused change.

Round-trip parsing is supplemented by a synthetic canonical-path init followed by a separate filename-based read. Final native evidence should include actual lifecycle migration and backup/restore, since preserving SQLx's parsed filename alone does not establish SQLite's filesystem behavior. The original report's native Windows gate remains open until the exact corrective release candidate passes those checks.

The frozen correction additionally makes the existing encoded-path lifecycle regression use a canonical parent for real init/migrate and backup-file checks, and adds the existing real backup/restore integration suite to the Windows CI job. These strengthen coverage but do not establish a successful native rerun. The author reports Linux lib 47, backup 7, lifecycle 22 and adoption 21 tests plus strict Clippy passed; these are author-reported corrective-branch results. At this addendum, the corrective Windows CI has not passed and the original native gate remains open.

The first corrective combined implementation manifest is retained as `integrated-source-manifest-windows-ci-fix1.sha256`, SHA256 `58301ffd60451293ee0fd91b69d82b749e31bc7119ce29657f51e6584a4c5628`. The pre-correction manifest is retained under its historical filename above. This addendum reviews the source correction; release automation must associate subsequent CI evidence with the exact published corrective commit.


## Historical addendum: second Windows CI corrective delta

Reviewed `windows-ci-fix2.patch` SHA256 `2980ffe749228e5146b5e5ba9f837b45229c93d6eb40823a29345427e6c9ee1b`, identified by the release team as a four-file correction after candidate `23ec301`. The patch hash was independently checked and the current integrated source inspected.

Backup path traversal now postpones filesystem inspection of a Windows Prefix component until its RootDir separator is appended; it also rejects drive-relative paths. Inspection of the complete root and subsequent components remains in place, including symlink rejection and parent-component refusal. The Windows-specific regression checks a real canonical directory, a new output filename, drive-relative input and a parent-component input.

Backup and migration flushes reopen their existing completed snapshots with read/write access, without create or truncate flags. This follows the documented Windows FlushFileBuffers requirement for GENERIC_WRITE: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers . Private staging/file creation, validation before publication and atomic no-clobber hard-link publication remain unchanged. Flush errors still stop publication/migration; the correction does not claim Windows directory-fsync durability. The bootstrap changes compare SQLx-decoded filenames, including literal percent/space cases, rather than asserting a particular URL spelling; they do not alter production config behavior.

No new blocking source defect was established in this focused correction. The author reports 56 Linux bootstrap/backup/lifecycle tests plus strict Clippy and formatting passed. This reviewer did not independently rerun those tests. The new Windows-specific regression and corrective native workflow remain pending; neither this addendum nor Linux results establish Windows acceptance.

The manifest for this correction is retained as `integrated-source-manifest-windows-ci-fix2.sha256`, SHA256 `94ca9fb3aadbafc021df7346ee7684cd9ad7a94fc2662dcc0de53508dd103fa6`. Both earlier manifests remain available under their historical filenames. Subsequent native results must be tied to the exact published corrective commit.


### Release verification note: lexical traversal fixtures

The subsequent native run passed 52 library tests but exposed a fixture that used PathBuf::join after a Windows verbatim prefix; this normalized away the intended parent component before validation. The corrective delta changes only two test fixtures to append raw OS text and assert an actual ParentDir component before checking rejection. Production behavior is unchanged. One Linux legacy restore fixture and formatting/diff checks passed; native Windows execution remains pending. This release-verification note is not an additional independent security review. The refreshed full-file source/test manifest is `integrated-source-manifest.sha256`, SHA256 `d5c3a664ec9339813d77c28d44e1c3027dcd51e99a092d761ac684b9874c8916`.


## Historical addendum: bounded package-startup diagnostics

Reviewed `windows-ci-fix8-diagnostics.patch`, SHA256 `67c3db6102434f8f209c2f690bf7198773663a67b5ea34889ebc08b8b24bd29c`. Production core behavior is unchanged. The verifier reads at most 65,537 bytes to detect truncation, then matches only the first 65,536 bytes against fixed category strings and a fixed numeric OS-error allowlist. Summary output contains no stderr substrings: only those fixed codes, the internally fixed process label, exit status, and truncation flag. Unknown content becomes `unclassified_startup_failure`; unreadable stderr becomes `stderr_unavailable`. The caller still raises `core_exited_before_ready`, so diagnostic classification cannot turn failure into success. These substring categories are diagnostic hints, not authenticated root-cause proof. Existing private local log files remain separate from this sanitized summary.

No new data-disclosure or fail-open defect was established in this focused source review. The author reports 13 verifier contract tests passed, including synthetic secret/path exclusion. This review does not establish candidate-package success; native package startup remains unresolved pending new CI evidence. Current implementation-manifest SHA256: `d68ea36956c1cd94e6896602de22e51fd851146d000ff18bd12f51cc81b6a4cf`.


## Historical addendum: synthetic Windows working-directory ACLs

Reviewed fix9 patch SHA256 `474aee1f0ace3e0f118c7cb8ace7f9f8009be5042f3c7e6304e17101a8ff15ab`. CPython 3.13's Windows mkdir implementation for mode 0700 explicitly creates a protected DACL with SYSTEM, Administrators and OWNER RIGHTS trustees, confirmed in its source: https://github.com/python/cpython/blob/3.13/Modules/posixmodule.c#L5288-L5298 . This differs from inheriting the fixture parent's explicit current-user SID and explains the reported ancestor-trustee rejection; it does not justify relaxing production ACL policy.

The corrective hook applies the existing private-output helper to eight fixed direct child names under the newly reserved synthetic output directory, before config/database creation or child processes. The helper requires new-directory creation with exist_ok=False and checks emptiness before setting the explicit current-user protected inheritable ACL. Existing user directories and production trustee validation are unchanged; Unix still uses its previous mode0700 creation. Within the declared same-OS-user trust model, no new blocking scope or data-protection defect was established. Author-reported 31 Python tests are not a native package acceptance result; that gate remains pending. Current implementation-manifest SHA256: `248785e9541f15221bbe5b60fbcd9aaeca285782aaa73d0cdd93fb4dc0a28e4b`.
