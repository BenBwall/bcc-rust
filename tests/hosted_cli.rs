//! The execution environment, header search order, resource-header chaining
//! and compiler identity, exercised through the CLI.

#![expect(
    unused_crate_dependencies,
    reason = "CLI integration tests exercise the compiled binary."
)]

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Tests own command output and generated sources outside compilation."
)]
mod tests {
    use std::process::{
        Command,
        Output,
    };

    fn bcc(args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(args)
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .unwrap()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn clean(output: &Output) {
        let text = stderr(output);
        assert!(
            output.status.success() && !text.contains("error:") && !text.contains("warning:"),
            "{text}"
        );
    }

    /// The identifiers in a `--tokens` dump, in order.
    fn identifiers(output: &Output) -> Vec<String> {
        stderr(output)
            .lines()
            .filter_map(|line| line.split_once("identifier `"))
            .map(|(_, rest)| rest.trim_end_matches('`').to_owned())
            .collect()
    }

    #[test]
    fn execution_environment_is_hosted_unless_freestanding_and_the_last_flag_wins() {
        let source =
            "#if __STDC_HOSTED__ == 1\nhosted\n#elif __STDC_HOSTED__ == 0\nfreestanding\n#endif\n";
        for (flags, expected) in [
            (&[][..], "hosted"),
            (&["-ffreestanding"][..], "freestanding"),
            (&["-fhosted"][..], "hosted"),
            (&["-ffreestanding", "-fhosted"][..], "hosted"),
            (&["-fhosted", "-ffreestanding"][..], "freestanding"),
            (
                &["-std=c89", "-pedantic-errors", "-ffreestanding"][..],
                "freestanding",
            ),
        ] {
            let mut args = flags.to_vec();
            args.extend(["--tokens", "--input", source]);
            let output = bcc(&args);
            clean(&output);
            assert_eq!(identifiers(&output), [expected], "{flags:?}");
        }
    }

    #[test]
    fn main_signature_is_checked_only_in_a_hosted_environment() {
        let source = "void main(void) {}\n";
        let hosted = bcc(&["--input", source]);
        assert!(
            stderr(&hosted).contains("main has a nonportable signature"),
            "{hosted:?}"
        );
        clean(&bcc(&["-ffreestanding", "--input", source]));
    }
}
