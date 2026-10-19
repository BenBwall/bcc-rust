//! Fixtures shared by the crate's unit tests: temporary directories for
//! include and resource searches, and snapshot files under `tests/fixtures/`.

// Shared process runner; integration tests include the same implementation.
#[path = "../tests/support/native.rs"]
pub(crate) mod native;

std::thread_local! {
    /// Counts effects-only recipe tasks lowering interprets, scoped to the
    /// compiling test thread so parallel tests cannot affect the bound.
    #[cfg_attr(
        windows,
        expect(
            clippy::missing_const_for_thread_local,
            reason = "The const initializer still trips the lint in the Windows std expansion."
        )
    )]
    pub(crate) static EFFECT_RECIPE_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Counts the predecessor edges constant threading's prefilter reads,
    /// scoped to the optimizing test thread.
    #[cfg_attr(
        windows,
        expect(
            clippy::missing_const_for_thread_local,
            reason = "The const initializer still trips the lint in the Windows std expansion."
        )
    )]
    pub(crate) static THREAD_PREFILTER_EDGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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

/// GCC scalar/library names remain ordinary calls when a local pointer binds
/// them. Spellings come from the documented families, independently of the
/// compiler classifier. The unbound clz call protects intrinsic recognition.
pub(crate) fn builtin_binding_program() -> String {
    use std::fmt::Write;
    let mut names = Vec::new();
    for family in ["clz", "ctz", "popcount", "parity", "ffs", "clrsb"] {
        for suffix in ["", "l", "ll"] {
            names.push(format!("__builtin_{family}{suffix}"));
        }
    }
    for operation in ["add", "sub", "mul"] {
        names.push(format!("__builtin_{operation}_overflow"));
        names.push(format!("__builtin_{operation}_overflow_p"));
        for sign in ["s", "u"] {
            for suffix in ["", "l", "ll"] {
                names.push(format!("__builtin_{sign}{operation}{suffix}_overflow"));
            }
        }
    }
    for name in [
        "bswap16",
        "bswap32",
        "bswap64",
        "expect",
        "expect_with_probability",
        "assume_aligned",
        "prefetch",
        "constant_p",
        "object_size",
        "dynamic_object_size",
        "trap",
        "unreachable",
        "alloca",
        "va_start",
        "va_end",
        "va_copy",
        "abort",
        "exit",
        "malloc",
        "calloc",
        "realloc",
        "free",
        "memcpy",
        "memmove",
        "memset",
        "memcmp",
        "strlen",
        "strcmp",
        "strncmp",
        "strcpy",
        "strncpy",
        "memchr",
        "strchr",
        "strrchr",
        "printf",
        "sprintf",
        "puts",
        "putchar",
        "abs",
        "fabs",
        "floor",
    ] {
        names.push(format!("__builtin_{name}"));
    }
    names.extend([
        "alloca".into(),
        "__sync_fetch_and_add".into(),
        "__atomic_load_n".into(),
    ]);
    let mut source = "int f(unsigned x){return 7;} int main(void){if(__builtin_clz(1u)!=31)return \
                      1;"
    .to_string();
    for name in names {
        write!(
            source,
            "{{int (*{name})(unsigned)=f;if(({name})(1)!=7)return 2;}}"
        )
        .unwrap();
    }
    source.push_str("return 0;}");
    source
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

/// A fresh temporary directory, removed with its contents when dropped.
#[derive(Debug)]
pub(crate) struct TempDir(PathBuf);

impl TempDir {
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
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}
