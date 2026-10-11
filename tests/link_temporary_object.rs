#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "CLI regression tests."
)]

#[path = "support/native.rs"]
mod native;

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary paths and process output."
)]
mod tests {
    use std::{
        path::PathBuf,
        process::Command,
    };

    #[test]
    fn existing_object_survives_successful_link() {
        check("int main(void) { return 0; }", true);
    }

    #[test]
    fn existing_object_survives_failed_link() {
        check(
            "void missing(void); int main(void) { missing(); return 0; }",
            false,
        );
    }

    fn check(source: &str, success: bool) {
        let target = if cfg!(all(windows, target_env = "gnu")) {
            "x86_64-w64-windows-gnu"
        } else if cfg!(windows) {
            "x86_64-pc-windows-msvc"
        } else if cfg!(target_env = "musl") {
            "x86_64-unknown-linux-musl"
        } else {
            "x86_64-unknown-linux-gnu"
        };
        let directory = std::env::temp_dir().join(format!(
            "bcc-link-sentinel-{}-{success}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let cleanup = Directory(directory);
        let output = cleanup.0.join("app.exe");
        let sentinel = cleanup.0.join("app.exe.bcc.o");
        std::fs::write(&sentinel, b"user-owned object").unwrap();
        let built = super::native::run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--target", target, "--input", source, "-o"])
                .arg(&output),
        )
        .unwrap();
        assert_eq!(built.status.success(), success, "{built:?}");
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"user-owned object");
        let mut names: Vec<_> = std::fs::read_dir(&cleanup.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        let expected: Vec<_> = if success {
            vec!["app.exe", "app.exe.bcc.o"]
        } else {
            vec!["app.exe.bcc.o"]
        }
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect();
        assert_eq!(
            names, expected,
            "this invocation leaves no temporary object"
        );
    }

    struct Directory(PathBuf);

    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }
}
