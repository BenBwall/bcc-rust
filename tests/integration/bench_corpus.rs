//! The benchmark corpus in `benches/c` stays runnable: every program, built
//! with `-DBENCH_REDUCED`, runs in the IR interpreter with and without the
//! bcc optimizer, passes its own checksum (exit status 0), and prints the same
//! output either way. `scripts/compare_optimizers.py` times the full-size
//! programs.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Test harnesses own process output and file paths outside compilation."
)]
mod tests {
    use std::{
        env,
        path::PathBuf,
        process::{
            Command,
            Output,
        },
    };

    /// Runs the interpreter on a reduced program, with `flags` added.
    fn interpret(program: &PathBuf, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .arg(program)
            .args([
                "--target=x86_64-w64-windows-gnu",
                "-DBENCH_REDUCED",
                "--interpret",
            ])
            .args(flags)
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }

    #[test]
    fn every_program_passes_its_own_check_in_the_interpreter() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/c");
        let mut programs: Vec<PathBuf> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
            .collect();
        programs.sort();
        assert!(programs.len() >= 6, "{programs:?}");
        for program in &programs {
            let plain = interpret(program, &[]);
            let name = program.display();
            assert_eq!(
                plain.status.code(),
                Some(0),
                "{name}: {}",
                String::from_utf8_lossy(&plain.stderr)
            );
            assert!(!plain.stdout.is_empty(), "{name} printed nothing");
            let optimized = interpret(program, &["--opt=bcc"]);
            assert_eq!(
                optimized.status.code(),
                Some(0),
                "{name} with --opt=bcc: {}",
                String::from_utf8_lossy(&optimized.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&plain.stdout),
                String::from_utf8_lossy(&optimized.stdout),
                "{name}: the optimizer changed the output"
            );
        }
    }
}
