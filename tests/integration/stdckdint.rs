//! C23's `<stdckdint.h>` (C23 7.20). GCC 14 and Clang ship it, in every
//! language mode, so bcc's own is found under their identities and bcc's;
//! MSVC and TCC 0.9.27 ship none. `tests/fixtures/programs/stdckdint.c`
//! checks each macro's stored result and overflow flag across types;
//! `stdckdint.expected` is what Clang 23.1.5's build prints.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::process::{
        Command,
        Output,
    };

    use crate::native::run_native;

    /// The header is bcc's own under the GCC, Clang and clang-cl identities,
    /// in C89 as in C23, and missing under MSVC's and TCC's.
    #[test]
    fn stdckdint_header_follows_each_identity() {
        let source = fixture("stdckdint_header.c");
        for (identity, target, found) in [
            ("gcc", "x86_64-unknown-linux-gnu", true),
            ("clang", "x86_64-unknown-linux-gnu", true),
            ("clang-cl", "x86_64-pc-windows-msvc", true),
            ("msvc", "x86_64-pc-windows-msvc", false),
            ("tcc", "x86_64-unknown-linux-gnu", false),
        ] {
            for standard in ["-std=c89", "-std=c23"] {
                let output = bcc(&[
                    &format!("--compiler-identity={identity}"),
                    &format!("--target={target}"),
                    standard,
                    "-nostdlibinc",
                    "-fsyntax-only",
                    &source,
                ]);
                let stderr = text(&output.stderr);
                if found {
                    assert!(output.status.success(), "{identity} {standard}: {stderr}");
                } else {
                    assert!(!output.status.success(), "{identity} {standard}");
                    assert!(
                        stderr.contains("stdckdint.h"),
                        "{identity} {standard}: {stderr}"
                    );
                }
            }
        }
    }

    /// Each macro stores the wrapped result and yields whether it
    /// overflowed, as Clang's build does, before and after optimization.
    #[test]
    fn stdckdint_results_match_clang() {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            eprintln!("skipping: the program reads the x86-64 Linux C library's headers");
            return;
        }
        let source = fixture("stdckdint.c");
        let expected = std::fs::read_to_string(fixture("stdckdint.expected")).unwrap();
        for identity in ["--compiler-identity=gcc", "--compiler-identity=clang"] {
            for optimization in ["--opt=none", "--opt=bcc"] {
                let output = bcc(&[identity, "-std=gnu17", "--interpret", optimization, &source]);
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{identity} {optimization}: {}",
                    text(&output.stderr)
                );
                assert_eq!(text(&output.stdout), expected, "{identity} {optimization}");
            }
        }
    }

    fn fixture(name: &str) -> String {
        format!(
            "{}/tests/fixtures/programs/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn bcc(args: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE"),
        )
        .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }
}
