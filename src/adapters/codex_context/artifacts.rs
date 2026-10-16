//! Bounded, local, point-in-time observations. Never updates checkpoint claims.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const FILE_LIMIT: u64 = 8 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 32 * 1024 * 1024;

pub fn observe(root: &Path, context: &Value) -> Vec<Value> {
    let Some(items) = context["checkpoint"]["artifacts"].as_array() else {
        return Vec::new();
    };
    let mut total = 0;
    items.iter().enumerate().map(|(index, item)| {
        let uri = item["uri"].as_str().unwrap_or("");
        let observation = if index >= 32 { json!({"result":"not_performed","reason":"artifact_count_limit"}) }
            else { one(root, uri, item["digest"].as_str(), &mut total) };
        json!({"artifact_index":index,"observed_at":chrono::Utc::now().to_rfc3339(),"observation":observation,"scope":"point_in_time_only_not_task_completion_or_authorization"})
    }).collect()
}
fn local_path(root: &Path, uri: &str) -> Option<PathBuf> {
    if uri.starts_with("file:") {
        let url = url::Url::parse(uri).ok()?;
        if url.host_str().is_some_and(|host| host != "localhost")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        return url.to_file_path().ok();
    }
    let path = PathBuf::from(uri);
    if uri.is_empty() || uri.contains("://") || uri.starts_with("\\\\") {
        return None;
    }
    if path.is_absolute() {
        Some(path)
    } else if uri.contains(':') {
        None
    } else {
        Some(root.join(path))
    }
}
fn one(root: &Path, uri: &str, expected: Option<&str>, total: &mut u64) -> Value {
    let Some(path) = local_path(root, uri) else {
        return json!({"result":"not_performed","reason":"unsupported_nonlocal_reference"});
    };
    #[cfg(windows)]
    if crate::support::windows_security::validate_local_path(&path).is_err() {
        return json!({"result":"not_performed","reason":"unsupported_network_or_device_path"});
    }
    if !path.starts_with(root)
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return json!({"result":"not_performed","reason":"outside_workspace"});
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return json!({"result":"not_performed","reason":"outside_workspace"});
    };
    let mut prefix = root.to_path_buf();
    for part in relative.components() {
        prefix.push(part);
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return json!({"result":"not_performed","reason":"symlink_reference_unsupported"});
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return json!({"result":"failed","reason":"artifact_missing"});
            }
            Err(_) => {
                return json!({"result":"not_performed","reason":"artifact_metadata_unavailable"});
            }
        }
    }
    let Ok(canonical) = path.canonicalize() else {
        return json!({"result":"not_performed","reason":"canonicalization_failed"});
    };
    if !canonical.starts_with(root) {
        return json!({"result":"not_performed","reason":"outside_workspace"});
    }
    let Ok(before) = fs::symlink_metadata(&canonical) else {
        return json!({"result":"not_performed","reason":"artifact_metadata_unavailable"});
    };
    if !before.is_file() || before.len() > FILE_LIMIT || *total + before.len() > TOTAL_LIMIT {
        return json!({"result":"not_performed","reason":"not_regular_file_or_size_limit"});
    }
    let Some(expected) = expected else {
        return json!({"result":"not_performed","exists":true,"reason":"no_digest_content_identity_unverified"});
    };
    let Some(hash) = expected
        .strip_prefix("sha256:")
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return json!({"result":"not_performed","exists":true,"reason":"unsupported_digest_require_sha256_prefix"});
    };
    let Ok(file) = open_artifact(&canonical) else {
        return json!({"result":"not_performed","reason":"artifact_read_unavailable"});
    };
    let Ok(opened) = file.metadata() else {
        return json!({"result":"not_performed","reason":"artifact_metadata_unavailable"});
    };
    if !same_file(&before, &opened)
        || path.canonicalize().ok().as_deref() != Some(canonical.as_path())
    {
        return json!({"result":"not_performed","reason":"artifact_changed_while_opening"});
    }
    let allowance = (FILE_LIMIT + 1).min(TOTAL_LIMIT.saturating_sub(*total));
    if allowance == 0 {
        return json!({"result":"not_performed","reason":"total_read_budget_exhausted"});
    }
    let mut bytes = Vec::new();
    let read_result = file.take(allowance).read_to_end(&mut bytes);
    // Count actual bytes even for errors, growth and rejected observations.
    *total += bytes.len() as u64;
    if read_result.is_err() || bytes.len() as u64 > FILE_LIMIT {
        return json!({"result":"not_performed","reason":"artifact_read_failed_or_size_limit"});
    }
    let Ok(after) = fs::metadata(&canonical) else {
        return json!({"result":"not_performed","reason":"artifact_changed_while_reading"});
    };
    if !same_file(&before, &after) || after.len() != bytes.len() as u64 {
        return json!({"result":"not_performed","reason":"artifact_changed_while_reading"});
    }
    let matched = format!("{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(hash);
    json!({"result":if matched {"passed"}else{"failed"},"exists":true,"digest_matches":matched,"bytes_read":bytes.len(),"reason":"sha256_point_in_time_observation"})
}
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    let same =
        a.is_file() && b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same && a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        same
    }
}

fn open_artifact(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Refuse a replaced final symlink; opening a raced FIFO must not wait
        // for a writer. Constants come from libc for the actual target OS.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "artifact handle is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn final_symlink_is_rejected_by_open_not_only_preflight() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        fs::write(&target, b"test").unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open_artifact(&link).is_err());
        assert!(open_artifact(&target).is_ok());
    }
    #[test]
    fn raced_fifo_is_rejected_without_waiting_for_writer() {
        use std::os::unix::ffi::OsStrExt;
        let temp = tempfile::tempdir().unwrap();
        let fifo = temp.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: NUL-terminated path to a new synthetic file in our own tempdir.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let started = std::time::Instant::now();
        assert!(open_artifact(&fifo).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
