#![expect(
    missing_docs,
    reason = "This integration test crate has no public API."
)]
#![expect(
    unused_crate_dependencies,
    reason = "CLI tests exercise the compiled binary."
)]

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "CLI test inputs, paths and subprocess output use host standard-library storage."
)]
mod tests {
    use std::process::{
        Command,
        Output,
    };

    fn run(flags: &[&str], source: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--syntax-tree", "--syntax-locations", "--input", source])
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("SOURCE_DATE_EPOCH")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .expect("the CLI must run")
    }

    fn clean_tree(output: &Output) -> String {
        let tree = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{output:?}");
        assert!(
            !tree.contains("error:") && !tree.contains("warning:") && !tree.contains("recovered"),
            "{tree}"
        );
        tree
    }

    #[test]
    fn representative_programs_cross_all_phases_in_every_standard_spelling() {
        for (modes, source, expected) in [
            (
                "c89 c90 iso9899:1990",
                "#define VALUE 7\nextern object; f(void){return VALUE;}\n",
                "constant 7",
            ),
            (
                "iso9899:199409",
                "#define VALUE 7\nextern object; int a<:1:> = <% VALUE %>;\n",
                "constant 7",
            ),
            (
                "c99 c9x iso9899:1999 iso9899:199x",
                "#define VALUES(...) __VA_ARGS__\nint a[]={VALUES(1,2)}; inline int \
                 f(void){for(int i=0;i<2;i++){a[i]+=0x1p0;} return a[0];}\n",
                "constant 1 (double)",
            ),
            (
                "c11 c1x iso9899:2011 c17 c18 iso9899:2017 iso9899:2018",
                "#define SELECT(x) _Generic((x),int:1,default:0)\n_Alignas(16) _Atomic(int) x; \
                 _Static_assert(SELECT(1),\"ok\"); const char *s=u8\"hello\";\n",
                "generic-selection",
            ),
            (
                "c23 c2x iso9899:2024",
                "#define VALUES(...) __VA_OPT__(__VA_ARGS__,) 3\n#if \
                 __has_c_attribute(maybe_unused)\n[[maybe_unused]] constexpr unsigned _BitInt(8) \
                 n=0b1'010uwb;\n#else\n#error missing attribute support\n#endif\nint \
                 a[]={VALUES(1,2)}; static_assert(true); auto c=u8'Q';\n",
                "_BitInt",
            ),
            (
                "c2y",
                "#define CODE 0o7\nchar c='\\x{41}'; int a[]={CODE}; int f(void){if(int \
                 n=_Countof(a); n){return _Generic(int,int:CODE,default:0);} return 0;}\n",
                "countof",
            ),
        ] {
            for mode in modes.split_whitespace() {
                let flag = format!("-std={mode}");
                let tree = clean_tree(&run(&[&flag, "-pedantic-errors"], source));
                assert!(tree.contains(expected), "{mode}: {tree}");
            }
        }
    }

