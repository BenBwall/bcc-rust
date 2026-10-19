#[path = "../../build_support/llvm/cache.rs"]
#[expect(
    clippy::disallowed_methods,
    reason = "Build-time cache tooling reads host files."
)]
mod cache;

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Test-only filesystem fixtures for the build cache."
)]
mod tests {
    use std::fs;

    use super::cache::lock_for_install;

    #[test]
    fn installed_libraries_need_no_writable_build_lock() {
        let prefix = std::env::temp_dir().join(format!("bcc-llvm-cache-{}", std::process::id()));
        fs::create_dir_all(prefix.join(".build-lock")).unwrap();
        let stamp = prefix.join(".installed-version");
        fs::write(&stamp, "installed").unwrap();
        // A directory in place of the lock makes any attempt to open it fail,
        // even when this test is run with administrator privileges.
        let result = lock_for_install(&prefix, &stamp, "installed");
        fs::remove_dir_all(&prefix).unwrap();
        assert!(
            matches!(result, Ok(None)),
            "installed cache attempted a write: {result:?}"
        );
    }

    #[test]
    fn incomplete_installations_still_take_the_build_lock() {
        let prefix =
            std::env::temp_dir().join(format!("bcc-llvm-incomplete-{}", std::process::id()));
        let stamp = prefix.join(".installed-version");
        let lock = lock_for_install(&prefix, &stamp, "installed")
            .unwrap()
            .unwrap();
        assert!(prefix.join(".build-lock").is_file());
        drop(lock);
        fs::write(&stamp, "older").unwrap();
        let lock = lock_for_install(&prefix, &stamp, "installed")
            .unwrap()
            .unwrap();
        drop(lock);
        fs::remove_dir_all(&prefix).unwrap();
    }
}
