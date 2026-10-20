//! Function-local storage retention and initializer reachability (issue #104).

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Integration tests own compiler processes and temporary files."
)]
mod tests {
    use std::{
        collections::BTreeSet,
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

    #[test]
    fn local_statics_at_o0_follow_the_emitted_function() {
        let source_file = temporary("source.c");
        let object = temporary("object.o");
        for (prefix, called, emitted) in [
            ("", false, true),
            ("static ", false, false),
            ("static __attribute__((noinline)) ", true, true),
            ("inline ", false, false),
            ("inline ", true, false),
            ("static inline ", false, false),
            ("static inline ", true, true),
            ("__attribute__((used)) static ", false, true),
        ] {
            for (storage, attribute) in [
                ("static const", ""),
                ("static const", "used,"),
                ("static _Thread_local const", ""),
                ("static __thread const", ""),
            ] {
                // Const isolates emission from C99 §6.7.4p3's constraint
                // against modifiable statics in external inline definitions.
                let caller = if called {
                    "int g(void){return f();}"
                } else {
                    ""
                };
                let source = format!(
                    "{prefix}int f(void){{ {storage} int b[4] \
                     __attribute__(({attribute}section(\".bss.t\")))={{1}}; return 0; }} {caller}"
                );
                std::fs::write(&source_file, &source).unwrap();
                let reference = Command::new(clang())
                    .args(["--target=x86_64-unknown-linux-gnu", "-std=c99", "-O0", "-c"])
                    .arg(&source_file)
                    .arg("-o")
                    .arg(&object)
                    .output()
                    .unwrap();
                assert_eq!(
                    reference.status.success(),
                    !emitted,
                    "{source}\n{}",
                    text(&reference.stderr)
                );
                if object.exists() {
                    std::fs::remove_file(&object).unwrap();
                }
                for backend in [
                    &[][..],
                    &["--backend=native"][..],
                    &["--backend=native", "--native-object"][..],
                ] {
                    let mut flags = vec![
                        "--target=x86_64-unknown-linux-gnu",
                        "-std=c99",
                        "-O0",
                        "-c",
                        "-o",
                        object.to_str().unwrap(),
                    ];
                    flags.extend_from_slice(backend);
                    let output = bcc(&source, &flags);
                    assert_eq!(
                        output.status.success(),
                        !emitted,
                        "{backend:?}: {source}\n{}",
                        text(&output.stderr)
                    );
                    assert_eq!(object.exists(), !emitted);
                    if emitted {
                        let diagnostic = text(&output.stderr);
                        assert!(
                            diagnostic.contains("BSS section '.bss.t' cannot have non-zero bytes"),
                            "{diagnostic}"
                        );
                        assert!(diagnostic.contains(":1:"), "{diagnostic}");
                    } else {
                        std::fs::remove_file(&object).unwrap();
                    }
                }
            }
        }
        std::fs::remove_file(source_file).unwrap();
    }

    #[test]
    fn local_static_untaken_statement_arms_match_clang() {
        let source_file = temporary("untaken.c");
        let object = temporary("untaken.o");
        for (body, emitted) in [
            ("if(0){STATIC return b[0];} return 0;", false),
            ("const int c=0; if(c){STATIC} return 0;", false),
            ("if(__builtin_expect(0,1)){STATIC} return 0;", false),
            ("int x; if(__builtin_constant_p(x)){STATIC} return 0;", true),
            ("if(1){}else{STATIC return b[0];} return 0;", false),
            ("if(0){if(1){STATIC}} return 0;", false),
            ("if(0){STATIC} if(1){} return 0;", false),
            ("if(0){STATIC} {static int zero;} return 0;", false),
            ("if(0){STATIC} else {STATIC} return 0;", true),
            ("if(0){STATIC label:;} return 0;", true),
            ("while(0){STATIC} return 0;", true),
            ("return 0; STATIC", true),
            ("if(1)return 0; STATIC", true),
        ] {
            for attribute in ["", "used,"] {
                let declaration = format!(
                    "static int b[4] __attribute__(({attribute}section(\".bss.t\")))={{1}};"
                );
                let body = body.replace("STATIC", &declaration);
                let source = format!("int f(void){{{body}}} int main(void){{return 0;}}");
                std::fs::write(&source_file, &source).unwrap();
                let reference = Command::new(clang())
                    .args(["--target=x86_64-unknown-linux-gnu", "-O0", "-c"])
                    .arg(&source_file)
                    .arg("-o")
                    .arg(&object)
                    .output()
                    .unwrap();
                assert_eq!(
                    reference.status.success(),
                    !emitted,
                    "{source}\n{}",
                    text(&reference.stderr)
                );
                if object.exists() {
                    std::fs::remove_file(&object).unwrap();
                }
                for flags in [
                    &["--emit=llvm"][..],
                    &["--backend=native", "--emit=asm"][..],
                    &[
                        "--backend=native",
                        "--native-object",
                        "-c",
                        "-o",
                        object.to_str().unwrap(),
                    ][..],
                    &["--interpret"][..],
                ] {
                    let output = bcc(
                        &source,
                        &[&["--target=x86_64-unknown-linux-gnu", "-O0"][..], flags].concat(),
                    );
                    assert_eq!(
                        output.status.success(),
                        !emitted,
                        "{flags:?}: {source}\n{}",
                        text(&output.stderr)
                    );
                    if object.exists() {
                        std::fs::remove_file(&object).unwrap();
                    }
                }
            }
        }
        std::fs::remove_file(source_file).unwrap();
    }

    #[test]
    fn local_statics_in_skipped_statements_match_clang() {
        for (body, emitted) in [
            ("return; if(g()){STATIC}", false),
            ("return; if(0){}else{STATIC}", false),
            ("return; while(g()){STATIC}", false),
            ("return; for(;;){STATIC}", false),
            ("return; do{STATIC}while(0);", false),
            ("return; switch(x){case 0:{STATIC}}", false),
            ("return; ({STATIC 0;});", false),
            ("for(;;){break; if(g()){STATIC}}", false),
            ("while(g()){continue; while(g()){STATIC}}", false),
            ("goto L; if(g()){STATIC} L:;", false),
            ("for(;;){} if(g()){STATIC}", false),
            ("die(); if(g()){STATIC}", false),
            ("switch(x){if(g()){STATIC} case 0:;}", false),
            ("die(); STATIC", true),
            ("return; {STATIC}", true),
            ("return; if(g()){L:;STATIC}", true),
        ] {
            let source = format!(
                "int g(void); __attribute__((noreturn)) void die(void); void f(int x){{{body}}} \
                 int main(void){{return 0;}}"
            );
            assert_o0_static_emission(&source, emitted);
        }
    }

    #[test]
    fn local_statics_in_ungenerated_operands_match_clang() {
        for (body, emitted) in [
            ("return sizeof(({STATIC a;}));", false),
            ("__typeof__(({STATIC a;})) x=0; return x;", false),
            ("return __alignof__(({STATIC a;}));", false),
            ("return _Generic(({STATIC a;}),int:0);", false),
            ("return _Generic(0,int:0,default:({STATIC a;}));", false),
            ("return _Generic(0,int:({STATIC a;}),default:0);", true),
            ("return __builtin_choose_expr(1,0,({STATIC a;}));", false),
            ("return __builtin_choose_expr(0,({STATIC a;}),0);", false),
            ("return __builtin_choose_expr(1,({STATIC a;}),0);", true),
            ("return 0 ? ({STATIC a;}) : 0;", false),
            ("return 1 ? 0 : ({STATIC a;});", false),
            ("return 0 && ({STATIC a;});", false),
            ("return 1 || ({STATIC a;});", false),
            ("return 1 && ({STATIC a;});", true),
            ("return sizeof(int[({STATIC n;})]);", true),
        ] {
            assert_o0_static_emission(
                &format!("int f(int n){{{body}}} int main(void){{return 0;}}"),
                emitted,
            );
        }
        for body in [
            "if(0)h();",
            "return;h();",
            "(void)(0&&(h(),1));",
            "(void)(1?0:(h(),1));",
        ] {
            assert_o0_static_emission(
                &format!(
                    "static void h(void){{STATIC}} void f(void){{{body}}} int main(void){{return \
                     0;}}"
                ),
                false,
            );
        }
        // An emitted static initializer after a jump still retains its
        // referent.
        assert_o0_static_emission(
            "static void h(void){STATIC} void f(void){return;static void(*p)(void)=h;} int \
             main(void){return 0;}",
            true,
        );
    }

    #[test]
    fn local_static_expression_labels_match_clang() {
        for (body, emitted) in [
            ("if(0){({L:;});STATIC}", true),
            ("if(0){int x=({L:0;});(void)x;STATIC}", true),
            ("return; (void)({L:;STATIC 0;});", true),
        ] {
            assert_o0_static_emission(
                &format!("void f(int x){{{body}}} int main(void){{return 0;}}"),
                emitted,
            );
        }
    }

    #[test]
    fn used_local_in_called_inline_body_is_checked_when_optimizing() {
        let object = temporary("inline.o");
        for header in ["inline", "extern inline __attribute__((gnu_inline))"] {
            let source = format!(
                "{header} int h(void){{static const int a \
                 __attribute__((used,section(\".bss.t\")))=1;return 0;}} int f(void){{return \
                 h();}} int main(void){{return 0;}}"
            );
            for options in [
                &["-O1", "--emit=llvm"][..],
                &["--backend=native", "--opt=bcc", "--emit=asm"][..],
                &[
                    "--backend=native",
                    "--native-object",
                    "--opt=bcc",
                    "-c",
                    "-o",
                    object.to_str().unwrap(),
                ][..],
                &["--interpret", "--opt=bcc"][..],
            ] {
                let output = bcc(
                    &source,
                    &[&["--target=x86_64-unknown-linux-gnu"][..], options].concat(),
                );
                assert!(!output.status.success(), "{options:?}: {source}");
                assert!(
                    text(&output.stderr)
                        .contains("BSS section '.bss.t' cannot have non-zero bytes"),
                    "{}",
                    text(&output.stderr)
                );
            }
        }
    }

    fn assert_o0_static_emission(template: &str, emitted: bool) {
        let source = template.replace(
            "STATIC",
            "static int a __attribute__((section(\".bss.t\")))=1;",
        );
        let source_file = temporary("emission.c");
        let object = temporary("emission.o");
        std::fs::write(&source_file, &source).unwrap();
        let reference = Command::new(clang())
            .args([
                "-std=gnu99",
                "--target=x86_64-unknown-linux-gnu",
                "-O0",
                "-c",
            ])
            .arg(&source_file)
            .arg("-o")
            .arg(&object)
            .output()
            .unwrap();
        assert_eq!(
            reference.status.success(),
            !emitted,
            "{source}\n{}",
            text(&reference.stderr)
        );
        if object.exists() {
            std::fs::remove_file(&object).unwrap();
        }
        for flags in [
            &["--emit=llvm"][..],
            &["--backend=native", "--emit=asm"][..],
            &[
                "--backend=native",
                "--native-object",
                "-c",
                "-o",
                object.to_str().unwrap(),
            ][..],
            &["--interpret"][..],
        ] {
            let output = bcc(
                &source,
                &[
                    &["-std=gnu99", "--target=x86_64-unknown-linux-gnu", "-O0"][..],
                    flags,
                ]
                .concat(),
            );
            assert_eq!(
                output.status.success(),
                !emitted,
                "{flags:?}: {source}\n{}",
                text(&output.stderr)
            );
            if emitted {
                assert!(
                    text(&output.stderr)
                        .contains("BSS section '.bss.t' cannot have non-zero bytes")
                );
            }
            if object.exists() {
                std::fs::remove_file(&object).unwrap();
            }
        }
        // Test retention independently from BSS diagnostics: all object paths
        // must agree with Clang's exact zero-initialized symbol set.
        let source = template.replace("STATIC", "static int a;");
        std::fs::write(&source_file, &source).unwrap();
        let reference = Command::new(clang())
            .args([
                "-std=gnu99",
                "--target=x86_64-unknown-linux-gnu",
                "-O0",
                "-c",
            ])
            .arg(&source_file)
            .arg("-o")
            .arg(&object)
            .output()
            .unwrap();
        assert!(reference.status.success(), "{}", text(&reference.stderr));
        let expected = defined_symbols(&object);
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            let output = bcc(
                &source,
                &[
                    &[
                        "-std=gnu99",
                        "--target=x86_64-unknown-linux-gnu",
                        "-O0",
                        "-c",
                        "-o",
                        object.to_str().unwrap(),
                    ][..],
                    backend,
                ]
                .concat(),
            );
            assert!(
                output.status.success(),
                "{backend:?}: {source}\n{}",
                text(&output.stderr)
            );
            if let (Some(expected), Some(actual)) = (&expected, defined_symbols(&object)) {
                assert_eq!(&actual, expected, "{backend:?}: {source}");
            }
        }
        std::fs::remove_file(source_file).unwrap();
        std::fs::remove_file(object).unwrap();
    }

