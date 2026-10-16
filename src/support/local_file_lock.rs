//! Advisory exclusion shared by cooperating local core and administrative adoption.
//! Does not protect against arbitrary processes owned by the same OS user.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

pub(crate) struct PrivateLock(File);
impl PrivateLock {
    pub(crate) fn acquire(path: &Path, uid: u32) -> Result<Self> {
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let before = fs::symlink_metadata(path)?;
                ensure!(
                    before.is_file()
                        && !before.file_type().is_symlink()
                        && before.uid() == uid
                        && before.nlink() == 1
                        && before.permissions().mode() & 0o077 == 0,
                    "core lock must be an owned private regular file"
                );
                let file = OpenOptions::new().read(true).write(true).open(path)?;
                let after = file.metadata()?;
                ensure!(
                    before.dev() == after.dev() && before.ino() == after.ino(),
                    "core lock changed while opening"
                );
                file
            }
            Err(error) => return Err(error.into()),
        };
        file.try_lock()
            .context("another core owns this database or socket")?;
        Ok(Self(file))
    }
}
impl Drop for PrivateLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
