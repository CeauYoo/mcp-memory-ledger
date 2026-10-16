//! Fail-closed, current-user Windows security for local-core state.
//!
//! Existing ACLs are inspected, never repaired. Only the current user, SYSTEM,
//! and BUILTIN Administrators may have allow ACEs. New files receive a protected
//! DACL containing only the current process user SID. Administrators with
//! elevated privileges and other processes of the same user are not isolated.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{OsStr, c_void},
    fs::{self, File},
    io::{self, Read},
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr::{addr_of_mut, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation, INHERIT_ONLY_ACE,
        IsValidAcl, IsValidSid, IsWellKnownSid, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid,
        WinLocalSystemSid,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW,
        GetFileInformationByHandle, OPEN_EXISTING, READ_CONTROL,
    },
    System::{
        SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE},
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this guard owns a non-null, non-pseudo OS handle.
        unsafe { CloseHandle(self.0) };
    }
}

/// The aligned TokenUser buffer owns the SID referenced by its TOKEN_USER header.
pub(crate) struct UserSid(Vec<usize>);
impl UserSid {
    pub(crate) fn current() -> Result<Self> {
        // SAFETY: GetCurrentProcess returns a borrowed pseudo-handle.
        unsafe { Self::for_process(GetCurrentProcess()) }
    }

    unsafe fn for_process(process: HANDLE) -> Result<Self> {
        let mut token = null_mut();
        // SAFETY: valid borrowed process handle and writable output handle.
        check(unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) })?;
        let token = OwnedHandle(token);
        let mut len = 0;
        // The sizing call normally fails with ERROR_INSUFFICIENT_BUFFER.
        unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut len) };
        ensure!(
            len as usize >= size_of::<TOKEN_USER>() && len <= 64 * 1024,
            "cannot determine process user token size"
        );
        let mut buffer = vec![0usize; (len as usize).div_ceil(size_of::<usize>())];
        // SAFETY: aligned writable storage contains at least len bytes.
        check(unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                len,
                &mut len,
            )
        })?;
        let sid = Self(buffer);
        // SAFETY: the OS initialized TOKEN_USER and its in-buffer SID.
        ensure!(
            unsafe { IsValidSid(sid.as_ptr()) } != 0,
            "invalid process user SID"
        );
        Ok(sid)
    }

    fn as_ptr(&self) -> PSID {
        // SAFETY: buffer is aligned, and successful GetTokenInformation initialized it.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    fn matches(&self, other: PSID) -> bool {
        // SAFETY: callers provide SID pointers returned in live Windows buffers.
        !other.is_null() && unsafe { IsValidSid(other) != 0 && EqualSid(self.as_ptr(), other) != 0 }
    }

    pub(crate) fn text(&self) -> Result<String> {
        let mut text = null_mut();
        // SAFETY: valid SID; API allocates a NUL-terminated string.
        check(unsafe { ConvertSidToStringSidW(self.as_ptr(), &mut text) })?;
        let allocation = LocalAllocation(text.cast());
        let mut len = 0;
        // SAFETY: the API returned a NUL-terminated SID string.
        unsafe {
            while *text.add(len) != 0 {
                len += 1;
            }
            let value = String::from_utf16(std::slice::from_raw_parts(text, len))?;
            drop(allocation);
            Ok(value)
        }
    }

    /// Fail closed if the peer exited or its process token cannot be inspected.
    pub(crate) fn verify_process(&self, pid: u32) -> Result<()> {
        // SAFETY: opens only query rights, never process memory or mutation rights.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        ensure!(
            !process.is_null(),
            "cannot verify local-core peer process: {}",
            io::Error::last_os_error()
        );
        let process = OwnedHandle(process);
        // SAFETY: process guard keeps the borrowed handle valid.
        let peer = unsafe { Self::for_process(process.0) }?;
        ensure!(
            self.matches(peer.as_ptr()),
            "local-core peer belongs to another user"
        );
        Ok(())
    }
}