    #[test]
    fn local_static_retention_uses_the_effective_optimizer_selection() {
        let source =
            "int main(void){static int b[4] __attribute__((section(\".bss.t\")))={1}; return 0;}";
        for (options, optimizing) in [
            (vec![], false),
            (vec!["-O0"], false),
            (vec!["--opt=none"], false),
            (vec!["--llvm-opt=none"], false),
            (vec!["-O0", "-D__OPTIMIZE__=1"], false),
            (vec!["-O"], true),
            (vec!["-O1"], true),
            (vec!["-O2"], true),
            (vec!["-Os"], true),
            (vec!["-Oz"], true),
            (vec!["-Og"], true),
            (vec!["--opt=bcc"], true),
            (vec!["--llvm-opt=O1"], true),
            (vec!["-O0", "--opt=bcc"], true),
            (vec!["-O2", "--opt=none"], true),
            (vec!["-O2", "--llvm-opt=none", "--opt=none"], false),
            (vec!["-O2", "-O0"], false),
            (vec!["-O2", "-U__OPTIMIZE__"], true),
        ] {
            for consumer in ["--emit=ir", "--emit=llvm", "--interpret"] {
                let flags = [
                    &options[..],
                    &[consumer, "--target=x86_64-unknown-linux-gnu"],
                ]
                .concat();
                let output = bcc(source, &flags);
                assert_eq!(
                    output.status.success(),
                    optimizing,
                    "{flags:?}\n{}",
                    text(&output.stderr)
                );
            }
        }
    }

