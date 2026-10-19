//! The installation stamp gates access to the shared build lock.

use std::{
    fs,
    io,
    path::Path,
};

/// Locks an incomplete installation, checking its stamp again under the lock.
pub(super) fn lock_for_install(
    prefix: &Path,
    stamp: &Path,
    identity: &str,
) -> io::Result<Option<fs::File>> {
    if fs::read_to_string(stamp).is_ok_and(|value| value == identity) {
        return Ok(None);
    }
    fs::create_dir_all(prefix)?;
    let lock = fs::File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(prefix.join(".build-lock"))?;
    lock.lock()?;
    if fs::read_to_string(stamp).is_ok_and(|value| value == identity) {
        return Ok(None);
    }
    Ok(Some(lock))
}