struct LocalAllocation(*mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: Windows allocated the buffer with LocalAlloc-compatible ownership.
        unsafe { LocalFree(self.0) };
    }
}

/// Supplies a short-lived SECURITY_ATTRIBUTES with a protected owner-only DACL.
/// It is valid only during `create`; neither descriptor nor pointers are retained.
pub(crate) fn with_private_security<T>(
    create: impl FnOnce(*mut c_void) -> io::Result<T>,
) -> Result<T> {
    with_security(false, create)
}

fn with_security<T>(inherit: bool, create: impl FnOnce(*mut c_void) -> io::Result<T>) -> Result<T> {
    let sid = UserSid::current()?.text()?;
    let flags = if inherit { "OICI" } else { "" };
    let sddl = wide(OsStr::new(&format!("O:{sid}D:P(A;{flags};GA;;;{sid})")))?;
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: NUL-terminated SDDL and valid output pointer; revision 1 is SDDL_REVISION_1.
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            null_mut(),
        )
    })?;
    let _allocation = LocalAllocation(descriptor);
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    Ok(create(
        (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
    )?)
}

fn check(success: i32) -> io::Result<()> {
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut value: Vec<_> = value.encode_wide().collect();
    ensure!(!value.contains(&0), "Windows path contains a NUL character");
    value.push(0);
    Ok(value)
}

/// Reject UNC/device namespaces, ADS, relative components and reparse ancestors.
/// No remote path is opened while inspecting security metadata.
pub(crate) fn validate_local_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "local-core state requires an absolute local path"
    );
    let mut components = path.components();
    ensure!(
        matches!(components.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))),
        "local-core state must be on a local drive, not a UNC or device path"
    );
    for component in components {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let text = name
                    .to_str()
                    .context("local state paths must be valid UTF-8")?;
                let stem = text.split('.').next().unwrap_or("").to_ascii_uppercase();
                let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                    || (stem.len() == 4
                        && (stem.starts_with("COM") || stem.starts_with("LPT"))
                        && matches!(stem.as_bytes()[3], b'1'..=b'9'));
                ensure!(
                    !device && !text.contains(':') && !text.ends_with(['.', ' ']),
                    "alternate streams and ambiguous Windows file names are not supported"
                );
            }
            _ => anyhow::bail!("local-core state path must not contain relative components"),
        }
    }
    let root = path
        .ancestors()
        .last()
        .context("local drive root required")?;
    let root = wide(root.as_os_str())?;
    // SAFETY: NUL-terminated local drive root. Reject mapped SMB drives before
    // opening any file or requesting metadata, not only explicit UNC paths.
    let drive_type = unsafe { GetDriveTypeW(root.as_ptr()) };
    ensure!(
        matches!(drive_type, 2 | 3),
        "local-core state requires a fixed or removable local drive"
    );
    for ancestor in path.ancestors().skip(1) {
        let metadata = fs::symlink_metadata(ancestor).context("local state parent must exist")?;
        ensure!(
            metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
            "local state ancestors must be real directories without reparse points"
        );
    }
    Ok(())
}

fn open_existing(
    path: &Path,
    read_contents: bool,
    directory: bool,
    share_delete: bool,
) -> Result<File> {
    validate_local_path(path)?;
    let path = wide(path.as_os_str())?;
    let access = READ_CONTROL | FILE_READ_ATTRIBUTES | if read_contents { GENERIC_READ } else { 0 };
    let sharing =
        FILE_SHARE_READ | FILE_SHARE_WRITE | if share_delete { FILE_SHARE_DELETE } else { 0 };
    // SAFETY: validated local, NUL-terminated path. OPEN_REPARSE_POINT prevents following
    // a final-component link; its attributes are rejected by validate_handle.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            sharing,
            null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
            null_mut(),
        )
    };
    ensure!(
        handle != INVALID_HANDLE_VALUE,
        "open private local state: {}",
        io::Error::last_os_error()
    );
    // SAFETY: successful CreateFile returned an owned handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Inspect the actual open object, not only a pathname before opening it.