    #[test]
    fn gnu_macros_keywords_and_imaginary_constants_reach_the_parser() {
        let source = "#define TYPE(x) typeof(x)\n#define MAKE(name,args...) __extension__ \
                      TYPE(1.0i) name = (args)\nMAKE($value,1.0i+2j+3.0fi+4.0Li);\ninline int \
                      f(void){asm(\"nop\"); return ({__auto_type n=__COUNTER__; n ?: 1;});}\n";
        for mode in [
            "gnu89", "gnu90", "gnu99", "gnu9x", "gnu11", "gnu1x", "gnu17", "gnu18", "gnu23",
            "gnu2x", "gnu2y",
        ] {
            let flag = format!("-std={mode}");
            let tree = clean_tree(&run(&[&flag], source));
            for expected in [
                "$value",
                "typeof",
                "constant 1i (double _Complex)",
                "constant 2i (int _Complex)",
                "constant 3i (float _Complex)",
                "constant 0x1p+2i (long double _Complex)",
                "asm",
                "statement-expression",
            ] {
                assert!(tree.contains(expected), "{mode}, {expected}: {tree}");
            }
        }
        let output = run(&["-std=c17"], source);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("error:"),
            "{output:?}"
        );
        let reserved = "#define TYPE(x) __typeof__(x)\nTYPE(1.0i) value=1.0i+2j; int \
                        f(void){__asm__(\"nop\"); return __real__ value;}\n";
        let tree = clean_tree(&run(&["-std=c17"], reserved));
        assert!(
            tree.contains("constant 1i (double _Complex)")
                && tree.contains("constant 2i (int _Complex)"),
            "{tree}"
        );
        let output = run(&["-std=c17", "-pedantic-errors"], reserved);
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostics.contains("is a GNU extension"), "{diagnostics}");
    }

    #[test]
    fn msvc_macro_pragmas_and_empty_variadic_calls_reach_the_parser() {
        let source = "#define DECL(name) __pragma(STDC FP_CONTRACT ON) __declspec(dllexport) \
                      __int64 __cdecl name\n#define CALL(x,...) target(x, \
                      __VA_ARGS__)\nDECL(f)(void){__pragma(STDC FENV_ACCESS OFF) __int64 * \
                      __ptr64 p; __try { CALL(1); } __except(1) { __asm { nop } } return 0;}\n";
        for mode in [
            "c89",
            "iso9899:199409",
            "c99",
            "c11",
            "c17",
            "c23",
            "c2y",
            "gnu17",
        ] {
            let flag = format!("-std={mode}");
            let tree = clean_tree(&run(&[&flag, "-fms-extensions"], source));
            for expected in [
                "__declspec",
                "__int64",
                "__cdecl",
                "__ptr64",
                "seh",
                "__asm",
            ] {
                assert!(tree.contains(expected), "{mode}, {expected}: {tree}");
            }
            assert!(!tree.contains("__pragma"), "{tree}");
        }
        for disable in ["-fno-ms-pragma", "-fno-ms-va-args"] {
            let output = run(&["-std=c17", "-fms-extensions", disable], source);
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("error:"),
                "{disable}: {output:?}"
            );
        }
        let tree = clean_tree(&run(
            &[
                "-std=c17",
                "-fms-extensions",
                "-fno-ms-pragma",
                "-fms-pragma",
            ],
            source,
        ));
        assert!(tree.contains("block-item: seh"), "{tree}");
    }

    #[test]
    fn embed_queries_and_parameters_produce_initializer_elements() {
        let resource = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/language/embed.bin"
        )
        .replace('\\', "/");
        let source = format!(
            "#define RESOURCE \"{resource}\"\n#if __has_embed(RESOURCE) != \
             __STDC_EMBED_FOUND__\n#error missing resource\n#endif\nunsigned char \
             bytes[]={{\n#embed RESOURCE prefix(9,) suffix(,8) limit(3)\n}};\nunsigned char \
             empty[]={{\n#embed RESOURCE limit(0) if_empty(7)\n}};\nint following;\n"
        );
        for mode in [
            "c23", "c2y", "gnu89", "gnu99", "gnu11", "gnu17", "gnu23", "gnu2y",
        ] {
            let flag = format!("-std={mode}");
            let flags = if mode.starts_with("gnu") {
                vec![flag.as_str()]
            } else {
                vec![flag.as_str(), "-pedantic-errors"]
            };
            let tree = clean_tree(&run(&flags, &source));
            for value in [
                "constant 9",
                "constant 0",
                "constant 65",
                "constant 255",
                "constant 8",
                "constant 7",
                "following",
            ] {
                assert!(tree.contains(value), "{mode}, {value}: {tree}");
            }
        }
        let output = run(&["-std=c17"], &source);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("error:"),
            "{output:?}"
        );
    }

    #[test]
    fn queried_c_attributes_are_retained_after_macro_expansion() {
        for attribute in [
            "deprecated",
            "fallthrough",
            "nodiscard",
            "maybe_unused",
            "noreturn",
            "unsequenced",
            "reproducible",
        ] {
            let source = format!(
                "#define ATTR [[{attribute}]]\n#if __has_c_attribute(__{attribute}__)\nATTR int \
                 f(void){{return 1;}}\n#else\n#error missing attribute\n#endif\n"
            );
            for mode in ["c23", "c2y", "gnu17"] {
                let flag = format!("-std={mode}");
                let tree = clean_tree(&run(&[&flag], &source));
                assert!(tree.contains(attribute), "{mode}, {attribute}: {tree}");
                assert!(tree.contains("attribute"), "{tree}");
            }
        }
        let tree = clean_tree(&run(
            &["-std=c23", "-pedantic-errors"],
            "#if __has_c_attribute(unknown) || __has_c_attribute(vendor::unknown)\n#error unknown \
             attribute\n#endif\nint following;\n",
        ));
        assert!(tree.contains("following"), "{tree}");
    }
    #[test]
    fn older_modes_preserve_nonreserved_identifiers_and_reject_new_literal_syntax() {
        let source = "#define NAME bool\nint NAME, true, false, nullptr, constexpr, \
                      static_assert, thread_local, alignas, alignof, typeof, typeof_unqual;\nint \
                      f(void){return bool+true+false+nullptr;}\n";
        for mode in ["c89", "iso9899:199409", "c99", "c11", "c17"] {
            let flag = format!("-std={mode}");
            let tree = clean_tree(&run(&[&flag, "-pedantic-errors"], source));
            assert!(
                tree.contains("declarator bool") && tree.contains("identifier nullptr"),
                "{mode}: {tree}"
            );
        }
        for (mode, source) in [
            ("c99", "const char *p=u8\"x\";\nint following;\n"),
            ("c17", "int n=0b10;\nint following;\n"),
            ("c17", "int n=7uwb;\nint following;\n"),
            ("c23", "int n=0o7;\nint following;\n"),
        ] {
            let flag = format!("-std={mode}");
            let output = run(&[&flag], source);
            let tree = String::from_utf8_lossy(&output.stderr);
            assert!(
                tree.contains("error:") && tree.contains("declarator following"),
                "{mode}: {tree}"
            );
        }
    }

    #[test]
    fn cross_phase_errors_preserve_following_declarations() {
        for (flags, source, diagnostic) in [
            (
                vec!["-std=c23"],
                "#if __has_c_attribute(unknown + 1)\n#endif\nint following;\n",
                "expected an identifier in preprocessing feature query",
            ),
            (
                vec!["-std=c23"],
                "unsigned char bytes[]={\n#embed \"missing.bin\"\n};\nint following;\n",
                "embedded resource not found",
            ),
            (
                vec!["-std=c17", "-fms-extensions"],
                "#define PRAGMA(x) __pragma(x)\nPRAGMA(STDC FP_CONTRACT BAD)\nint following;\n",
                "FP_CONTRACT",
            ),
            (vec!["-std=gnu17"], "int n=2ij;\nint following;\n", "error:"),
            (
                vec!["-std=c23"],
                "#define ATTR [[maybe_unused(]\nATTR int broken;\nint following;\n",
                "error:",
            ),
        ] {
            let output = run(&flags, source);
            let tree = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success() && tree.contains("error:") && tree.contains(diagnostic),
                "{flags:?}: {tree}"
            );
            assert!(tree.contains("declarator following"), "{flags:?}: {tree}");
        }
    }

    #[test]
    fn opaque_syntax_tokens_do_not_print_numeric_sentinels() {
        for (flags, source, expected) in [
            (
                vec!["-std=c23"],
                "[[deprecated(\"old\")]] int x;\n",
                "deprecated",
            ),
            (
                vec!["-std=gnu17"],
                "int (*p)(int) __attribute__((nonnull(1)));\n",
                "token 1\n",
            ),
            (
                vec!["-fms-extensions"],
                "__declspec(align(16)) int x; int f(void){__asm { mov eax, 42 }}\n",
                "token 42\n",
            ),
        ] {
            let output = run(&flags, source);
            let tree = clean_tree(&output);
            assert!(
                !tree.contains('\0'),
                "opaque syntax leaked a sentinel: {tree:?}"
            );
            // Locations are requested by run(), so the token is followed by its
            // source location.
            assert!(tree.contains(expected.trim_end()), "{tree}");
        }
    }
}
