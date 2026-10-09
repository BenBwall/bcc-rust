//! Fixtures shared by the crate's unit tests: temporary directories for
//! include and resource searches, and snapshot files under `tests/fixtures/`.

use std::{
    path::{
        Path,
        PathBuf,
    },
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

/// A fresh temporary directory, removed with its contents when dropped.
#[derive(Debug)]
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    /// Creates `bcc-<prefix>-…` in the system temporary directory. The process
    /// id, a timestamp, and a counter keep concurrent tests apart.
    pub(crate) fn new(prefix: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "bcc-{prefix}-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn join(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.0.join(relative)
    }

    /// Writes `contents` to `relative`, creating its parent directories.
    pub(crate) fn write(&self, relative: impl AsRef<Path>, contents: &str) {
        let path = self.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

/// Compares `actual` with the snapshot at `relative`, a path from the crate
/// root. With `BLESS=1` in the environment the snapshot is rewritten instead.
#[track_caller]
pub(crate) fn assert_snapshot(relative: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    if std::env::var_os("BLESS").is_some_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("{}: {error}; run with BLESS=1 to create it", path.display())
    });
    pretty_assertions::assert_eq!(expected, actual);
}