pub(crate) fn validate_private_handle(file: &File) -> Result<()> {
    validate_handle(file, false)
}

fn validate_handle(file: &File, directory: bool) -> Result<()> {
    let handle = file.as_raw_handle();
    // SAFETY: zero initializes an output-only POD structure.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    // SAFETY: file keeps handle alive and info is writable.
    check(unsafe { GetFileInformationByHandle(handle, &mut info) })?;
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "private state must not be a reparse point"
    );
    ensure!(
        (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) == directory,
        "private state has the wrong file type"
    );
    ensure!(
        directory || info.nNumberOfLinks == 1,
        "private state must not have hard-link aliases"
    );
    validate_owner_acl(handle, directory)
}

fn validate_owner_acl(handle: HANDLE, include_inheritance: bool) -> Result<()> {
    let mut owner: PSID = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: borrowed handle remains live; output pointers address initialized storage.
    let error = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    ensure!(
        error == 0,
        "inspect private state ACL: {}",
        io::Error::from_raw_os_error(error as i32)
    );
    let _allocation = LocalAllocation(descriptor);
    validate_acl(owner, dacl, include_inheritance)
}

fn validate_acl(owner: PSID, dacl: *mut ACL, include_inheritance: bool) -> Result<()> {
    let user = UserSid::current()?;
    ensure!(
        user.matches(owner),
        "private state must be owned by the current Windows user"
    );
    // A missing/NULL DACL grants everyone access and must never pass this check.
    ensure!(
        !dacl.is_null() && unsafe { IsValidAcl(dacl) } != 0,
        "private state must have a valid non-NULL DACL"
    );
    // SAFETY: GetSecurityInfo returned a valid ACL in the live descriptor allocation.
    for index in 0..unsafe { (*dacl).AceCount } {
        let mut ace = null_mut();
        // SAFETY: index is inside AceCount and output storage is valid.
        check(unsafe { GetAce(dacl, index as u32, &mut ace) })?;
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if !include_inheritance && header.AceFlags & INHERIT_ONLY_ACE as u8 != 0 {
            continue;
        }
        match header.AceType as u32 {
            ACCESS_DENIED_ACE_TYPE => {}
            ACCESS_ALLOWED_ACE_TYPE => {
                ensure!(
                    header.AceSize as usize >= size_of::<ACCESS_ALLOWED_ACE>(),
                    "malformed private-state ACL"
                );
                let allowed = ace.cast::<ACCESS_ALLOWED_ACE>();
                // SAFETY: OS validated this ACE; SidStart is the beginning of its SID.
                let sid = unsafe { addr_of_mut!((*allowed).SidStart).cast() };
                // SYSTEM and Administrators already bypass this same-user trust
                // boundary through privilege. Other users/groups must not be granted.
                let trusted = user.matches(sid)
                    || unsafe {
                        IsWellKnownSid(sid, WinLocalSystemSid) != 0
                            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
                    };
                ensure!(
                    trusted,
                    "private state DACL grants access outside the current user, SYSTEM and Administrators; configure a private directory/file explicitly"
                );
            }
            _ => anyhow::bail!(
                "unsupported private-state DACL entry; cannot establish owner-only access"
            ),
        }
    }
    Ok(())
}

pub(crate) fn validate_private_parent(path: &Path) -> Result<()> {
    validate_local_path(path)?;
    ensure!(
        path.file_name().is_some(),
        "private state requires a file name"
    );
    let parent = path
        .parent()
        .context("private state requires a parent directory")?;
    validate_handle(&open_existing(parent, false, true, true)?, true)
        .context("local state parent must be owned by the current user with a private DACL")
}

