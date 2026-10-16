use super::{
    preview, protocol, receipts,
    runtime::{self, Paths},
};
use crate::domain::action_check::{
    CONTRACT, CheckMode, Operation, OperatorGate, OperatorPolicy, TaskVersionMode,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub async fn try_run_cli(args: &[String]) -> Result<bool> {
    if args.first().map(String::as_str) != Some("codex-actions") {
        return Ok(false);
    }
    let action = args.get(1).map(String::as_str).unwrap_or("help");
    // All hook errors, including bad configuration arguments, produce a valid
    // deny if this executable is running. Never report a bare generic exit 1.
    if action == "hook" {
        let result = hook(&args[2..]).await;
        let output = match result {
            Ok(decision) => protocol::render(decision),
            Err(_) => protocol::unavailable(),
        };
        if write_json(&output).is_err() {
            eprintln!("Action check output unavailable; blocking current tool call.");
            std::process::exit(2);
        }
        return Ok(true);
    }
    if action == "help" {
        println!(
            "codex-actions policy-preview --state ABS --registry ABS --socket ABS --session-id ID\ncodex-actions init --receipts ABS\ncodex-actions inspect --receipts ABS --session-id ID\ncodex-actions preview --state ABS --registry ABS --socket ABS --policy ABS --receipts ABS --existing-config ABS --output NEW_ABS_DIR\ncodex-actions restore-preview --manifest ABS --current ABS --output NEW_ABS_DIR\ncodex-actions hook --state ABS --registry ABS --socket ABS --policy ABS --receipts ABS\ncodex-actions capabilities\nAll configuration outputs are candidates only. No installation or host trust changes. Windows preview unsupported."
        );
        return Ok(true);
    }
    let mut flags = parse_flags(&args[2..])?;
    match action {
        "capabilities" => {
            ensure!(flags.is_empty(), "unexpected flags");
            write_json(&capabilities())?;
        }
        "init" => {
            let path = take_path(&mut flags, "--receipts")?;
            ensure!(flags.is_empty(), "unexpected flags");
            receipts::State::initialize(&path).await?;
            write_json(
                &json!({"initialized":path,"authority":"none","outcome_tracking":"observational_only"}),
            )?;
        }
        "inspect" => {
            let path = take_path(&mut flags, "--receipts")?;
            let session = take(&mut flags, "--session-id")?;
            ensure!(flags.is_empty(), "unexpected flags");
            write_json(
                &receipts::State::open(&path)
                    .await?
                    .inspect(&session)
                    .await?,
            )?;
        }
        "policy-preview" => {
            let state = take_path(&mut flags, "--state")?;
            let registry = take_path(&mut flags, "--registry")?;
            let socket = take_path(&mut flags, "--socket")?;
            let session = take(&mut flags, "--session-id")?;
            ensure!(flags.is_empty(), "unexpected flags");
            let paths = Paths {
                state,
                registry,
                socket,
                policy: PathBuf::new(),
                receipts: PathBuf::new(),
            };
            let (binding, minimum) = runtime::binding(&paths, &session).await?;
            let context = runtime::fetch(&paths.socket, &binding).await?;
            let version =
                crate::adapters::codex_context::protocol::context_version(&context, &binding)?;
            ensure!(version >= minimum, "stale task version");
            let proposed = OperatorPolicy {
                schema_version: 1,
                contract: CONTRACT.into(),
                policy_id: format!(
                    "patch-{}",
                    crate::adapters::codex_context::protocol::digest(
                        format!("{}:{}", binding.workspace_id, binding.task_id).as_bytes()
                    )
                ),
                epoch: 1,
                workspace_id: binding.workspace_id,
                namespace: binding.namespace,
                canonical_root: binding.workspace_root,
                task_id: binding.task_id,
                expected_task_version: version,
                task_version_mode: TaskVersionMode::LatestActive,
                gate: OperatorGate::Paused,
                mode: CheckMode::PatchOnly,
                allowed_operations: vec![Operation::Add, Operation::Update],
                forbidden_roots: vec![],
                expires_at: None,
            };
            // Deliberately paused. Printing a proposal cannot approve it. Changes
            // to an existing policy must increment its epoch after operator review.
            write_json(&serde_json::to_value(proposed)?)?;
        }
        "preview" => {
            let paths = take_paths(&mut flags)?;
            let existing = take_path(&mut flags, "--existing-config")?;
            let output = take_path(&mut flags, "--output")?;
            ensure!(flags.is_empty(), "unexpected flags");
            let (policy, digest) = protocol::load_policy(&paths.policy)?;
            let executable = std::env::current_exe()?.canonicalize()?;
            let argv = vec![
                path_text(&executable)?,
                "codex-actions".into(),
                "hook".into(),
                "--state".into(),
                path_text(&paths.state)?,
                "--registry".into(),
                path_text(&paths.registry)?,
                "--socket".into(),
                path_text(&paths.socket)?,
                "--policy".into(),
                path_text(&paths.policy)?,
                "--receipts".into(),
                path_text(&paths.receipts)?,
            ];
            let command = argv
                .iter()
                .map(|v| format!("'{}'", v.replace('\'', "'\\''")))
                .collect::<Vec<_>>()
                .join(" ");
            let metadata = preview::PreviewMetadata {
                target_file: path_text(&existing)?,
                binary_path: Some(path_text(&executable)?),
                binary_sha256: Some(file_digest(&executable)?),
                policy_revision: Some(policy.epoch),
                policy_digest: Some(digest),
                strict_local: policy.mode == CheckMode::StrictLocal,
                ..Default::default()
            };
            let bundle = preview::build_preview(
                &read_limited(&existing, 4 * 1024 * 1024)?,
                &command,
                metadata,
            )?;
            new_private_dir(&output)?;
            save(&output.join("hooks.candidate.json"), &bundle.after_bytes)?;
            save(&output.join("hooks.before.json"), &bundle.before_bytes)?;
            save(
                &output.join("manifest.json"),
                &serde_json::to_vec_pretty(&bundle.manifest)?,
            )?;
            save(&output.join("diff.txt"), bundle.diff.as_bytes())?;
            save(
                &output.join("rollback-plan.json"),
                &serde_json::to_vec_pretty(&bundle.rollback_plan)?,
            )?;
            write_json(
                &json!({"output":output,"status":"preview_only_not_installed","policy_gate":policy.gate,"sync_location_verified":false,"trust_review_required":true,"capabilities":capabilities()}),
            )?;
        }
        "restore-preview" => {
            let manifest_path = take_path(&mut flags, "--manifest")?;
            let current = take_path(&mut flags, "--current")?;
            let output = take_path(&mut flags, "--output")?;
            ensure!(flags.is_empty(), "unexpected flags");
            let manifest: preview::PreviewManifest =
                serde_json::from_slice(&read_limited(&manifest_path, 40 * 1024 * 1024)?)?;
            ensure!(
                path_text(&current)? == manifest.metadata.target_file,
                "restore target differs from preview manifest"
            );
            let plan = preview::plan_restore(&read_limited(&current, 4 * 1024 * 1024)?, &manifest);
            new_private_dir(&output)?;
            if let preview::RestorePlan::Restore { restore_bytes, .. } = &plan {
                save(&output.join("hooks.restore-candidate.json"), restore_bytes)?;
            }
            save(
                &output.join("restore-plan.json"),
                &serde_json::to_vec_pretty(&plan)?,
            )?;
            write_json(&json!({"output":output,"plan":plan,"applied":false}))?;
        }
        _ => anyhow::bail!("unknown codex-actions command"),
    }
    Ok(true)
}
async fn hook(args: &[String]) -> Result<crate::domain::action_check::Decision> {
    let mut flags = parse_flags(args)?;
    let paths = take_paths(&mut flags)?;
    ensure!(flags.is_empty(), "unexpected flags");
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((protocol::MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let input = protocol::ActionInput::parse(&bytes)?;
    // Leave time to emit an explicit denial before the preview's 15 s host
    // timeout. A host that never starts this process remains outside coverage.
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime::check(&paths, &input),
    )
    .await
    .context("action check deadline exceeded")?
}
fn capabilities() -> Value {
    json!({"contract":CONTRACT,"mode":"partial_parameter_checks_unverified_host","supported_parameter_contracts":["apply_patch_strict_subset"],"default_matcher":"^apply_patch$","shell_semantics":"unsupported","hosted_tools":"uncovered","write_stdin":"uncovered","hook_start_failure":"may_fail_open","native_permissions":"unchanged","execution_authorization":false,"race_free_path_confinement":false,"mount_and_sync_detection":"unverified","same_os_user_tamper_resistance":false,"actual_host_probe":"blocked_or_unverified","windows_command_preview":"unsupported","windows_checked_writes":"unsupported","installation":"not_performed"})
}
fn parse_flags(args: &[String]) -> Result<BTreeMap<String, String>> {
    ensure!(args.len().is_multiple_of(2), "flags need values");
    let mut flags = BTreeMap::new();
    for p in args.chunks_exact(2) {
        ensure!(
            p[0].starts_with("--") && flags.insert(p[0].clone(), p[1].clone()).is_none(),
            "duplicate or invalid flag"
        );
    }
    Ok(flags)
}
fn take(flags: &mut BTreeMap<String, String>, key: &str) -> Result<String> {
    flags.remove(key).with_context(|| format!("missing {key}"))
}
fn take_path(flags: &mut BTreeMap<String, String>, key: &str) -> Result<PathBuf> {
    let p = PathBuf::from(take(flags, key)?);
    ensure!(p.is_absolute(), "absolute path required");
    path_text(&p)?;
    Ok(p)
}
fn take_paths(flags: &mut BTreeMap<String, String>) -> Result<Paths> {
    Ok(Paths {
        state: take_path(flags, "--state")?,
        registry: take_path(flags, "--registry")?,
        socket: take_path(flags, "--socket")?,
        policy: take_path(flags, "--policy")?,
        receipts: take_path(flags, "--receipts")?,
    })
}
fn path_text(path: &Path) -> Result<String> {
    let s = path.to_str().context("non UTF-8 path unsupported")?;
    ensure!(!s.chars().any(char::is_control), "control bytes in path");
    Ok(s.into())
}
fn read_limited(path: &Path, max: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((max + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= max, "file exceeds limit");
    Ok(bytes)
}
fn file_digest(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn new_private_dir(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "absolute output directory required");
    for ancestor in path.parent().context("output parent missing")?.ancestors() {
        let m = fs::symlink_metadata(ancestor)?;
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "output ancestors must be real directories"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(windows)]
    crate::support::windows_security::private_create_directory(path)?;
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("private output directory unsupported");
    Ok(())
}
fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(windows)]
    let mut file = crate::support::windows_security::private_create(path)?;
    #[cfg(not(windows))]
    let mut file = {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(path)?
    };
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn write_json(value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    let mut out = std::io::stdout().lock();
    out.write_all(&bytes)?;
    out.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_flags_and_relative_paths_rejected() {
        assert!(parse_flags(&["--x".into(), "a".into(), "--x".into(), "b".into()]).is_err());
        let mut f = BTreeMap::from([("--x".into(), "relative".into())]);
        assert!(take_path(&mut f, "--x").is_err());
    }
    #[test]
    fn capabilities_do_not_promote_documentation_to_protection() {
        let v = capabilities();
        assert_eq!(v["actual_host_probe"], "blocked_or_unverified");
        assert_eq!(v["execution_authorization"], false);
        assert_eq!(v["write_stdin"], "uncovered");
    }
    #[cfg(unix)]
    #[test]
    fn candidate_writes_never_overwrite() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let out = root.join("candidate");
        new_private_dir(&out).unwrap();
        assert!(new_private_dir(&out).is_err());
        save(&out.join("x"), b"first").unwrap();
        assert!(save(&out.join("x"), b"second").is_err());
        assert_eq!(fs::read(out.join("x")).unwrap(), b"first");
    }
}