    #[test]
    fn local_static_initializers_retain_referents_and_match_clang_symbols() {
        // The chain reaches another emitted function, whose own unused local
        // reaches an object. Shadowed names and declarations after jumps
        // exercise scope ownership independently of executable
        // references.
        let source = "static int file_static; static int g(void){static int \
                      *q=&file_static;return 0;} static int omitted(void){static int gone \
                      __attribute__((used));return 0;} inline int inline_only(void){static const \
                      int gone __attribute__((used));return 0;} struct P {int a,b;}; int \
                      main(void){static int b[4]={0};static struct P s={0};static const int c=0; \
                      static _Thread_local int tls; static __thread int gnu_tls; static int \
                      (*p)(void)=g; {static int b;} if(0){static int dead;} return 0; static int \
                      after_return;}";
        let source_file = temporary("symbols.c");
        let reference_object = temporary("clang.o");
        let object = temporary("bcc.o");
        std::fs::write(&source_file, source).unwrap();
        // GNU nm reads ELF and COFF. Other tests check emission and
        // diagnostics on hosts without this optional utility.
        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            let target_flag = format!("--target={target}");
            let reference = Command::new(clang())
                .args([&target_flag, "-std=c99", "-O0", "-c"])
                .arg(&source_file)
                .arg("-o")
                .arg(&reference_object)
                .output()
                .unwrap();
            assert!(reference.status.success(), "{}", text(&reference.stderr));
            for backend in [
                &[][..],
                &["--backend=native"][..],
                &["--backend=native", "--native-object"][..],
            ] {
                let flags = [
                    &[
                        &target_flag[..],
                        "-std=c99",
                        "-O0",
                        "-c",
                        "-o",
                        object.to_str().unwrap(),
                    ][..],
                    backend,
                ]
                .concat();
                let output =
                    if target == "x86_64-w64-windows-gnu" && backend == ["--backend=native"] {
                        // The native driver requires a MinGW sysroot even for
                        // header-free assembly. Assemble its text directly so
                        // this comparison also runs without one.
                        let assembly = temporary("native.s");
                        let output = bcc(
                            source,
                            &[
                                &target_flag,
                                "-std=c99",
                                "-O0",
                                "--backend=native",
                                "--emit=asm",
                            ],
                        );
                        assert!(output.status.success(), "{}", text(&output.stderr));
                        std::fs::write(&assembly, &output.stdout).unwrap();
                        let assembled = Command::new(clang())
                            .arg(&target_flag)
                            .arg("-c")
                            .arg(&assembly)
                            .arg("-o")
                            .arg(&object)
                            .output()
                            .unwrap();
                        std::fs::remove_file(assembly).unwrap();
                        assembled
                    } else {
                        bcc(source, &flags)
                    };
                assert!(
                    output.status.success(),
                    "{target} {backend:?}\n{}",
                    text(&output.stderr)
                );
                if let (Some(expected), Some(actual)) =
                    (defined_symbols(&reference_object), defined_symbols(&object))
                {
                    assert_eq!(actual, expected, "{target} {backend:?}");
                    assert!(
                        actual.contains("file_static")
                            && actual.contains("g.q")
                            && actual.contains("main.p")
                    );
                    assert!(!actual.iter().any(|name| name.contains("gone")));
                }
                std::fs::remove_file(&object).unwrap();
            }
            std::fs::remove_file(&reference_object).unwrap();
        }
        for flags in [
            &["--interpret", "-O0"][..],
            &["--interpret", "--opt=bcc"][..],
        ] {
            let output = bcc(source, flags);
            assert!(output.status.success(), "{}", text(&output.stderr));
        }
        std::fs::remove_file(source_file).unwrap();
    }

    fn defined_symbols(object: &Path) -> Option<BTreeSet<String>> {
        let output = Command::new("nm")
            .arg("--defined-only")
            .arg(object)
            .output()
            .ok()?;
        assert!(output.status.success(), "{}", text(&output.stderr));
        Some(
            text(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split_whitespace();
                    let _address = fields.next()?;
                    let kind = fields.next()?;
                    let name = fields.next()?;
                    // Ignore assembler-generated section and absolute metadata.
                    (!matches!(kind, "a" | "A") && !name.starts_with('.')).then(|| name.to_owned())
                })
                .collect(),
        )
    }

    fn bcc(source: &str, flags: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(flags)
                .args(["--input", source]),
        )
        .unwrap()
    }

    fn clang() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        })
    }

    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target");
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(format!(
            "local-statics-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }
}
