//! `-fsanitize=undefined` under `--gcc-mode` and `--clang-mode`: each check
//! reports through the library its compiler links, at the compiler's
//! file, line and column, with its type names and values; `-fsanitize-trap`
//! needs no library; and the options are read, and refused, in each
//! compiler's words. The expected reports are GCC 13.3's and Clang 23.1's for
//! the same programs.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use std::{
        os::unix::process::ExitStatusExt,
        path::PathBuf,
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    /// A program whose every check recovers, so that one run reports them
    /// all in order.
    const CHECKS: &str = r#"#include <limits.h>
#include <stdio.h>
#include <string.h>
struct S { int a; int b[4]; int c; };
__attribute__((nonnull)) int g(int *p) { return p == 0; }
__attribute__((returns_nonnull)) int *h(int *p) { return p; }
int main(int argc, char **argv) {
    int big = INT_MAX - 1 + argc, small = INT_MIN + argc - 1;
    int r = big + 1;
    r = small - 1;
    long m = 0x100000000L * argc;
    m = m * m;
    r = -small;
    big++;
    r = 1 << (31 + argc);
    r = -argc << 1;
    static long long buffer[4];
    int *p = (int *)((char *)buffer + argc);
    r = *p;
    struct S s = { 0 };
    int i = 3 + argc;
    r = s.b[i];
    _Bool b;
    unsigned char two = 1 + argc;
    memcpy(&b, &two, 1);
    r = b;
    int n = argc - 1;
    int v[n];
    r = __builtin_clz(argc - 1);
    char *q = (char *)0 + argc;
    r = g(argc > 5 ? &r : 0);
    r = h(argc > 5 ? &r : 0) != 0;
    double d = 1e10 * argc;
    r = (int)d;
    printf("%d %ld %d %zu %d %p\n", r, m, big, sizeof v, s.a, (void *)q);
    return 0;
}
"#;

    /// GCC's reports for [`CHECKS`] with `-fsanitize=float-cast-overflow`
    /// added, which its `undefined` lacks.
    const GCC: &[&str] = &[
        "checks.c:9:17: runtime error: signed integer overflow: 2147483647 + 1 cannot be \
         represented in type 'int'",
        "checks.c:10:15: runtime error: signed integer overflow: -2147483648 - 1 cannot be \
         represented in type 'int'",
        "checks.c:12:7: runtime error: signed integer overflow: 4294967296 * 4294967296 cannot be \
         represented in type 'long int'",
        "checks.c:13:9: runtime error: negation of -2147483648 cannot be represented in type \
         'int'; cast to an unsigned type to negate this value to itself",
        "checks.c:14:8: runtime error: signed integer overflow: 2147483647 + 1 cannot be \
         represented in type 'int'",
        "checks.c:15:11: runtime error: shift exponent 32 is too large for 32-bit type 'int'",
        "checks.c:16:15: runtime error: left shift of negative value -1",
        "checks.c:19:9: runtime error: load of misaligned address ADDR for type 'int', which \
         requires 4 byte alignment",
        "checks.c:22:12: runtime error: index 4 out of bounds for type 'int [4]'",
        "checks.c:26:7: runtime error: load of value 2, which is not a valid value for type \
         '_Bool'",
        "checks.c:28:9: runtime error: variable length array bound evaluates to non-positive \
         value 0",
        "checks.c:29:9: runtime error: passing zero to clz(), which is not a valid argument",
        "checks.c:31:9: runtime error: null pointer passed as argument 1, which is declared to \
         never be null",
        "checks.c:6:58: runtime error: null pointer returned from function declared to never \
         return null",
        "checks.c:34:5: runtime error: 1e+10 is outside the range of representable values of type \
         'int'",
    ];

    /// Clang's reports for [`CHECKS`]: it also reports the offset from a
    /// null pointer, which GCC allows.
    const CLANG: &[&str] = &[
        "checks.c:9:17: runtime error: signed integer overflow: 2147483647 + 1 cannot be \
         represented in type 'int'",
        "checks.c:10:15: runtime error: signed integer overflow: -2147483648 - 1 cannot be \
         represented in type 'int'",
        "checks.c:12:11: runtime error: signed integer overflow: 4294967296 * 4294967296 cannot \
         be represented in type 'long'",
        "checks.c:13:9: runtime error: negation of -2147483648 cannot be represented in type \
         'int'; cast to an unsigned type to negate this value to itself",
        "checks.c:14:8: runtime error: signed integer overflow: 2147483647 + 1 cannot be \
         represented in type 'int'",
        "checks.c:15:11: runtime error: shift exponent 32 is too large for 32-bit type 'int'",
        "checks.c:16:15: runtime error: left shift of negative value -1",
        "checks.c:19:9: runtime error: load of misaligned address ADDR for type 'int', which \
         requires 4 byte alignment",
        "checks.c:22:9: runtime error: index 4 out of bounds for type 'int[4]'",
        "checks.c:26:9: runtime error: load of value 2, which is not a valid value for type \
         '_Bool'",
        "checks.c:28:11: runtime error: variable length array bound evaluates to non-positive \
         value 0",
        "checks.c:29:23: runtime error: passing zero to __builtin_clz(), which is not a valid \
         argument",
        "checks.c:30:25: runtime error: applying non-zero offset 1 to null pointer",
        "checks.c:31:11: runtime error: null pointer passed as argument 1, which is declared to \
         never be null",
        "checks.c:6:51: runtime error: null pointer returned from function declared to never \
         return null",
        "checks.c:34:9: runtime error: 1e+10 is outside the range of representable values of type \
         'int'",
    ];

    /// Each check reports as the compiler's own build of the program does,
    /// through the LLVM back end and the native one; the run goes on after
    /// each report, and a program without undefined behavior reports nothing.
    #[test]
    fn checks_report_as_each_compiler() {
        let directory = Directory::new();
        directory.write("checks.c", CHECKS);
        directory.write(
            "clean.c",
            "#include <stdlib.h>\nint g[4];\nint main(int argc, char **argv) {\n    int *p = \
             malloc(sizeof *p);\n    *p = argc << 30;\n    for (int i = 0; i < 4; i++)\n        \
             g[i] = *p / (i + 1);\n    free(p);\n    return g[3] - g[3];\n}\n",
        );
        for (mode, expected) in [("--gcc-mode", GCC), ("--clang-mode", CLANG)] {
            for back_end in [&[][..], &["-Xbcc", "--backend=native"]] {
                let mut args = vec![
                    "-fsanitize=undefined",
                    "-fsanitize=float-cast-overflow",
                    "checks.c",
                    "-o",
                    "checks",
                ];
                args.extend_from_slice(back_end);
                let build = directory.bcc(mode, &args);
                // A system without GCC's libubsan, or a bundled toolchain
                // without compiler-rt, cannot link the report; the trap test
                // below checks the code without either.
                if !build.status.success() && stderr(&build).contains("ubsan") {
                    continue;
                }
                succeeds(&build);
                let run = Command::new(directory.path("checks")).output().unwrap();
                assert!(
                    run.status.success(),
                    "{mode} {back_end:?}: {}",
                    stderr(&run)
                );
                let reports: Vec<String> = stderr(&run)
                    .lines()
                    .filter(|line| line.contains("runtime error"))
                    .map(masked)
                    .collect();
                assert_eq!(reports, expected, "{mode} {back_end:?}");
                let mut args = vec!["-fsanitize=undefined", "clean.c", "-o", "clean"];
                args.extend_from_slice(back_end);
                succeeds(&directory.bcc(mode, &args));
                let run = Command::new(directory.path("clean")).output().unwrap();
                assert!(run.status.success(), "{mode} {back_end:?}");
                assert_eq!(stderr(&run), "", "{mode} {back_end:?}");
            }
        }
    }

    /// `-fno-sanitize-recover` ends the program at the first report, and
    /// `unreachable` always does; a division by zero reports before the
    /// division runs.
    #[test]
    fn checks_that_end_the_program() {
        let directory = Directory::new();
        directory.write(
            "abort.c",
            "#include <limits.h>\nint main(int argc, char **argv) {\n    int x = INT_MAX - 1 + \
             argc;\n    x = x + 1;\n    return x + 1;\n}\n",
        );
        directory.write(
            "unreachable.c",
            "int f(int x) {\n    if (x)\n        return x;\n    __builtin_unreachable();\n}\nint \
             main(int argc, char **argv) { return f(argc - 1); }\n",
        );
        directory.write(
            "divide.c",
            "int main(int argc, char **argv) { return 7 / (argc - 1); }\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let build = |args: &[&str]| directory.bcc(mode, args);
            let args = [
                "-fsanitize=undefined",
                "-fno-sanitize-recover=undefined",
                "abort.c",
                "-o",
                "abort",
            ];
            let output = build(&args);
            if !output.status.success() && stderr(&output).contains("ubsan") {
                continue;
            }
            succeeds(&output);
            let run = Command::new(directory.path("abort")).output().unwrap();
            assert_eq!(run.status.code(), Some(1), "{mode}");
            assert_eq!(
                stderr(&run)
                    .lines()
                    .filter(|line| line.contains("runtime error"))
                    .count(),
                1,
                "{mode}"
            );
            succeeds(&build(&[
                "-fsanitize=unreachable",
                "unreachable.c",
                "-o",
                "unreachable",
            ]));
            let run = Command::new(directory.path("unreachable"))
                .output()
                .unwrap();
            assert!(
                stderr(&run).contains(
                    "unreachable.c:4:5: runtime error: execution reached an unreachable program \
                     point"
                ),
                "{mode}: {}",
                stderr(&run)
            );
            assert!(!run.status.success(), "{mode}");
            succeeds(&build(&[
                "-fsanitize=undefined",
                "divide.c",
                "-o",
                "divide",
            ]));
            let run = Command::new(directory.path("divide")).output().unwrap();
            assert!(
                stderr(&run).contains("divide.c:1:44: runtime error: division by zero"),
                "{mode}: {}",
                stderr(&run)
            );
        }
    }

    /// `-fsanitize-trap` runs the `trap` instruction and links no library;
    /// `-fsanitize-undefined-trap-on-error` is the same, and
    /// `-fno-sanitize=` takes a check back.
    #[test]
    fn trapping_checks_need_no_library() {
        let directory = Directory::new();
        directory.write(
            "add.c",
            "#include <limits.h>\nint main(int argc, char **argv) {\n    int x = INT_MAX - 1 + \
             argc;\n    return x + 1 > 0;\n}\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            for options in [
                &["-fsanitize=undefined", "-fsanitize-trap=undefined"][..],
                &[
                    "-fsanitize=signed-integer-overflow",
                    "-fsanitize-undefined-trap-on-error",
                ],
                &["-fsanitize=undefined", "-fsanitize-trap"],
            ] {
                let args = [
                    options,
                    &["add.c", "-o", "add", "-Xbcc", "--backend=native"],
                ]
                .concat();
                for args in [&args[..args.len() - 2], &args[..]] {
                    succeeds(&directory.bcc(mode, args));
                    let run = Command::new(directory.path("add")).output().unwrap();
                    assert_eq!(run.status.signal(), Some(4), "{mode} {args:?}");
                    assert_eq!(stderr(&run), "", "{mode} {args:?}");
                }
            }
            let args = [
                "-fsanitize=undefined",
                "-fsanitize-trap=undefined",
                "-fno-sanitize=signed-integer-overflow",
                "add.c",
                "-o",
                "add",
            ];
            succeeds(&directory.bcc(mode, &args));
            let run = Command::new(directory.path("add")).output().unwrap();
            assert!(run.status.success(), "{mode}");
        }
    }

    /// GCC's and Clang's errors for a name the compiler does not take, bcc's
    /// for a sanitizer it lacks; the groups and lists bcc has, with
    /// `AddressSanitizer` too; `no_sanitize`; and Clang's feature test.
    #[test]
    fn options_are_read_as_each_compiler_reads_them() {
        let directory = Directory::new();
        directory.write("a.c", "int f(int x) { return x + 1; }\n");
        directory.write(
            "n.c",
            "__attribute__((no_sanitize(\"undefined\"))) int f(int x) { return x + 1; \
             }\n__attribute__((no_sanitize(\"signed-integer-overflow\"))) int g(int x) { return x \
             + 1; }\nint h(int x) { return x + 1; }\n",
        );
        directory.write(
            "m.c",
            "#if __has_feature(undefined_behavior_sanitizer)\nint ubsan;\n#endif\n",
        );
        let error = |mode: &str, option: &str| {
            let output = directory.bcc(mode, &[option, "-c", "a.c", "-o", "a.o"]);
            assert!(!output.status.success(), "{mode} {option}");
            stderr(&output)
        };
        let gcc = |option| error("--gcc-mode", option);
        let clang = |option| error("--clang-mode", option);
        assert!(
            gcc("-fsanitize=undefined,foo")
                .contains("unrecognized argument to '-fsanitize=' option: 'foo'")
        );
        assert!(
            clang("-fsanitize=foo").contains("unsupported argument 'foo' to option '-fsanitize='")
        );
        assert!(gcc("-fsanitize=all").contains("'-fsanitize=all' option is not valid"));
        assert!(
            gcc("-fsanitize-recover=unreachable")
                .contains("'-fsanitize-recover=unreachable' is not supported")
        );
        assert!(
            clang("-fsanitize-recover=return")
                .contains("unsupported argument 'return' to option '-fsanitize-recover='")
        );
        assert!(gcc("-fsanitize=function").contains("unrecognized argument"));
        assert!(clang("-fsanitize=function").contains("is not supported"));
        assert!(clang("-fsanitize=memory").contains("is not supported"));
        assert!(gcc("-fsanitize=bounds-strict").contains("is not supported"));
        assert!(clang("-fsanitize-minimal-runtime").contains("is not supported"));
        for mode in ["--gcc-mode", "--clang-mode"] {
            for options in [
                &["-fsanitize=undefined,address"][..],
                &["-fsanitize=shift,null", "-fno-sanitize=all"],
                &[
                    "-fsanitize=undefined",
                    "-fno-sanitize-recover",
                    "-fsanitize-recover=null",
                ],
                &["-fsanitize=undefined", "-fsanitize=thread"],
            ] {
                let args = [options, &["-c", "a.c", "-o", "a.o"]].concat();
                succeeds(&directory.bcc(mode, &args));
            }
            let args = ["-fsanitize=undefined", "-S", "n.c", "-o", "-"];
            let assembly = stdout(&directory.bcc(mode, &args));
            let checked: Vec<&str> = assembly
                .split("\n.Lfunc_end")
                .filter(|function| function.contains("__ubsan_handle_add_overflow"))
                .filter_map(|function| {
                    ["f:", "g:", "h:"]
                        .into_iter()
                        .find(|label| function.contains(&format!("\n{label}\n")))
                })
                .collect();
            assert_eq!(checked, ["h:"], "{mode}");
            let defined = |options: &[&str]| {
                let args = [options, &["-E", "-P", "m.c"]].concat();
                stdout(&directory.bcc(mode, &args)).trim().to_owned()
            };
            // GCC 14's `__has_feature` answers as Clang's does.
            assert_eq!(defined(&["-fsanitize=undefined"]), "int ubsan;", "{mode}");
            assert_eq!(defined(&[]), "", "{mode}");
        }
    }

    /// `line` with each hexadecimal address replaced by `ADDR`.
    fn masked(line: &str) -> String {
        let mut text = String::new();
        let mut rest = line;
        while let Some(start) = rest.find("0x") {
            text.push_str(&rest[..start]);
            let digits = rest[start + 2..]
                .find(|c: char| !c.is_ascii_hexdigit())
                .unwrap_or(rest.len() - start - 2);
            text.push_str("ADDR");
            rest = &rest[start + 2 + digits..];
        }
        text.push_str(rest);
        text
    }

    /// A fresh working directory, removed afterwards.
    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bcc-ubsan-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).unwrap();
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// Runs bcc in this directory with `mode` first.
        fn bcc(&self, mode: &str, args: &[&str]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .arg(mode)
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("UBSAN_OPTIONS")
                .output()
                .unwrap()
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn succeeds(output: &Output) {
        assert!(
            output.status.success(),
            "{}\n{}",
            stdout(output),
            stderr(output)
        );
    }
}