/// Retains non-delete-sharing handles from the drive root through the private
/// parent. A less-privileged writer of an ancestor cannot rename/substitute a
/// directory after validation while a cooperating writer/core owns this guard.
pub(crate) struct DirectoryGuard {
    _directories: Vec<File>,
}

pub(crate) fn lock_private_parent(path: &Path) -> Result<DirectoryGuard> {
    validate_local_path(path)?;
    let parent = path.parent().context("private state parent required")?;
    let mut ancestors: Vec<_> = parent.ancestors().collect();
    ancestors.reverse();
    let mut directories = Vec::new();
    for ancestor in ancestors {
        let file = open_existing(ancestor, false, true, false)?;
        let attributes = file.metadata()?.file_attributes();
        ensure!(
            attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
                && attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
            "local state ancestors must be real directories"
        );
        directories.push(file);
    }
    validate_handle(
        directories
            .last()
            .context("private state parent required")?,
        true,
    )?;
    Ok(DirectoryGuard {
        _directories: directories,
    })
}

pub(crate) fn open_private_file(path: &Path) -> Result<File> {
    let _parent = lock_private_parent(path)?;
    let file = open_existing(path, true, false, true)?;
    validate_private_handle(&file)?;
    Ok(file)
}

pub(crate) fn validate_private_file(path: &Path) -> Result<()> {
    validate_private_handle(&open_existing(path, false, false, true)?)
}

pub(crate) fn read_private_file(path: &Path, limit: u64) -> Result<String> {
    let mut value = String::new();
    open_private_file(path)?
        .take(limit.saturating_add(1))
        .read_to_string(&mut value)?;
    ensure!(
        value.len() as u64 <= limit,
        "private capability file exceeds size limit"
    );
    Ok(value)
}

/// Atomically creates a new file with private security; never changes an existing ACL.
pub(crate) fn private_create(path: &Path) -> Result<File> {
    create_file(path, true)
}

/// Creates a new private directory, including private inheritance for its children.
/// An existing directory is never modified.
pub(crate) fn private_create_directory(path: &Path) -> Result<()> {
    validate_local_path(path)?;
    let path = wide(path.as_os_str())?;
    with_security(true, |attributes| {
        // SAFETY: valid path and callback-scoped security attributes.
        check(unsafe { CreateDirectoryW(path.as_ptr(), attributes.cast()) })
    })
}

pub(crate) struct PrivateTempDir(PathBuf);
impl PrivateTempDir {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn private_tempdir_in(parent: &Path, prefix: &str) -> Result<PrivateTempDir> {
    let path = parent.join(format!("{prefix}{}", uuid::Uuid::new_v4()));
    private_create_directory(&path)?;
    Ok(PrivateTempDir(path))
}

fn create_file(path: &Path, share_delete: bool) -> Result<File> {
    validate_local_path(path)?;
    let path = wide(path.as_os_str())?;
    with_private_security(|attributes| {
        // SAFETY: lifetime of descriptor is bounded by the callback; valid path.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
                FILE_SHARE_READ
                    | FILE_SHARE_WRITE
                    | if share_delete { FILE_SHARE_DELETE } else { 0 },
                attributes.cast(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: a fresh, uniquely owned file handle.
            Ok(unsafe { File::from_raw_handle(handle) })
        }
    })
}

/// Cooperating writer lock. Persistent sidecar, kernel lock released on process exit.
/// No FILE_SHARE_DELETE means the lock path cannot be renamed while held.
pub(crate) struct PrivateLock {
    file: File,
    _parent: DirectoryGuard,
}
impl PrivateLock {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        let parent = lock_private_parent(path)?;
        let file = match create_file(path, false) {
            Ok(file) => file,
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::AlreadyExists) =>
            {
                let file = open_existing(path, true, false, false)?;
                validate_private_handle(&file)?;
                file
            }
            Err(error) => return Err(error),
        };
        file.try_lock()
            .context("another core or adoption owns this database or endpoint")?;
        Ok(Self {
            file,
            _parent: parent,
        })
    }
}
impl Drop for PrivateLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) fn acquire_database_lock(database: &Path, registry: &Path) -> Result<PrivateLock> {
    validate_private_parent(registry)?;
    validate_private_file(database)
        .context("ledger must be a current-user-owned private regular file")?;
    let lock = PrivateLock::acquire(&sidecar(database, ".local-core.lock"))?;
    // Recheck only after the ancestor handles are held, closing the validation
    // versus parent-substitution gap before SQLite opens the database pathname.
    validate_private_file(database)?;
    Ok(lock)
}

