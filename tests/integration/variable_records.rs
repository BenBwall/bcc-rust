//! GNU structures and unions with members of variable size under the GCC
//! identity, on every back end: the IR interpreter (unoptimized and with
//! bcc's optimizer), LLVM at `-O0` and `-O2`, and the native back end.
//! `tests/fixtures/programs/variable_records.c` prints what GCC prints.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    use crate::native::run_native;

    const GCC: &str = "--compiler-identity=gcc";

    /// One block of output per call of `run`, for 3 and 2, 1 and 5, and 8
    /// and 1 elements, then one per call of `aligned`.
    const EXPECTED: &str = concat!(
        "value 20 41\n",
        "S 48 4 16 24 40\nidx 12 32\ncopy 1 2 3 14 0.25 932744692\ndiff 2 48\n",
        "O 168 56 64 160 14 9 77\nU 16 8\nT 12 12\nTD 16 4\nva 5 12 6 99 20 8\nloop 642\n",
        "P 5 5 17 99 4470\n48\n",
        "S 64 4 8 16 56\nidx 4 32\ncopy 1 2 3 0 1 1534126\ndiff 2 64\n",
        "O 216 72 80 208 0 9 77\nU 40 8\nT 4 4\nTD 16 4\nva 5 10 6 99 20 8\nloop 642\n",
        "P 3 5 17 99 4470\n64\n",
        "S 56 4 36 40 48\nidx 32 40\ncopy 1 2 3 49 0 2133126390\ndiff 2 56\n",
        "O 192 64 72 184 49 9 77\nU 8 8\nT 32 32\nTD 24 8\nva 5 17 6 99 20 8\nloop 642\n",
        "P 10 5 17 99 4470\n56\n",
        // `aligned` and packing after members of variable size, for 1, 3
        // and 6 elements.
        "A1 32 8 11\nA2 4 2 12\nA3 32 16 16 13\nA4 8 2 -2 14 1430862904\n",
        "A5 16 4 12 55 5 15\nA6 5 4 16\nA7 10 2 2 17\n",
        "A1 32 8 11\nA2 6 4 12\nA3 32 16 16 13\nA4 8 4 -2 14 5958200\n",
        "A5 32 4 20 55 5 15\nA6 5 4 16\nA7 12 2 4 17\n",
        "A1 32 8 11\nA2 8 6 12\nA3 32 16 16 13\nA4 8 7 -2 14 200\n",
        "A5 48 8 36 55 5 15\nA6 9 8 16\nA7 14 2 6 17\n",
    );

    #[test]
    fn variable_records_match_gcc_on_every_back_end() {
        // Windows targets lay records out as Microsoft does, which has no
        // run-time form here.
        if !cfg!(all(target_arch = "x86_64", target_os = "linux")) {
            eprintln!("skipping: the fixture runs on an x86-64 Linux host");
            return;
        }
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs/variable_records.c");
        let source = source.to_str().unwrap();
        for mode in [
            &["--interpret", "--opt=none"][..],
            &["--interpret", "--opt=bcc"],
        ] {
            let output = bcc(&[&[GCC, source][..], mode].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{mode:?}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), EXPECTED, "{mode:?}");
        }
        for mode in [&["-O0"][..], &["-O2"], &["--backend=native"]] {
            let executable = temporary("records.exe");
            let built =
                bcc(&[&[GCC, source, "-o", executable.to_str().unwrap()][..], mode].concat());
            assert!(built.status.success(), "{mode:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{mode:?}");
            assert_eq!(text(&run.stdout), EXPECTED, "{mode:?}");
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// Microsoft record layout keeps such a record unmodeled, refused by
    /// name rather than laid out as GCC would.
    #[test]
    fn microsoft_layout_refuses_variable_records() {
        let input = temporary("msvc.c");
        std::fs::write(
            &input,
            "int f(int n) { struct s { int a[n]; char c; } s; s.c = 1; return s.c; }\n",
        )
        .unwrap();
        let output = bcc(&[
            GCC,
            "--target=x86_64-w64-windows-gnu",
            "--emit=ir",
            input.to_str().unwrap(),
        ]);
        std::fs::remove_file(input).unwrap();
        assert!(!output.status.success());
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains("structures with variably modified members in Microsoft record layout"),
            "{stderr}"
        );
        assert!(!stderr.contains("internal error"), "{stderr}");
    }

    fn bcc(args: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH"),
        )
        .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "bcc-records-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
