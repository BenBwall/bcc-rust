#![expect(
    missing_docs,
    reason = "This integration test crate contains no public API."
)]
#![expect(
    unused_crate_dependencies,
    reason = "These regressions exercise the compiled CLI."
)]

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write as _,
        io::Read as _,
        path::PathBuf,
        process::{
            Command,
            Output,
            Stdio,
        },
        thread,
        time::{
            Duration,
            Instant,
        },
    };

    struct TemporaryInput(PathBuf);

    impl Drop for TemporaryInput {
        fn drop(&mut self) {
            drop(std::fs::remove_file(&self.0));
        }
    }

    fn run(source: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
        _ = command.args(["--syntax-tree", "--input", source]);
        run_command(&mut command)
    }

    fn run_command(command: &mut Command) -> Output {
        let mut child = command
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the parser CLI must start");
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let stdout_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let start = Instant::now();
        let (status, timed_out) = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break (status, false);
            }
            if start.elapsed() >= Duration::from_secs(5) {
                child.kill().expect("the stalled parser must be stopped");
                break (child.wait().unwrap(), true);
            }
            thread::sleep(Duration::from_millis(10));
        };
        let output = Output {
            status,
            stdout: stdout_reader.join().unwrap(),
            stderr: stderr_reader.join().unwrap(),
        };
        assert!(
            !timed_out,
            "parser did not finish in five seconds: {command:?}"
        );
        output
    }

    fn accepted(source: &str) -> String {
        let output = run(source);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{source:?}: {}", output.status);
        assert!(!stderr.contains("error:"), "{source:?}: {stderr}");
        stderr.into_owned()
    }

    #[test]
    fn torture_whitespace_is_insignificant_at_token_boundaries() {
        for whitespace in ["\t", "\x0b", "\x0c"] {
            let source = format!("int{whitespace}f(void) {{\n{whitespace}return 0;\n}}\n");
            drop(accepted(&source));
        }
    }

    #[test]
    fn torture_l_prefix_identifiers_remain_whole() {
        let tree = accepted("int Long; int L92; int L;\n");
        assert!(tree.contains("declarator Long"), "{tree}");
        assert!(tree.contains("declarator L92"), "{tree}");
    }

    #[test]
    fn torture_numeric_spellings_preserve_values() {
        for (literal, expected) in [
            ("1e3", "constant 1000 (double)"),
            ("1e-3", "constant 0.001 (double)"),
            ("1.5e-2", "constant 0.015 (double)"),
            ("0x1p3", "constant 8 (double)"),
            ("0x1p-3", "constant 0.125 (double)"),
        ] {
            let tree = accepted(&format!("double value = {literal};\n"));
            assert!(tree.contains(expected), "{literal}: {tree}");
        }
        drop(accepted("unsigned value = 0x89ABCDEF;\n"));
        for (literal, expected) in [
            ("0x1p3f", "constant 8 (float)"),
            ("0x1p-3f", "constant 0.125 (float)"),
        ] {
            let tree = accepted(&format!("float value = {literal};\n"));
            assert!(tree.contains(expected), "{literal}: {tree}");
        }
    }

    #[test]
    fn torture_escaped_backslash_character_keeps_its_closing_quote() {
        let tree = accepted("int value = '\\\\'; int after;\n");
        assert!(tree.contains("declarator after"), "{tree}");
    }

    #[test]
    fn torture_long_double_literal_survives_native_conversion() {
        let tree = accepted("long double value = 1.0L; int after;\n");
        assert!(tree.contains("constant 0x1p+0 (long double)"), "{tree}");
        assert!(tree.contains("declarator after"), "{tree}");
    }

    #[test]
    fn torture_multi_character_constants_retain_their_integer_value() {
        let tree = accepted("int value = 'ab';\n#if 'ab' == 24930\nint after;\n#endif\n");
        assert!(
            tree.contains("constant 24930 (int, multi-character)"),
            "{tree}"
        );
        assert!(tree.contains("declarator after"), "{tree}");
    }

    #[test]
    fn torture_nested_macro_arguments_use_the_callers_parameters() {
        for parameter in ["x", "y"] {
            let tree = accepted(&format!(
                "#define A({parameter}) {parameter}\n#define B(x) A(x)\nint value = B(3);\n"
            ));
            assert!(tree.contains("constant 3 (int)"), "{tree}");
        }
    }

    #[test]
    fn torture_whitespace_keeps_parenthesized_macros_object_like() {
        for whitespace in [" ", "\t", "/**/"] {
            let tree = accepted(&format!(
                "#define VALUE{whitespace}(3)\nint value = VALUE;\n"
            ));
            assert!(tree.contains("constant 3 (int)"), "{tree}");
        }
    }

    #[test]
    fn torture_macro_rescanning_disables_only_active_replacement_lists() {
        for source in [
            "#define value value\nint value;\n",
            "#define f(x) f(x)\nvoid g(void) { f(3); }\n",
            "#define f(x) x\nint value = f(f(3));\n",
        ] {
            drop(accepted(source));
        }
        let tree = accepted("#define f(x) x\nint value = f(f(3));\n");
        assert!(tree.contains("constant 3 (int)"), "{tree}");
    }

    #[test]
    fn torture_l_prefix_macros_expand_without_name_collisions() {
        let tree = accepted(
            "#define LIM1(x) int x;\n#define LIM2(x) LIM1(x##0) LIM1(x##1)\nLIM2(value)\n",
        );
        assert!(tree.contains("declarator value0"), "{tree}");
        assert!(tree.contains("declarator value1"), "{tree}");
        let tree = accepted(
            "#define LIM1(x) x##0; x##1;\n#define LIM2(x) LIM1(x##0) LIM1(x##1)\n#define LIM3(x) \
             LIM2(x##0) LIM2(x##1)\nLIM3(int value)\n",
        );
        assert!(tree.contains("declarator value000"), "{tree}");
        assert!(tree.contains("declarator value111"), "{tree}");
        let tree = accepted("#define CAT(a,b) a##b\nCAT(int num,ber=3;)\n");
        assert!(tree.contains("declarator number"), "{tree}");
        assert!(tree.contains("constant 3 (int)"), "{tree}");
    }

    #[test]
    fn torture_malformed_call_initializer_recovers_to_following_input() {
        for initializer in ["(int){x: 0}", "(struct {int x;}){x: 0}"] {
            let source = format!("void f(void) {{ g({initializer}); }} int after;\n");
            let output = run(&source);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{}", output.status);
            assert!(stderr.contains("error:"), "{stderr}");
            assert!(stderr.contains("declarator after"), "{stderr}");
            assert!(
                stderr.matches("error:").count() < 20,
                "recovery repeated diagnostics"
            );
        }
    }

    #[test]
    fn torture_long_label_chains_finish_or_report_a_resource_limit() {
        let path =
            std::env::temp_dir().join(format!("bcc-torture-labels-{}.c", std::process::id()));
        let mut labels = String::new();
        for i in 0..30_000 {
            write!(labels, "case {i}: ").unwrap();
        }
        std::fs::write(
            &path,
            format!("void f(int x) {{ switch(x) {{ {labels} break; }} }}\n"),
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
        let input = TemporaryInput(path);
        _ = command.arg(&input.0);
        let output = run_command(&mut command);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{}: {stderr}", output.status);
        assert!(
            !stderr.contains("error:") || stderr.contains("limit of"),
            "{stderr}"
        );
    }
}