pub(crate) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileIdentity(u32, u32, u32);

/// Uses handle-based Win32 identity; std's Windows inode access is nightly-only.
pub(crate) fn file_identity(file: &File) -> Result<FileIdentity> {
    // SAFETY: output-only POD and a live borrowed handle.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    check(unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) })?;
    Ok(FileIdentity(
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}

pub(crate) fn path_identity(path: &Path) -> Result<FileIdentity> {
    file_identity(&open_private_file(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use windows_sys::Win32::Security::{GetSecurityDescriptorDacl, GetSecurityDescriptorOwner};
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_OBJECT_ACE_TYPE;

    fn check_sddl(sddl: &str, directory: bool) -> Result<()> {
        check_sddl_with_ace_type(sddl, directory, None)
    }

    fn check_sddl_with_ace_type(
        sddl: &str,
        directory: bool,
        expected_type: Option<u32>,
    ) -> Result<()> {
        let sddl = wide(OsStr::new(sddl))?;
        let mut descriptor = null_mut();
        // SAFETY: valid UTF-16 input and writable output; allocation is scoped below.
        check(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        })?;
        let _allocation = LocalAllocation(descriptor);
        let mut owner = null_mut();
        let mut dacl = null_mut();
        let mut defaulted = 0;
        let mut present = 0;
        // SAFETY: valid descriptor, output pointers; memory remains live during validation.
        check(unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) })?;
        check(unsafe {
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
        })?;
        ensure!(present != 0, "missing DACL");
        if let Some(expected_type) = expected_type {
            assert!(!dacl.is_null(), "ACE fixture requires a non-NULL DACL");
            // SAFETY: the parsed descriptor owns this ACL until validation returns.
            assert_eq!(unsafe { (*dacl).AceCount }, 1);
            let mut ace = null_mut();
            // SAFETY: the fixture has exactly one ACE and output storage is valid.
            check(unsafe { GetAce(dacl, 0, &mut ace) })?;
            assert_eq!(
                unsafe { (*ace.cast::<ACE_HEADER>()).AceType as u32 },
                expected_type,
                "SDDL fixture did not produce its intended ACE type"
            );
        }
        validate_acl(owner, dacl, directory)
    }

    #[test]
    fn acl_policy_distinguishes_owner_privileged_broad_null_and_inherited_grants() {
        let user = UserSid::current().unwrap();
        let sid = user.text().unwrap();
        for dacl in [
            format!("D:P(A;;GA;;;{sid})"),
            format!("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})"),
            format!("D:(A;ID;GA;;;{sid})"),
            format!("D:(D;;GA;;;WD)(A;;GA;;;{sid})"),
            "D:".into(),
        ] {
            assert!(
                check_sddl(&format!("O:{sid}{dacl}"), true).is_ok(),
                "{dacl}"
            );
        }
        for dacl in [
            "D:(A;;GR;;;WD)",
            "D:(A;;GR;;;BU)",
            "D:(A;;GR;;;AU)",
            "D:(A;ID;GR;;;WD)",
            "D:(D;;GA;;;WD)(A;;GR;;;WD)",
            "D:(OA;;GA;;;WD)",
            "D:NO_ACCESS_CONTROL",
        ] {
            assert!(
                check_sddl(&format!("O:{sid}{dacl}"), true).is_err(),
                "{dacl}"
            );
        }
        assert!(check_sddl(&format!("O:WDD:(A;;GA;;;{sid})"), true).is_err());
        assert!(validate_acl(user.as_ptr(), null_mut(), false).is_err());
        // Inherit-only entries do not grant access to a file itself, but could
        // expose future SQLite WAL/journal children of a directory.
        let inherit = format!("O:{sid}D:(A;OICIIO;GR;;;WD)(A;;GA;;;{sid})");
        assert!(check_sddl(&inherit, false).is_ok());
        assert!(check_sddl(&inherit, true).is_err());
    }

    #[test]
    fn object_ace_fixtures_check_normalization_and_reject_actual_object_entries() {
        let sid = UserSid::current().unwrap().text().unwrap();
        // The Win32 converter normalizes OA without either GUID to a normal A:
        // https://learn.microsoft.com/windows/win32/api/sddl/nf-sddl-convertstringsecuritydescriptortosecuritydescriptorw
        assert!(
            check_sddl_with_ace_type(
                &format!("O:{sid}D:(OA;;GA;;;{sid})"),
                true,
                Some(ACCESS_ALLOWED_ACE_TYPE),
            )
            .is_ok()
        );
        let guid = "01234567-89ab-cdef-0123-456789abcdef";
        for (object_guid, inherit_guid) in [(guid, ""), ("", guid), (guid, guid)] {
            let error = check_sddl_with_ace_type(
                &format!("O:{sid}D:(OA;;GA;{object_guid};{inherit_guid};{sid})"),
                true,
                Some(ACCESS_ALLOWED_OBJECT_ACE_TYPE),
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("unsupported private-state DACL entry"),
                "object ACE must be rejected by policy, not fixture parsing: {error}"
            );
        }
    }

    #[test]
    fn private_file_checks_use_handles_and_reject_hard_links_and_large_tokens() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let dir = private_tempdir_in(&root, "ledger-security-").unwrap();
        let path = dir.path().join("token");
        let mut file = private_create(&path).unwrap();
        file.write_all(b"synthetic-test-only").unwrap();
        drop(file);
        assert_eq!(read_private_file(&path, 64).unwrap(), "synthetic-test-only");
        assert!(read_private_file(&path, 4).is_err());
        assert!(private_create(&path).is_err());
        fs::hard_link(&path, dir.path().join("alias")).unwrap();
        assert!(validate_private_file(&path).is_err());
    }

    #[test]
    fn writer_lock_blocks_duplicate_owner_and_parent_rename_until_release() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let dir = private_tempdir_in(&root, "ledger-lock-").unwrap();
        let path = dir.path().join("state.lock");
        let first = PrivateLock::acquire(&path).unwrap();
        assert!(PrivateLock::acquire(&path).is_err());
        let renamed = dir.path().with_extension("renamed");
        assert!(fs::rename(dir.path(), &renamed).is_err());
        drop(first);
        let next = PrivateLock::acquire(&path).unwrap();
        drop(next);
        fs::rename(dir.path(), &renamed).unwrap();
        fs::rename(&renamed, dir.path()).unwrap();
    }

    #[test]
    fn remote_device_relative_and_alternate_stream_paths_fail_before_access() {
        for path in [
            r"\\server\share\registry.json",
            r"\\?\UNC\server\share\registry.json",
            r"\\.\pipe\foreign",
            r"relative\registry.json",
            r"C:\registry.json:stream",
            r"C:\ambiguous.\registry.json",
            r"C:\parent\..\registry.json",
        ] {
            assert!(validate_local_path(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn peer_identity_is_taken_from_a_live_os_process_token() {
        let user = UserSid::current().unwrap();
        assert!(user.verify_process(std::process::id()).is_ok());
        assert!(user.verify_process(0).is_err());
    }
}
