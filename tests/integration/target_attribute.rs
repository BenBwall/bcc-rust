//! x86 `target` attributes through the command line: each compiler's reading
//! of the text, measured from GCC 14.2 and, where the pinned Clang is beside
//! the build, compared with it live; the calls both compilers refuse across
//! features; the features LLVM gets; redeclarations; and bcc's inliner.
//! `x86_intrinsics.rs` runs programs that choose such functions at run time.

#[cfg(test)]
#[cfg(target_arch = "x86_64")]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        fs,
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

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn source(text: &str) -> PathBuf {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "bcc-target-attribute-{}-{number}.c",
            std::process::id()
        ));
        fs::write(&path, text).unwrap();
        path
    }

    fn bcc(args: &[&str], text: &str) -> Output {
        let path = source(text);
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["--target=x86_64-unknown-linux-gnu"])
            .args(args)
            .arg(&path)
            .env("NO_COLOR", "1")
            .output()
            .expect("the bcc-rust test binary must run");
        fs::remove_file(path).unwrap();
        output
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// The first diagnostic's severity and message, without its location.
    fn first_diagnostic(output: &Output) -> Option<String> {
        stderr(output).lines().find_map(|line| {
            ["error: ", "warning: "]
                .iter()
                .find_map(|kind| line.find(kind).map(|at| line[at..].to_owned()))
        })
    }

    fn attribute(text: &str) -> String {
        format!("__attribute__((target(\"{text}\"))) int f(int x) {{ return x; }}\n")
    }

    /// What each compiler says of a text: GCC 14.2's measured error, and
    /// Clang 23's warning, after which it ignores the attribute.
    const TEXTS: &[(&str, Option<&str>, Option<&str>)] = &[
        ("avx2,no-bmi,arch=haswell,tune=znver3", None, None),
        (
            "fpmath=sse,prefer-vector-width=256,cld",
            None,
            Some("unsupported 'fpmath='"),
        ),
        (
            " avx2",
            Some("attribute 'target' argument ' avx2' is unknown"),
            None,
        ),
        (
            "foo",
            Some("attribute 'target' argument 'foo' is unknown"),
            Some("unsupported 'foo'"),
        ),
        (
            "no-foo",
            Some("attribute 'target' argument 'no-foo' is unknown"),
            Some("unsupported 'foo'"),
        ),
        (
            "avx2,,bmi",
            Some("attribute 'target' argument '' is unknown"),
            Some("unsupported ''"),
        ),
        (
            "arch=foo",
            Some("bad value 'foo' for 'target(\"arch=\")' attribute"),
            Some("unknown CPU 'foo'"),
        ),
        (
            "tune=foo",
            Some("bad value 'foo' for 'target(\"tune=\")' attribute"),
            Some("unknown tune CPU 'foo'"),
        ),
        (
            "arch=haswell,arch=skylake",
            Some("attribute value 'arch=skylake' was already specified in 'target' attribute"),
            Some("duplicate 'arch='"),
        ),
        ("3dnow", None, Some("unsupported '3dnow'")),
        (
            "x87,general-regs-only",
            Some("attribute 'target' argument 'x87' is unknown"),
            None,
        ),
        (
            "no-general-regs-only",
            Some(
                "pragma or attribute 'target(\"general-regs-only\")' does not allow a negated form",
            ),
            None,
        ),
        (
            "",
            Some("empty string in attribute 'target'"),
            Some("unsupported ''"),
        ),
    ];

    #[test]
    fn texts_are_read_as_each_compiler_reads_them() {
        let clang = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/llvm/bin/clang");
        for &(text, gcc, clang_words) in TEXTS {
            let program = attribute(text);
            let gcc_output = bcc(&["--gcc-mode", "-fsyntax-only"], &program);
            let expected = gcc.map(|words| {
                let kind = if text.is_empty() { "warning" } else { "error" };
                format!("{kind}: {words}")
            });
            let first = first_diagnostic(&gcc_output);
            assert_eq!(
                first
                    .as_deref()
                    .map(|line| line.split(" [-W").next().unwrap()),
                expected.as_deref(),
                "GCC {text:?}"
            );
            let clang_output = bcc(&["--clang-mode", "-fsyntax-only"], &program);
            let expected = clang_words.map(|words| {
                format!(
                    "warning: {words} in the 'target' attribute string; 'target' attribute \
                     ignored [-Wignored-attributes]"
                )
            });
            assert_eq!(first_diagnostic(&clang_output), expected, "Clang {text:?}");
            if clang.exists() {
                let path = source(&program);
                let reference = Command::new(&clang)
                    .args(["--target=x86_64-unknown-linux-gnu", "-fsyntax-only"])
                    .arg(&path)
                    .output()
                    .unwrap();
                fs::remove_file(path).unwrap();
                assert_eq!(
                    first_diagnostic(&reference),
                    expected,
                    "pinned Clang {text:?}"
                );
            }
        }
    }

    #[test]
    fn arguments_and_subjects_follow_each_compiler() {
        let program = "__attribute__((target(\"avx2\", \"bmi2\"))) int f(int x) { return x; \
                       }\n__attribute__((target(\"avx2\"))) int v;\n";
        let gcc = bcc(&["--gcc-mode", "-fsyntax-only"], program);
        assert!(gcc.status.success(), "{}", stderr(&gcc));
        assert!(
            stderr(&gcc).contains("warning: attribute ignored"),
            "{}",
            stderr(&gcc)
        );
        let clang = bcc(&["--clang-mode", "-fsyntax-only"], program);
        assert!(!clang.status.success());
        assert!(stderr(&clang).contains("error: 'target' attribute takes one argument"));
        assert!(stderr(&clang).contains("error: 'target' attribute only applies to functions"));
        let gcc = bcc(
            &["--gcc-mode", "-fsyntax-only"],
            "__attribute__((target)) int f(void);\n",
        );
        assert!(
            stderr(&gcc).contains("wrong number of arguments specified for 'target' attribute")
        );
    }

    /// What both compilers accept: a later part that turns a baseline
    /// feature back on (and GCC's `arch=`, which forgets earlier parts),
    /// an encoding-prefixed string, read as its characters, and in GCC an
    /// argument of any kind on what is not a function, which it ignores.
    #[test]
    fn accepted_attributes_compile_as_both_compilers_read_them() {
        let function = |text: &str| {
            format!("__attribute__((target({text}))) int f(int x) {{ return x + 1; }}\n")
        };
        for mode in ["--gcc-mode", "--clang-mode"] {
            for text in [
                "\"no-sse2,sse2\"",
                "\"no-sse,sse2\"",
                "L\"avx2\"",
                "u8\"avx2\"",
            ] {
                let output = bcc(&[mode, "-c", "-o", "/dev/null"], &function(text));
                assert!(
                    output.status.success(),
                    "{mode} {text}: {}",
                    stderr(&output)
                );
            }
            let output = bcc(
                &[mode, "-c", "-o", "/dev/null"],
                &function("\"no-sse2,sse\""),
            );
            assert!(!output.status.success(), "{mode}: SSE2 stays off");
        }
        let output = bcc(
            &["--gcc-mode", "-c", "-o", "/dev/null"],
            &function("\"no-sse2,arch=haswell\""),
        );
        assert!(output.status.success(), "{}", stderr(&output));
        let output = bcc(
            &["--clang-mode", "-c", "-o", "/dev/null"],
            &function("L\"avx2\""),
        );
        assert!(
            stderr(&output)
                .contains("encoding prefix on an unevaluated string literal has no effect"),
            "{}",
            stderr(&output)
        );
        for program in [
            "int v __attribute__((target(42)));\n",
            "typedef int T __attribute__((target(1)));\n",
            "int v __attribute__((target(\"bogus\")));\n",
        ] {
            let output = bcc(&["--gcc-mode", "-fsyntax-only"], program);
            assert!(output.status.success(), "{program}: {}", stderr(&output));
            assert!(
                stderr(&output).contains("ignored"),
                "{program}: {}",
                stderr(&output)
            );
        }
        let output = bcc(
            &["--gcc-mode", "-fsyntax-only"],
            "int v __attribute__((target()));\n",
        );
        assert!(
            stderr(&output).contains("wrong number of arguments specified for 'target' attribute"),
            "{}",
            stderr(&output)
        );
    }

    /// An `always_inline` function or builtin that needs a feature the
    /// calling function lacks, as measured: both compilers refuse it in
    /// emitted code at every level, whichever of the two has the attribute.
    /// Freestanding, `<immintrin.h>` needs no C library headers, which
    /// Windows hosts lack.
    #[test]
    fn calls_across_features_are_refused_in_emitted_code() {
        let program = "#include <immintrin.h>\n#define AI static inline \
                       __attribute__((always_inline))\nAI __attribute__((target(\"avx2\"))) int \
                       wide(int x) { return x + 1; }\nAI int plain(int x) { return x + 1; \
                       }\nstatic int unused(int x) { return wide(x); }\nint dead(int x) { if (0) \
                       return wide(x); return x; }\n__attribute__((target(\"arch=haswell\"))) int \
                       fine(int x) { return wide(x) + plain(x); }\nint caller(int x) { return \
                       wide(x); }\n__attribute__((target(\"no-avx\"))) int narrow(int x) { return \
                       plain(x); }\n__m256i add(__m256i *a) { return _mm256_add_epi32(*a, *a); \
                       }\ntypedef long long v2 __attribute__((vector_size(16)));\nv2 round(v2 *a) \
                       { return __builtin_ia32_aesenc128(*a, *a); }\n";
        for level in ["-O0", "-O2"] {
            let clang = bcc(
                &[
                    "--clang-mode",
                    "-ffreestanding",
                    "-mavx",
                    level,
                    "-c",
                    "-o",
                    "/dev/null",
                ],
                program,
            );
            let text = stderr(&clang);
            for expected in [
                "error: always_inline function 'wide' requires target feature 'avx2', but would \
                 be inlined into function 'caller' that is compiled without support for 'avx2'",
                "error: always_inline function 'plain' requires target feature 'avx', but would \
                 be inlined into function 'narrow' that is compiled without support for 'avx'",
                "error: always_inline function '_mm256_add_epi32' requires target feature 'avx2'",
                "error: '__builtin_ia32_aesenc128' needs target feature aes",
            ] {
                assert!(text.contains(expected), "{level}: {expected}\n{text}");
            }
            assert_eq!(text.matches("error:").count(), 4, "{level}: {text}");
            let gcc = bcc(
                &[
                    "--gcc-mode",
                    "-ffreestanding",
                    "-mavx",
                    level,
                    "-c",
                    "-o",
                    "/dev/null",
                ],
                program,
            );
            let text = stderr(&gcc);
            assert!(
                text.contains(
                    "error: inlining failed in call to 'always_inline' 'wide': target specific \
                     option mismatch"
                ),
                "{level}: {text}"
            );
            assert!(text.contains("'__builtin_ia32_aesenc128' needs isa option -maes"));
        }
    }

    /// The LLVM IR of `program` under `args`, where `--gcc-mode` and
    /// `--clang-mode` name the identity alone, since the personalities take
    /// only their compiler's options.
    fn llvm(args: &[&str], program: &str) -> String {
        let args: Vec<&str> = args
            .iter()
            .map(|&arg| match arg {
                | "--gcc-mode" => "--compiler-identity=gcc",
                | "--clang-mode" => "--compiler-identity=clang",
                | arg => arg,
            })
            .collect();
        let output = bcc(&[&args[..], &["--emit=llvm"]].concat(), program);
        assert!(output.status.success(), "{}", stderr(&output));
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// The attributes of the function `name` in LLVM IR `text`.
    fn attributes_of(text: &str, name: &str) -> String {
        let define = text
            .lines()
            .find(|line| line.starts_with("define") && line.contains(&format!("@{name}(")))
            .unwrap_or_else(|| panic!("no @{name} in\n{text}"));
        let Some(group) = define.rsplit(' ').find(|word| word.starts_with('#')) else {
            return String::new();
        };
        text.lines()
            .find(|line| line.starts_with(&format!("attributes {group} =")))
            .unwrap()
            .to_owned()
    }

    #[test]
    fn llvm_gets_each_function_its_processor_and_features() {
        let program = "__attribute__((target(\"bmi2,arch=haswell,tune=znver3\"))) unsigned \
                       f(unsigned x, unsigned n) { return x >> n; \
                       }\n__attribute__((target(\"no-avx2\"))) int g(int x) { return x; }\nint \
                       h(int x) { return x; }\n";
        let text = llvm(&["--clang-mode", "-march=x86-64-v3"], program);
        let f = attributes_of(&text, "f");
        assert!(f.contains("\"target-cpu\"=\"haswell\""), "{f}");
        assert!(f.contains("\"tune-cpu\"=\"znver3\""), "{f}");
        assert!(
            f.contains("+avx2,") && f.contains("+bmi2,") && f.contains("+movbe,"),
            "{f}"
        );
        let g = attributes_of(&text, "g");
        assert!(
            g.contains("-avx2,") && g.contains("+avx,") && !g.contains("target-cpu"),
            "{g}"
        );
        assert!(!attributes_of(&text, "h").contains("target-features"));
        // GCC's `arch=` forgets the features turned on before it.
        let program = "__attribute__((target(\"avx2,arch=core2\"))) int f(int x) { return x; }\n";
        assert!(!attributes_of(&llvm(&["--gcc-mode"], program), "f").contains("avx2"));
        assert!(attributes_of(&llvm(&["--clang-mode"], program), "f").contains("+avx2"));
        // The BMI2 instruction for the shift.
        let output = bcc(
            &["-O2", "-S", "-o", "-"],
            "__attribute__((target(\"bmi2\"))) unsigned f(unsigned x, unsigned n) { return x >> \
             n; }\n",
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("shrx"));
    }

    #[test]
    fn redeclarations_merge_as_each_compiler_does() {
        // Both: a definition without the attribute keeps an earlier one's.
        let inherited =
            "__attribute__((target(\"bmi2\"))) int f(int);\nint f(int x) { return x; }\n";
        for mode in ["--gcc-mode", "--clang-mode"] {
            assert!(
                attributes_of(&llvm(&[mode], inherited), "f").contains("+bmi2"),
                "{mode}"
            );
        }
        // GCC: a later declaration's replace an earlier one's, after the
        // definition too, and one declaration's are read in turn.
        let replaced = "__attribute__((target(\"bmi2\"))) int \
                        f(int);\n__attribute__((target(\"lzcnt\"))) int f(int x) { return x; \
                        }\n__attribute__((target(\"popcnt\"))) int \
                        f(int);\n__attribute__((target(\"bmi\"), target(\"movbe\"))) int g(int x) \
                        { return x; }\n";
        let text = llvm(&["--gcc-mode"], replaced);
        let f = attributes_of(&text, "f");
        assert!(
            f.contains("+popcnt") && !f.contains("bmi2") && !f.contains("lzcnt"),
            "{f}"
        );
        let g = attributes_of(&text, "g");
        assert!(g.contains("+bmi,") && g.contains("+movbe"), "{g}");
        // Clang: a declaration keeps its first; another text on a
        // redeclaration is multiversioning, which bcc refuses.
        let first =
            "__attribute__((target(\"bmi\"), target(\"movbe\"))) int g(int x) { return x; }\n";
        let g = attributes_of(&llvm(&["--clang-mode"], first), "g");
        assert!(g.contains("+bmi") && !g.contains("movbe"), "{g}");
        let versions = "__attribute__((target(\"avx2\"))) int \
                        f(int);\n__attribute__((target(\"bmi2\"))) int f(int x) { return x; }\n";
        let output = bcc(&["--clang-mode", "-c", "-o", "/dev/null"], versions);
        assert!(
            stderr(&output).contains("function multiversioning attributes"),
            "{}",
            stderr(&output)
        );
        // Clang ignores an attribute after the definition.
        let late = "int f(int x) { return x; }\n__attribute__((target(\"bmi2\"))) int f(int);\n";
        let output = bcc(&["--clang-mode", "-fsyntax-only"], late);
        assert!(stderr(&output).contains("attribute declaration must precede definition"));
    }

    /// bcc's inliner expands a callee only into a caller with its features.
    #[test]
    fn the_inliner_keeps_callees_with_more_features() {
        let program = "__attribute__((target(\"avx2\"))) static int wide(int x) { return x * 3; \
                       }\nstatic int plain(int x) { return x * 5; }\nint narrow(int x) { return \
                       wide(x) + plain(x); }\n__attribute__((target(\"avx2,bmi2\"))) int \
                       broad(int x) { return wide(x); }\n";
        let output = bcc(&["--opt=bcc", "--emit=ir"], program);
        let text = String::from_utf8_lossy(&output.stdout);
        let body = |name: &str| {
            text.split("function @")
                .find(|function| function.starts_with(&format!("{name}(")))
                .unwrap_or_else(|| panic!("no @{name} in\n{text}"))
                .to_owned()
        };
        assert!(body("narrow").contains("call @wide"), "{text}");
        assert!(!body("narrow").contains("call @plain"), "{text}");
        assert!(!body("broad").contains("call @wide"), "{text}");
        // A function turning off SSE2 is refused rather than miscompiled.
        let output = bcc(
            &["-c", "-o", "/dev/null"],
            "__attribute__((target(\"no-sse2\"))) int f(int x) { return x; }\n",
        );
        assert!(
            stderr(&output).contains("turns off x87, MMX, FXSR, SSE or SSE2"),
            "{}",
            stderr(&output)
        );
    }
}
