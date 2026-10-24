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
    fn retained_static_relocations_follow_final_element_order() {
        for initializer in ["[0 ... 2].p=g,[0].p=h", "[2].p=g,[1].p=g,[0].p=h"] {
            for result in ["0", "s[0].p()!=2 || s[1].p()!=1 || s[2].p()!=1"] {
                assert_runs_like_clang(&format!(
                    "int g(void){{return 1;}} int h(void){{return 2;}} int main(void){{static \
                     struct {{int (*p)(void);int x;}} s[3]={{{initializer}}};return {result};}}"
                ));
            }
        }
    }

    #[test]
    fn optional_retention_covers_all_preparation_refusals() {
        for source in [
            "struct S{unsigned:32;unsigned:32;int x;};static int h(struct S s){return s.x;} int \
             main(void){static int(*p)(struct S)=h;return 0;}",
            "struct S{char a[4294967296ULL];};static int h(struct S s){return 0;} int \
             main(void){static int(*p)(struct S)=h;return 0;}",
            "static int h(void){char a[4294967296ULL];return 0;} int main(void){static \
             int(*p)(void)=h;return 0;}",
            "static int h(void){char a[1073741825ULL];return 0;} int main(void){static \
             int(*p)(void)=h;return 0;}",
            "int main(void){static int x __attribute__((section(\"a b\"),aligned(32)))=0;return \
             0;}",
            "int main(void){static char huge[536870912];return 0;}",
        ] {
            assert_runs_like_clang(source);
        }
        for attribute in ["__attribute__((used))", ""] {
            let source = format!(
                "struct S{{unsigned:32;unsigned:32;int x;}};static int h(struct S s){{return \
                 s.x;}} int main(void){{static int(*p)(struct S) {attribute}=h;return {};}}",
                if attribute.is_empty() {
                    "p((struct S){0})"
                } else {
                    "0"
                },
            );
            for flags in [
                &["--emit=ir-abi"][..],
                &["--backend=native", "--emit=asm"][..],
                &["--interpret"][..],
            ] {
                let output = bcc(&source, flags);
                assert!(!output.status.success(), "{flags:?}: {source}");
                assert!(
                    text(&output.stderr).contains("first eightbyte"),
                    "{}",
                    text(&output.stderr)
                );
            }
        }
        let output = bcc(
            "int main(void){static char huge[536870912] __attribute__((used));return 0;}",
            &["--interpret"],
        );
        assert_eq!(output.status.code(), Some(70));
        assert!(text(&output.stderr).contains("global is larger"));
    }

    #[test]
    fn unmodeled_constant_folds_keep_optional_storage_best_effort() {
        for condition in [
            "&x==&y",
            "!__builtin_constant_p((char*)8)",
            "!__builtin_constant_p((long)\"a\"+1-1)",
        ] {
            assert_runs_like_clang(&format!(
                "int x,y;extern int missing;int main(void){{if({condition}){{static int b \
                 __attribute__((section(\".bss.bad\")))=1;static int*p=&missing;}}return 0;}}"
            ));
            assert_runs_like_clang_with_linker_symbols(&format!(
                "int printf(const char*,...);int x,y;extern int __start_tbl[],__stop_tbl[]; \
                 static int anchor __attribute__((used,section(\"tbl\")))=1; int \
                 main(void){{if({condition}){{static int entry \
                 __attribute__((section(\"tbl\")))=2;}} \
                 printf(\"%ld\\n\",(long)(__stop_tbl-__start_tbl));return 0;}}"
            ));
        }
        for body in [
            "return (&x==&y)?({STATIC 0;}):0;",
            "return (&x==&y)&&({STATIC 0;});",
            "switch(&x==&y){case 1:{STATIC break;}default:break;}return 0;",
        ] {
            assert_runs_like_clang(&format!(
                "int x,y;extern int missing;int main(void){{{}}}",
                body.replace(
                    "STATIC",
                    "static int b __attribute__((section(\".bss.bad\")))=1;static int*p=&missing;"
                )
            ));
        }
        for tag in ["struct", "union"] {
            let template = format!(
                "{tag} S{{int a;}};int x,y;int main(void){{{tag} S s=(&x==&y)?({{STATIC ({tag} \
                 S){{1}};}}):({tag} S){{2}};return s.a-2;}}"
            );
            assert_o0_static_emission(&template, false);
            assert_runs_like_clang(&template.replace(
                "STATIC",
                "static int b __attribute__((section(\".bss.bad\")))=1;",
            ));
            let dynamic = format!(
                "{tag} S{{int a;}};int x,y;int main(void){{struct O{{{tag} S s;int \
                 v;}}o={{(&x==&y)?({{STATIC ({tag} S){{1}};}}):({tag} S){{2}},x}};return \
                 o.s.a-2;}}"
            );
            assert_o0_static_emission(&dynamic, true);
        }
        for body in [
            "if(&x==&y)h();return 0;",
            "struct S s=(&x==&y)?h():(struct S){2};return s.a-2;",
        ] {
            for helpers in [
                "static struct S h(void){STATIC return (struct S){1};}",
                "static int leaf(void){STATIC return 1;}static struct S h(void){return (struct \
                 S){leaf()};}",
            ] {
                let template =
                    format!("struct S{{int a;}};int x,y;{helpers}int main(void){{{body}}}");
                // The evaluator still emits the helper. Protect baseline
                // acceptance without asserting Clang's folded symbol set.
                assert_runs_like_clang(&template.replace(
                    "STATIC",
                    "static int a __attribute__((section(\".bss.bad\")))=1;",
                ));
            }
        }
        let live_helper = "struct S{int a;};int x,y;static struct S h(void){STATIC return (struct \
                           S){1};}int main(void){if(&x==&y)h();return h().a-1;}";
        assert_o0_static_emission(live_helper, true);
        // Required storage retains its diagnostic even under an unmodeled
        // fold; the best-effort policy only removes optional roots.
        let output = bcc(
            "int x,y;int main(void){if(&x==&y){static int b \
             __attribute__((used,section(\".bss.bad\")))=1;}return 0;}",
            &["--emit=llvm"],
        );
        assert!(!output.status.success());
        assert!(text(&output.stderr).contains("BSS section"));
    }

    #[test]
    fn nested_constant_queries_keep_values_through_wrappers() {
        for operand in [
            "+__builtin_constant_p(g)",
            "!__builtin_constant_p(g)",
            "(long)__builtin_constant_p(g)",
            "__builtin_constant_p(g)+1",
            "__builtin_constant_p(g)*2",
            "__builtin_constant_p(g)==0",
            "__builtin_constant_p(g)?2:3",
            "__builtin_choose_expr(1,__builtin_constant_p(g),g)",
        ] {
            assert_runs_like_clang(&format!(
                "int g; enum {{ q=__builtin_constant_p({operand}) }}; \
                 _Static_assert(q==1,\"nested query\"); int main(void){{return \
                 __builtin_constant_p({operand})!=q;}}"
            ));
        }
    }

    #[test]
    fn comma_constant_queries_distinguish_emitted_constants_and_loads() {
        assert_runs_like_clang(
            "int g;int main(void){return __builtin_constant_p((g,__builtin_strlen(\"a\")));}",
        );
        // Clang O0 emits a load; Clang O2 can optimize it to a constant.
        // This compiler conservatively keeps the O0 answer on these arms.
        for value in ["(int){1}", "(int[]){1}[0]", "(int){1}+0"] {
            assert_run_arms(
                &format!("int g;int main(void){{return __builtin_constant_p((g,{value}));}}"),
                false,
                Some(b""),
            );
        }
    }

    #[test]
    fn aggregate_conditional_statement_expression_tails_stay_typed() {
        for selection in [
            "0?({(struct S){1};}):(struct S){2}",
            "1?(struct S){2}:({(struct S){1};})",
        ] {
            assert_runs_like_clang(&format!(
                "struct S{{int a;}};int main(void){{struct S s={selection};return s.a-2;}}"
            ));
        }
    }

    #[test]
    fn aggregate_value_and_ignored_results_use_distinct_conditional_visitors() {
        for tag in ["struct", "union"] {
            let prelude = format!("{tag} S{{int a;}};");
            for selection in [
                format!("0?({{STATIC ({tag} S){{1}};}}):({tag} S){{2}}"),
                format!("1?({tag} S){{2}}:({{STATIC ({tag} S){{1}};}})"),
            ] {
                for value in [selection.clone(), format!("({{{selection};}})")] {
                    let template =
                        format!("{prelude}int main(void){{{tag} S s={value};return s.a-2;}}");
                    assert_o0_static_emission(&template, false);
                    assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
                    assert_runs_like_clang(&template.replace(
                        "STATIC",
                        "static int a __attribute__((used,section(\".bss.t\")))=1;",
                    ));
                }
                let helper = format!(
                    "{prelude}static {tag} S h(void){{STATIC return ({tag} S){{1}};}}int \
                     main(void){{{tag} S s=0?h():({tag} S){{2}};return s.a-2;}}"
                );
                assert_o0_static_emission(&helper, false);
                assert_runs_like_clang(&helper.replace("STATIC", "static int a;"));
                for statement in [
                    format!("(void)({selection});"),
                    format!("typedef void V;(V)({selection});"),
                    format!("({selection}),0;"),
                    format!("for({selection};0;){{}}"),
                    format!("for(;0;{selection}){{}}"),
                    format!("__builtin_choose_expr(1,({selection}),({tag} S){{0}});"),
                    format!("_Generic(0,int:({selection}));"),
                ] {
                    let template = format!("{prelude}int main(void){{{statement}return 0;}}");
                    assert_o0_static_emission(&template, true);
                    assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
                }
            }
        }
    }

    #[test]
    fn aggregate_visitors_fold_only_constant_automatic_initializers() {
        for tag in ["struct", "union"] {
            let prelude = format!(
                "{tag} S{{int a;}};static {tag} S h(void){{STATIC return ({tag} S){{1}};}}static \
                 {tag} S g={{2}};"
            );
            for body in [
                format!("{tag} S s=0?h():g;return s.a-2;"),
                format!("{tag} S a[]={{0?h():({tag} S){{2}},g}};return a[0].a-2;"),
                format!("{tag} S s;s=0?h():({tag} S){{2}};return s.a-2;"),
                format!("return (0?h():({tag} S){{2}}).a-2;"),
                format!("return take(0?h():({tag} S){{2}})-2;"),
                format!(
                    "struct O{{{tag} S s;int x;}}o={{0?h():({tag} S){{2}},take(g)}};return \
                     o.s.a-2;"
                ),
                "return make().a-2;".to_owned(),
            ] {
                let make = if body == "return make().a-2;" {
                    format!("{tag} S make(void){{return 0?h():({tag} S){{2}};}}")
                } else {
                    String::new()
                };
                let template = format!(
                    "{prelude}int take({tag} S s){{return s.a;}}{make}int main(void){{{body}}}"
                );
                assert_o0_static_emission(&template, true);
                assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
            }
            for body in [
                format!("{tag} S a[]={{0?h():({tag} S){{2}}}};return a[0].a-2;"),
                format!("const {tag} S c={{2}};{tag} S s=0?h():c;return s.a-2;"),
            ] {
                let template = format!("{prelude}int main(void){{{body}}}");
                assert_o0_static_emission(&template, false);
                assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
            }
        }
    }

    #[test]
    fn overwritten_initializers_keep_optional_retention_best_effort() {
        for value in ["h()", "0?h():g", "1?h():g", "0?({STATIC (struct S){1};}):g"] {
            let source = format!(
                "struct S{{int a;}};struct O{{struct S s;int b;}};static struct S g={{2}};static \
                 struct S h(void){{STATIC return (struct S){{1}};}}int main(void){{struct O \
                 o={{.s={value},.s={{2}}}};return o.s.a-2;}}"
            );
            assert_runs_like_clang(&source.replace(
                "STATIC",
                "static int a __attribute__((section(\".bss.bad\")))=1;",
            ));
        }
        // A partial overwrite preserves the earlier aggregate value.
        let output = bcc(
            "struct S{int a,b;};static struct S h(void){static int a \
             __attribute__((section(\".bss.bad\")))=1;return (struct S){1,2};}int \
             main(void){struct S s={1,2};struct O{struct S s;}o={.s=h(),.s.a=2};return o.s.b-2;}",
            &["--emit=llvm"],
        );
        assert!(!output.status.success());
        assert!(text(&output.stderr).contains("BSS section"));
    }

    #[test]
    fn omitted_scalar_statement_expression_tails_preserve_following_code() {
        for statement in ["for(;;);", "return 99;", "__builtin_unreachable();"] {
            for expression in [
                format!("0?({{{statement}0;}}):0"),
                format!("1?0:({{{statement}0;}})"),
            ] {
                assert_runs_like_clang(&format!(
                    "int f(void){{(void)({expression});return 17;}}int main(void){{return \
                     f()-17;}}"
                ));
            }
        }
    }

    #[test]
    fn scalar_logical_left_operands_use_branch_emission() {
        for body in [
            "return (v&&(0&&h()))&&v;",
            "do {} while((v&&(0&&h()))&&v);return 0;",
        ] {
            let template = format!(
                "static int h(void){{STATIC return a;}} int f(int v){{{body}}} int \
                 main(void){{return f(1);}}"
            );
            assert_o0_static_emission(&template, true);
            assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
        }
    }

    #[test]
    fn folded_if_conditions_omit_their_local_definitions() {
        for condition in ["0&&({STATIC 1;})", "1||({STATIC 0;})", "1?0:({STATIC 1;})"] {
            let template = format!("int main(void){{if({condition})return 0;return 0;}}");
            assert_o0_static_emission(&template, false);
            assert_runs_like_clang(
                &template.replace("STATIC", "extern int missing;static int *p=&missing;"),
            );
        }
    }

    #[test]
    fn aggregate_and_complex_conditionals_keep_both_operands_emitted() {
        for (definitions, selected, omitted) in [
            (
                "struct S { int a; }; static struct S g={3}; static struct S h(void){STATIC \
                 return g;}",
                "g",
                "h()",
            ),
            (
                "union S { int a; }; static union S g={3}; static union S h(void){STATIC return \
                 g;}",
                "g",
                "h()",
            ),
            (
                "static _Complex double g=3; static _Complex double h(void){STATIC return g;}",
                "g",
                "h()",
            ),
            (
                "static _Complex double h(void){STATIC return 3;}",
                "1.0",
                "h()",
            ),
        ] {
            for expression in [
                format!("0?{omitted}:{selected}"),
                format!("1?{selected}:{omitted}"),
            ] {
                let template =
                    format!("{definitions} int main(void){{(void)({expression});return 0;}}");
                assert_o0_static_emission(&template, true);
                assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
            }
        }
    }

    #[test]
    fn nested_constant_queries_keep_unfolded_control_operands() {
        for query in [
            "__builtin_constant_p(g)",
            "__builtin_constant_p(__builtin_constant_p(g))",
        ] {
            let template = format!(
                "int g; static int h(void){{STATIC return a;}} int \
                 main(void){{if(__builtin_constant_p({query})) return 0; else return h();}}"
            );
            assert_o0_static_emission(&template, true);
            assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
        }
    }

    #[test]
    fn unsupported_unused_local_statics_keep_main_acceptance() {
        assert_runs_like_clang(
            r"
            typedef int vector __attribute__((vector_size(16)));
            typedef int extvector __attribute__((ext_vector_type(4)));
            int f(int a) {
                static __float128 quad = 1.5Q;
                static vector v = {1,2,3,4};
                static extvector e = {1,2,3,4};
                static _BitInt(200) bits = 7;
                return a + 1;
            }
            int main(void) { return f(4) != 5; }
        ",
        );
        assert_runs_like_clang(
            r"
            static void unsupported(void) { __float128 q = 1.5Q; (void)q; }
            int f(void) { static void (*hook)(void) = unsupported; return 42; }
            int main(void) { return f() != 42; }
        ",
        );
        assert_runs_like_clang(
            r#"
            typedef long jb[8]; int _setjmp(jb);
            int f(void) {
                static __thread int t __attribute__((section(".tdata.mine"))) = 1;
                static int (*sj)(jb) = _setjmp;
                return 42;
            }
            int main(void) { return f() != 42; }
        "#,
        );
        assert_runs_like_clang(
            "int g; static void helper(void){g=__builtin_ia32_rdtsc();} void f(void){static void \
             (*p)(void)=helper;}int main(void){f();return 0;}",
        );
        assert_runs_like_clang(
            "int rt(void) __attribute__((returns_twice));int rt(void){return 0;} static int \
             helper(void){return rt();}int f(void){static int(*p)(void)=helper;return 0;} int \
             main(void){return f();}",
        );
        let used_native_helper = bcc(
            "int rt(void) __attribute__((returns_twice));int rt(void){return 0;} static int \
             helper(void){return rt();}int f(void){static int(*p)(void) \
             __attribute__((used))=helper;return 0;}int main(void){return f();}",
            &["--backend=native", "--emit=asm"],
        );
        assert!(!used_native_helper.status.success());
        let used_helper = bcc(
            "int g; static void helper(void){g=__builtin_ia32_rdtsc();} void f(void){static void \
             (*p)(void) __attribute__((used))=helper;} int main(void){f();return 0;}",
            &["--emit=llvm"],
        );
        assert!(!used_helper.status.success());
        let used = bcc(
            "int main(void){static __float128 q __attribute__((used))=1;return 0;}",
            &["--emit=llvm"],
        );
        assert!(!used.status.success());
        assert!(text(&used.stderr).contains("binary128"));
    }

    #[test]
    fn folded_switch_paths_include_stacked_and_range_labels() {
        assert_runs_like_clang(
            r#"
            int printf(const char *, ...);
            static int h(void) { return 4; }
            int a(void) { switch(sizeof(short)) { case 1: case 2: return 2; case 4: return 4; } return 0; }
            int b(void) { switch(3) { default: case 3: return 3; case 4: return 4; } return 0; }
            int c(void) { switch(1) { case 5 ... 6: case 1: return 1; } return 3; }
            int d(void) { int r=0; switch(2) { case 0: if(h()) {case 7:r=7;} case 8: case 2: r+=2; } return r; }
            int main(void) { printf("%d %d %d %d\n",a(),b(),c(),d());return 0; }
        "#,
        );
    }

    #[test]
    fn floating_values_keep_the_emitted_constant_query_value() {
        assert_runs_like_clang(
            r#"
            int printf(const char *, ...); int g=3;
            int main(void) {
                int x=g;
                double d=__builtin_constant_p((x,1))+0.5;
                float f=__builtin_constant_p((x,1))*2.0f;
                _Complex double z=__builtin_constant_p((x,1))+0.5;
                printf("%d %d %d\n",(int)(d*10),(int)(f*10),(int)(__real__ z*10));
                return 0;
            }
        "#,
        );
    }

    #[test]
    fn effects_only_recipes_skip_elided_operands_and_keep_live_effects() {
        // Clang does not support GCC's overflow predicates. This result is
        // independently pinned against main, including the two live calls.
        assert_run_arms(
            r#"
            int printf(const char *, ...); static int g;
            static int h(void) { printf("h\n");return 1; }
            int main(void) {
                int k=0; const int z=0;
                int a=__builtin_add_overflow_p(1,2,(int)((&g==0)?h():k));
                int b=__builtin_add_overflow_p(1,2,(int)((&g==0)&&h()));
                int c=__builtin_add_overflow_p(1,2,(int)(z?h():k));
                (void)__builtin_add_overflow_p(1,2,(int)((&g!=0)?h():k));
                (void)__builtin_add_overflow_p(1,2,(int)((&g!=0)&&h()));
                printf("%d %d %d\n",a,b,c);return 0;
            }
        "#,
            false,
            Some(b"h\nh\n0 0 0\n"),
        );
    }

    #[test]
    fn retroactive_skips_elide_static_declarations_and_initializers() {
        for body in [
            "switch(1){case 1: break; static int *p=&missing;} return 6;",
            "switch(1){case 1: {break;} static int *p=&missing;} return 6;",
            "({goto out; (void)({static int *p=&missing;0;});});out:return 6;",
        ] {
            assert_runs_like_clang(&format!("extern int missing; int main(void){{{body}}}"));
        }
        for body in [
            "switch(1){case 1: break; STATIC} return 6;",
            "switch(1){case 1: {break;} STATIC} return 6;",
            "({goto out; (void)({STATIC 0;});});out:return 6;",
        ] {
            let source = format!("int f(void){{{body}}} int main(void){{return f()!=6;}}");
            assert_o0_static_emission(&source, false);
            assert_runs_like_clang(&source.replace("STATIC", "static int a;"));
        }
    }

    #[test]
    fn branch_conditions_emit_nested_short_circuit_operands() {
        for condition in [
            "v && (0 && h())",
            "v || (1 || h())",
            "v || !(0 && h())",
            "v && !(1 || h())",
            "v ? (0 && h()) : v",
            "v ? v : (1 || h())",
        ] {
            let template = format!(
                "static int h(void){{STATIC return a;}} int f(int v){{if({condition})return \
                 1;return 0;}} int printf(const char*,...); int main(void){{printf(\"%d \
                 %d\\n\",f(0),f(1));return 0;}}"
            );
            assert_o0_static_emission(&template, true);
            assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
        }
        for condition in [
            "v && ((&h!=0) || 0)",
            "v || ((&h==0) && 1)",
            "v && (0 ? h() : 1)",
            "v || (1 ? 0 : h())",
        ] {
            let template = format!(
                "static int h(void){{STATIC return a;}} int f(int v){{if({condition})return \
                 1;return 0;}} int printf(const char*,...); int main(void){{printf(\"%d \
                 %d\\n\",f(0),f(1));return 0;}}"
            );
            assert_o0_static_emission(&template, false);
            assert_runs_like_clang(&template.replace("STATIC", "static int a;"));
        }
    }

    #[test]
    fn discarded_comma_queries_keep_control_flow_and_pointer_answers() {
        assert_runs_like_clang(
            r"
            extern int puts(const char *);
            int x;
            static int slow(void) { static int count; return ++count; }
            int f(void) {
                int r = 0;
                if (__builtin_constant_p((x, 1))) r += 2; else r += slow();
                r += __builtin_constant_p((x, 1)) ? 3 : slow();
                r += __builtin_constant_p((x, 1)) || slow();
                r += __builtin_constant_p((x, 1)) && slow();
                switch (__builtin_constant_p((x, 1))) {
                    case 0: r += slow(); break; case 1: r += 4;
                }
                return r;
            }
            int main(void) {
                if (__builtin_constant_p((x, (char *)0)) ||
                    __builtin_constant_p((x, (void *)0))) return 1;
                return f() != 11;
            }
        ",
        );
        // Neither the evaluator nor the emitted-value shortcut may remove
        // an arm's used section entry before the linker builds the table.
        assert_runs_like_clang_with_linker_symbols(
            r#"
            extern int printf(const char *, ...);
            extern int __start_tbl[], __stop_tbl[];
            int f(int x) {
                if (__builtin_constant_p((x, 1))) {
                    static int a __attribute__((used, section("tbl"))) = 1;
                } else {
                    static int b __attribute__((used, section("tbl"))) = 2;
                }
                return 0;
            }
            int main(void) {
                f(1);
                printf("%ld %d\n", (long)(__stop_tbl - __start_tbl),
                       __start_tbl[0] + __start_tbl[1]);
                return 0;
            }
        "#,
        );
    }

    #[test]
    fn constant_query_operands_share_one_emission_decision() {
        assert_runs_like_clang(
            r"
            int x;
            static int g = 4;
            static int h(void) { return 3; }
            int (*fp)(void);
            int main(void) {
                int a = __builtin_constant_p(g + 1);
                int b = __builtin_constant_p(fp == h);
                int c = __builtin_constant_p(({ static int yes = 2; x + yes; }));
                int d = __builtin_constant_p(({ static int no; &no; }));
                int e = __builtin_constant_p(({ static int no2; ++x + no2; }));
                return a + b + c + d + e;
            }
        ",
        );
    }

    #[test]
    fn constant_query_operand_referents_match_clang() {
        for (operand, emitted) in [
            ("a", true),
            ("a + 1", true),
            ("(long)&h", true),
            ("fp == h", true),
            ("a++", false),
            ("&a", false),
        ] {
            assert_o0_static_emission(
                &format!(
                    "static int a; static int h(void){{STATIC return 0;}} int (*fp)(void); int \
                     main(void){{return __builtin_constant_p({operand});}}"
                ),
                emitted && operand.contains('h'),
            );
        }
        for (operand, emitted) in [("a", true), ("a + 1", true), ("(x, 1)", true)] {
            assert_o0_static_emission(
                &format!(
                    "static int h(void){{STATIC return 0;}} int main(void){{int x=0; static int \
                     a; return __builtin_constant_p({operand}) ? 0 : h();}}"
                ),
                emitted,
            );
        }
        for operand in ["({STATIC &a;})", "({STATIC ++x; 1;})"] {
            assert_o0_static_emission(
                &format!("int main(void){{int x=0;return __builtin_constant_p({operand});}}"),
                false,
            );
        }
    }

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
    fn constant_queries_take_clang_constant_context_values() {
        // Clang's constant evaluator cannot discard a non-constant comma
        // operand or read through an array compound literal, so integer
        // constant expressions and static data see zero there, while
        // function code still emits one.
        assert_runs_like_clang(
            r#"
int printf(const char*,...);
int g=3;
static int h1(void){return 1;}
static int h2(void){return 2;}
enum{E1=__builtin_constant_p((g,1)),E2=__builtin_constant_p(((int[]){1,2})[1]),
     E3=__builtin_constant_p(((const int[]){1,2})[1]),E4=__builtin_constant_p((1,2)),
     E5=__builtin_constant_p(((struct{int a[2];}){{1,2}}).a[1])};
static int s1=__builtin_constant_p((g,1));
_Static_assert(!__builtin_constant_p((g,1)),"comma");
int main(void){
    int x=g;
    switch(x-2){case 1+__builtin_constant_p((x,1)):printf("one\n");break;default:printf("other\n");}
    printf("%d %d %d %d %d %d %d\n",E1,E2,E3,E4,E5,s1,
        (int)sizeof(int[__builtin_constant_p((g,1))?1:2]));
    int r=__builtin_constant_p((x,1)),n=!__builtin_constant_p((x,1));
    while(!__builtin_constant_p(((int[]){1,2})[1]))r+=10;
    printf("%d %d %d %d %d\n",r,n,__builtin_constant_p((x,1))?3:4,
        __builtin_constant_p((x,"abc")),__builtin_constant_p((x,(long)"abc")));
    static int (*sp)(void)=__builtin_constant_p((g,1))?h1:h2;
    while(__builtin_constant_p((x,1))){r+=100;break;}
    switch(__builtin_constant_p(((int[]){1,2})[1])){case 1:r+=1000;break;default:r+=5000;}
    printf("%d %d %d\n",__builtin_choose_expr(__builtin_constant_p((g,1)),10,20),sp(),r);
    return 0;
}
"#,
        );
    }

    #[test]
    fn dead_automatic_initializers_follow_emit_auto_var_init() {
        for (template, emitted) in [
            ("int f(void){return 0;int x=({STATIC 0;});return x;}", false),
            (
                "void exit(int);int f(void){exit(0);int x=({STATIC 0;});return x;}",
                false,
            ),
            (
                "int f(void){return 0;int x[2]={({STATIC 0;}),1};return x[0];}",
                false,
            ),
            (
                "int f(void){return 0;int y=0,x=({STATIC 0;});return x+y;}",
                false,
            ),
            (
                "static void h(void){STATIC} int f(void){return 0;int x=(h(),0);return x;}",
                false,
            ),
            (
                "int f(void){return 0;int x=({L:;STATIC 0;});return x;}",
                true,
            ),
            // A label in the initializer gives code generation an insertion
            // point, so the statements after the declaration are emitted.
            (
                "int f(int k){return 0;int y=({L2:;0;});if(k){STATIC}return y;}",
                true,
            ),
        ] {
            assert_o0_static_emission(&format!("{template} int main(void){{return 0;}}"), emitted);
        }
    }

    #[test]
    fn refused_local_statics_are_reported_once() {
        let output = bcc(
            "int dup=3; int f(void){static int x __asm__(\"dup\")=1;return x;}",
            &["-std=gnu99", "-O0", "--emit=llvm"],
        );
        assert!(!output.status.success());
        let diagnostic = text(&output.stderr);
        assert_eq!(
            diagnostic.matches("assembler labels").count(),
            1,
            "{diagnostic}"
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
        assert_run_arms(source, false, None);
    }

    fn assert_runs_like_clang_with_linker_symbols(source: &str) {
        assert_run_arms(source, true, None);
    }

    fn assert_run_arms(source: &str, linker_symbols: bool, expected_output: Option<&[u8]>) {
        // The sources target the default Linux C library, which only a Linux
        // host can link and run; the static-emission checks still run.
        if !cfg!(target_os = "linux") {
            eprintln!("running these programs requires a Linux host");
            return;
        }
        let source_file = temporary("run.c");
        let executable = temporary("run.exe");
        std::fs::write(&source_file, source).unwrap();
        let (expected_code, expected_stdout) = if let Some(output) = expected_output {
            (Some(0), output.to_vec())
        } else {
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
            (expected.status.code(), expected.stdout)
        };
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
            if linker_symbols && interpreted {
                // The interpreter has no linker-synthesized section symbols.
                continue;
            }
            let mut args = vec!["-std=gnu99"];
            args.extend_from_slice(flags);
            if !interpreted {
                args.extend_from_slice(&["-o", executable.to_str().unwrap()]);
            }
            let compiled = bcc(source, &args);
            assert!(
                interpreted || compiled.status.success(),
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
                expected_code,
                "{flags:?}: {}",
                text(&actual.stderr)
            );
            assert_eq!(actual.stdout, expected_stdout, "{flags:?}");
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
