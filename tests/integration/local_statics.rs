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
            ("switch(1){case 0:{STATIC}break;case 1:break;}", false),
            ("switch(5){case 0:{STATIC}break;}", false),
            ("switch(1){case 1:break;case 2:{STATIC}}", false),
            ("switch(1){default:{STATIC}break;case 1:break;}", false),
            ("switch(1){default:break;case 1:{STATIC}break;}", true),
            ("switch(1){case 0:case 1:{STATIC}break;}", true),
            ("switch(1){{case 1:{STATIC}break;}}", true),
            ("switch(1){case 1:return;STATIC}", true),
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

    #[test]
    fn case_labels_prevent_ancestor_elision() {
        assert_runs_like_clang(
            "int printf(const char*,...); int f(int y){int x=0; switch(2){default:x=9;break; case \
             1:if(y){case 2:x=2;} x+=1;} return x;} int g(int \
             y){switch(2){default:break;while(y){case 2:return 1;}}return 0;} int \
             main(void){printf(\"%d %d %d\\n\",f(0),f(1),g(0));return 0;}",
        );
    }

    #[test]
    fn folded_switch_regions_lower_only_the_selection_path() {
        // Clang's CollectStatementsForCase skips every statement before the
        // selected label, case-label ancestors included, and lowering jumps
        // straight to the selection, so none of their conditions or
        // declarations may be lowered.
        assert_runs_like_clang(
            "int printf(const char*,...); static int g=1; static int more(void){return 0;} static \
             int h(void){return 4;} int a(void){switch(1){case 0:if(g){case 2:return 2;}return \
             0;case 1:return 1;}return 3;} int b(void){int k=0;switch(1){case \
             0:while(more()){case 2:k++;}case 1:k+=10;}return k;} int c(void){switch(1){case \
             0:{static int s=1;if(s){case 2:return 2;}return 0;}case 1:return 1;}return 3;} int \
             d(void){switch(1){case 0:{int y=h();case 2:return 2;}case 1:return 1;}return 3;} int \
             e(void){switch(5){case 0:if(g){case 2:return 2;}}return 3;} int f(void){int \
             r=0;switch(1){case 0:do{case 2:r=2;}while(more());case 1:r+=1;}return r;} int t(int \
             r){switch(2){default:r=1;break;case 0:if(h()){case 7:r=7;}case 2:r+=2;}return r;} \
             int u(int r){switch(1){case 1:r=1;break;case 0:if(h()){case 7:r=7;}}return r;} int \
             main(void){printf(\"%d %d %d %d %d %d %d \
             %d\\n\",a(),b(),c(),d(),e(),f(),t(0),u(0));return 0;}",
        );
        for body in [
            "switch(1){case 0:if(h()){case 2:return 2;}case 1:return 0;}return 3;",
            "switch(1){case 0:while(h()){case 2:;}case 1:return 0;}return 3;",
            "switch(5){case 0:if(h()){case 2:return 2;}}return 0;",
            "switch(1){case 0:{int y=h();case 2:return y;}case 1:return 0;}return 3;",
            "int r=0;switch(2){default:r=1;break;case 0:if(h()){case 7:r=7;}case 2:r+=2;}return \
             r-2;",
            "int r=0;switch(1){case 1:r=1;break;case 0:if(h()){case 7:r=7;}}return r-1;",
        ] {
            assert_o0_static_emission(
                &format!("static int h(void){{STATIC return a;}} int main(void){{{body}}}"),
                false,
            );
        }
    }

    #[test]
    fn elided_logical_operands_stay_unlowered_in_branches() {
        // Branch-form conditions and loop conditions follow the same operand
        // elision as value contexts.
        assert_runs_like_clang(
            "int printf(const char*,...); static int g=1; static int h(void){return 1;} volatile \
             int v=1; int main(void){int n=0;const int k=0;while((&g==0)&&h())n++;do{n+=2;}while( \
             (&g==0)&&h());for(;v>5||(k&&h());n++);if(v&&((&g==0)&&h()))n+=100;if(!(k&&h()))n+=4; \
             while(v-->5||(k&&h()))n+=1000;n+=(v&&(k&&h()))?16:32;if(v<0||(1||h()))n+=64; \
             printf(\"%d\\n\",n);return 0;}",
        );
    }

    #[test]
    fn omitted_conditional_common_operands_are_emitted() {
        // GNU `x ?: y` emits its common operand whichever way it folds.
        assert_runs_like_clang(
            "int printf(const char*,...); static int g=3; static int helper(void){return 1;} \
             static int s; static const int zero=0; static const int two=2; static const char \
             *const name=\"bcc\"; int f(void){static const int z=0;return z?:3;} int \
             main(void){printf(\"%d %d %d %d %d %d %s\\n\",(&g==0)?:7,(helper==0)?:7,(&s!=0)?:5, \
             zero?:3,two?:3,f(),name?:\"none\");return 0;}",
        );
        assert_o0_static_emission(
            "static int h(void){STATIC return a;} int main(void){return (h==0)?:0;}",
            true,
        );
    }

    #[test]
    fn lowered_control_uses_the_same_fold_as_elision() {
        assert_runs_like_clang(
            "int printf(const char*,...); int f(void){if(__builtin_constant_p(\"abc\"))return \
             1;return 2;} int g(void){int \
             x=0;if(__builtin_constant_p(\"abc\")){x=1;}else{x=2;}return x;} int h(void){int \
             x=5;if(!__builtin_constant_p(\"abc\")){x=1;}else{x=2;}return x;} int \
             s(void){switch(__builtin_constant_p(\"abc\")){case 1:return 11;case 0:return \
             10;}return -1;} int c(void){return __builtin_constant_p(\"abc\")?1:2;} int \
             l(void){return __builtin_constant_p(\"abc\")||(printf(\"rhs\\n\"),0);} int \
             main(void){printf(\"%d %d %d %d %d %d\\n\",f(),g(),h(),s(),c(),l());return 0;}",
        );
    }

    #[test]
    fn dead_initializers_and_cleanup_references_agree_with_lowering() {
        assert_runs_like_clang(
            "int printf(const char*,...); void exit(int); static int h(void){return 31;} static \
             void clean(int *p){*p=0;} int f(int k){if(k){exit(0); int x=h(); return x;}return \
             1;} int g(int k){if(k){exit(0); int x __attribute__((cleanup(clean)))=h();}return \
             2;} int v(int k){if(k){exit(0); int a[h()]; a[0]=1;return a[0];}return 3;} int \
             r(void){static int s=5;return 4;int x=s;return x;} int q(void){goto L;int x=({static \
             int s=7;h()+s;}); L:return 5;} int main(void){printf(\"%d %d %d %d \
             %d\\n\",f(0),g(0),v(0),r(),q());return 0;}",
        );
    }

    #[test]
    fn retained_statics_in_skipped_operands_have_storage() {
        for source in [
            "int f(void){return 0 && ({L:;STATIC 1;});}",
            "int f(void){return __builtin_constant_p(({STATIC 0;}));}",
        ] {
            assert_o0_static_emission(source, true);
        }
    }

    #[test]
    fn emitted_constant_queries_match_the_elision_evaluator() {
        assert_o0_static_emission(
            "void f(void){while(__builtin_constant_p(\"abc\")){return;}if(1){STATIC}}int \
             main(void){f();return 0;}",
            false,
        );
        assert_runs_like_clang(
            r#"
int printf(const char*,...);
static int q=__builtin_constant_p("abc");
_Static_assert(__builtin_constant_p(1.0),"floating constant");
int f(int x){
    const int c=3;
    printf("%d %d %d %d %d %d %d\n",
        __builtin_constant_p("abc"),__builtin_constant_p(c),
        __builtin_constant_p((0,1)),__builtin_constant_p((void*)0),
        __builtin_constant_p(3.5),__builtin_constant_p(x),
        __builtin_constant_p(__builtin_constant_p(x)));
    printf("%d %d %d %d %d\n",
        __builtin_constant_p((x,1)),__builtin_constant_p((long)"abc"),
        __builtin_constant_p(&*"abc"),__builtin_constant_p("abc"+0),
        __builtin_constant_p((char*)"abc"+1));
    return __builtin_constant_p(__builtin_constant_p(x))?1:2;
}
int main(void){printf("%d %d\n",f(2),q);return 0;}
"#,
        );
    }

    #[test]
    fn constant_query_pointer_casts_preserve_target_dll_references() {
        let source_file = temporary("query-width.c");
        let reference_object = temporary("query-width-clang.o");
        let object = temporary("query-width-bcc.o");
        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            let target_flag = format!("--target={target}");
            for operand in [
                "(char)\"a\"",
                "(int)\"a\"",
                "(long)\"a\"",
                "(long long)\"a\"",
                "(long long)(int)\"a\"",
                "(char*)(int)\"a\"",
                "(double)(long)\"a\"",
                "(_Bool)\"a\"",
                "(__int128)\"a\"",
                "(unsigned __int128)(long)\"a\"",
                "(long)(__int128)\"a\"",
                "(char *)(__int128)\"a\"",
                "(unsigned long long)(__int128)\"a\"",
            ] {
                let source = format!(
                    "__declspec(dllimport) extern int x; int live(void){{return x;}} extern int \
                     x; int probe(void){{if(!__builtin_constant_p({operand})) return x; return \
                     0;}}"
                );
                std::fs::write(&source_file, &source).unwrap();
                let reference = Command::new(clang())
                    .args([&target_flag, "-std=gnu99", "-fms-extensions", "-O0", "-c"])
                    .arg(&source_file)
                    .arg("-o")
                    .arg(&reference_object)
                    .output()
                    .unwrap();
                assert!(reference.status.success(), "{}", text(&reference.stderr));
                let actual = bcc(
                    &source,
                    &[
                        &target_flag,
                        "-std=gnu99",
                        "-fms-extensions",
                        "-O0",
                        "-c",
                        "-o",
                        object.to_str().unwrap(),
                    ],
                );
                assert!(actual.status.success(), "{}", text(&actual.stderr));
                if let Some(expected) = undefined_symbols(&reference_object) {
                    let actual = undefined_symbols(&object).unwrap();
                    // Clang also records MSVC's _fltused marker for the
                    // unevaluated floating cast. This probe compares the
                    // reference that controls DLL import inheritance.
                    for symbol in ["x", "__imp_x"] {
                        assert_eq!(
                            actual.contains(symbol),
                            expected.contains(symbol),
                            "{target}: {operand}: {symbol}"
                        );
                    }
                }
                std::fs::remove_file(&reference_object).unwrap();
                std::fs::remove_file(&object).unwrap();
            }
        }
        std::fs::remove_file(source_file).unwrap();
    }

    #[test]
    fn constant_queries_drop_calls_and_keep_nested_static_data() {
        assert_o0_static_emission(
            "static int h(void){STATIC return a;} int f(void){return __builtin_constant_p(h());} \
             int main(void){return f();}",
            false,
        );
        assert_o0_static_emission(
            "static int h(void){STATIC return a;} static int q=__builtin_constant_p(h()); int \
             main(void){return q;}",
            false,
        );
        assert_o0_static_emission(
            "static int h(void){STATIC return a;} int f(void){return __builtin_constant_p( \
             ({static int (*p)(void)=h;0;}));} int main(void){return f();}",
            true,
        );
        assert_o0_static_emission(
            "static int h(void){STATIC return a;} int f(void){return __builtin_constant_p( \
             ({static int q=__builtin_constant_p(h());0;}));} int main(void){return f();}",
            false,
        );
        assert_runs_like_clang(
            "static int h(void){return 3;} static int x; int f(void){return \
             __builtin_constant_p(({static int (*p)(void)=h; static int *q=&x; 0;}));} int \
             main(void){return f()+__builtin_constant_p(h());}",
        );
    }

    #[test]
    fn constant_queries_in_static_data_omit_operand_storage() {
        for body in [
            "static int v=__builtin_constant_p(({STATIC 0;}));return v;",
            "static int v[]={__builtin_constant_p(({STATIC 0;}))};return v[0];",
            "static _Thread_local int v=__builtin_constant_p(({STATIC 0;}));return v;",
            "static int v=1?__builtin_constant_p(({STATIC 0;})):2;return v;",
            "static int v=__builtin_constant_p(({L:;STATIC 0;}));return v;",
        ] {
            assert_o0_static_emission(
                &format!("int f(void){{{body}}}int main(void){{return f();}}"),
                false,
            );
            assert_runs_like_clang(&format!(
                "int f(void){{{}}}int main(void){{return f();}}",
                body.replace("STATIC", "static int a;")
            ));
        }
        assert_o0_static_emission(
            "static int h(void){STATIC return a;} int f(void){static int v= \
             __builtin_constant_p(({static int (*p)(void)=h;0;}));return v;} int \
             main(void){return f();}",
            false,
        );
        assert_o0_static_emission(
            "int f(void){int v=__builtin_constant_p(({STATIC 0;}));return v;} int \
             main(void){return f();}",
            true,
        );
    }

    #[test]
    fn constant_queries_read_fresh_compound_literal_values() {
        assert_runs_like_clang(
            "int printf(const char*,...); struct S{int a;}; int main(void){ printf(\"%d %d %d \
             %d\\n\",__builtin_constant_p((struct S){1}.a), \
             __builtin_constant_p((int){1}),__builtin_constant_p((struct S){1}), \
             __builtin_constant_p((int[]){1,2}[1]));return 0;}",
        );
        assert_o0_static_emission(
            "int f(void){if(__builtin_constant_p((struct S{int a;}){1}.a)){}else{STATIC}return \
             0;} int main(void){return f();}",
            false,
        );
    }

    #[test]
    fn folded_controls_drop_only_their_executable_condition_references() {
        for body in [
            "if(&h!=0)return 0;return 1;",
            "switch(&h!=0){case 1:return 0;default:return 1;}",
            "return (&h!=0)?0:1;",
            "return (&h==0)&&1;",
            "return ((&h!=0)||0)!=1;",
        ] {
            let template =
                format!("static int h(void){{STATIC return a;}} int main(void){{{body}}}");
            assert_o0_static_emission(&template, false);
            assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
        }
        for body in [
            "if(&h!=0){L:;}else{M:;}return 0;",
            "if(__builtin_constant_p(({static int (*p)(void)=h;0;}))){}return 0;",
        ] {
            assert_o0_static_emission(
                &format!("static int h(void){{STATIC return a;}}int main(void){{{body}}}"),
                true,
            );
        }
    }

    fn assert_runs_like_clang(source: &str) {
        // The sources target the default Linux C library, which only a Linux
        // host can link and run; the static-emission checks still run.
        if !cfg!(target_os = "linux") {
            eprintln!("running these programs requires a Linux host");
            return;
        }
        let source_file = temporary("run.c");
        let executable = temporary("run.exe");
        std::fs::write(&source_file, source).unwrap();
        let reference = Command::new(clang())
            .args(["-std=gnu99", "-O0"])
            .arg(&source_file)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(reference.status.success(), "{}", text(&reference.stderr));
        let expected = run_native(&mut Command::new(&executable)).unwrap();
        std::fs::remove_file(&executable).unwrap();
        for flags in [
            &["-O0"][..],
            &["-O2"][..],
            &["--backend=native", "--opt=none"][..],
            &["--backend=native", "--opt=bcc"][..],
            &["--backend=native", "--native-object", "--opt=none"][..],
            &["--backend=native", "--native-object", "--opt=bcc"][..],
            &["--interpret", "--opt=none"][..],
            &["--interpret", "--opt=bcc"][..],
        ] {
            let interpreted = flags.contains(&"--interpret");
            let mut args = vec!["-std=gnu99"];
            args.extend_from_slice(flags);
            if !interpreted {
                args.extend_from_slice(&["-o", executable.to_str().unwrap()]);
            }
            let compiled = bcc(source, &args);
            assert!(
                compiled.status.success(),
                "{flags:?}: {}",
                text(&compiled.stderr)
            );
            let actual = if interpreted {
                compiled
            } else {
                let output = run_native(&mut Command::new(&executable)).unwrap();
                std::fs::remove_file(&executable).unwrap();
                output
            };
            assert_eq!(
                actual.status.code(),
                expected.status.code(),
                "{flags:?}: {}",
                text(&actual.stderr)
            );
            assert_eq!(actual.stdout, expected.stdout, "{flags:?}");
        }
        std::fs::remove_file(source_file).unwrap();
    }

    fn undefined_symbols(object: &Path) -> Option<BTreeSet<String>> {
        let output = Command::new("nm").arg("-u").arg(object).output().ok()?;
        assert!(output.status.success(), "{}", text(&output.stderr));
        Some(
            text(&output.stdout)
                .lines()
                .filter_map(|line| line.split_whitespace().last().map(str::to_owned))
                .collect(),
        )
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
