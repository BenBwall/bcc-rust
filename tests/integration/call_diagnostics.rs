//! GNU error/warning contracts follow calls through the actual codegen
//! pipeline.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "The test harness owns subprocess output and temporary artifacts."
)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process::{
            Command,
            Output,
        },
    };

    fn compile(name: &str, source: &str, flags: &[&str]) -> (Output, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("bcc-call-diagnostics-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let object = root.join(format!("{name}.o"));
        drop(fs::remove_file(&object));
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args([
                "--target=x86_64-unknown-linux-gnu",
                "--opt=none",
                "--input",
                source,
            ])
            .args(flags)
            .arg("-c")
            .arg("-o")
            .arg(&object)
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .output()
            .unwrap();
        (output, object)
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    #[test]
    fn llvm_only_elimination_suppresses_error_before_artifact_emission() {
        let source = "int bad(void) __attribute__((error(\"boom\"))); static int zero(void) { \
                      return 0; } int f(void) { if (zero()) return bad(); return 1; }";
        for (name, flags, retained) in [
            ("dead-o0", &[][..], true),
            ("dead-o2", &["--llvm-opt=O2"][..], false),
            (
                "dead-codegen-only",
                &["--llvm-opt=O2", "--llvm-codegen-only"][..],
                true,
            ),
        ] {
            let (output, object) = compile(name, source, flags);
            let text = stderr(&output);
            assert_eq!(output.status.success(), !retained, "{name}: {text}");
            assert_eq!(object.exists(), !retained, "{name}: {text}");
            assert_eq!(
                text.contains("declared with 'error' attribute: boom"),
                retained,
                "{name}: {text}"
            );
            drop(fs::remove_file(object));
        }
    }

    #[test]
    fn llvm_warnings_follow_inlined_copies_and_warning_controls() {
        let source = "int warn(void) __attribute__((warning(\"careful\"))); static int \
                      helper(void) { return warn(); } int f(void) { return helper(); } int \
                      g(void) { return helper(); }";
        for (name, flags, warnings, success) in [
            ("warn-o2", &["--llvm-opt=O2"][..], 2, true),
            (
                "warn-disabled",
                &["--llvm-opt=O2", "-Wno-attribute-warning"][..],
                0,
                true,
            ),
            (
                "warn-error",
                &["--llvm-opt=O2", "-Werror=attribute-warning"][..],
                2,
                false,
            ),
        ] {
            let (output, object) = compile(name, source, flags);
            let text = stderr(&output);
            assert_eq!(output.status.success(), success, "{name}: {text}");
            assert_eq!(object.exists(), success, "{name}: {text}");
            assert_eq!(
                text.matches("declared with 'warning' attribute: careful")
                    .count(),
                warnings,
                "{name}: {text}"
            );
            drop(fs::remove_file(object));
        }
    }

    #[test]
    fn llvm_merged_calls_follow_pinned_clang_source_location_behavior() {
        // Pinned Clang loses per-source call diagnostics when SimplifyCFG
        // merges this diamond. Preserve that behavior even though one call
        // remains in the optimized IR.
        let source = "int bad(void) __attribute__((error(\"merged\"))); int f(int x) { if(x) \
                      return bad(); return bad(); }";
        let (output, object) = compile("merged-o2", source, &["--llvm-opt=O2"]);
        let text = stderr(&output);
        assert!(output.status.success(), "{text}");
        assert!(
            object.exists(),
            "the merged call follows pinned Clang behavior: {text}"
        );
        assert!(!text.contains("declared with 'error'"), "{text}");
        drop(fs::remove_file(object));
    }

    #[test]
    fn abi_rewritten_calls_retain_their_diagnostics() {
        let source = "struct S { long x,y,z; }; struct S bad(void) \
                      __attribute__((error(\"record\"))); int f(void) { return (int)bad().x; }";
        let (output, object) = compile("aggregate-o0", source, &[]);
        let text = stderr(&output);
        assert!(!output.status.success(), "{text}");
        assert!(
            !object.exists(),
            "refused aggregate call emits no object: {text}"
        );
        assert!(
            text.contains("declared with 'error' attribute: record"),
            "{text}"
        );
    }

    #[test]
    fn ir_output_and_indirect_call_syntax_withhold_contracts() {
        let source = "int bad(void) __attribute__((error(\"boom\"))); int f(void) { int \
                      (*p)(void) = bad; p(); (*bad)(); (&bad)(); return 0; }";
        let (output, object) = compile("indirect-o0", source, &[]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(object.exists(), "object is emitted for indirect syntax");
        drop(fs::remove_file(object));
        let direct =
            "int bad(void) __attribute__((error(\"boom\"))); int f(void) { return bad(); }";
        for emit in ["--emit=ir", "--emit=ir-abi", "--emit=llvm"] {
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--input", direct, emit])
                .output()
                .unwrap();
            assert!(output.status.success(), "{emit}: {}", stderr(&output));
            assert!(
                !stderr(&output).contains("boom"),
                "{emit}: {}",
                stderr(&output)
            );
        }
    }
}
