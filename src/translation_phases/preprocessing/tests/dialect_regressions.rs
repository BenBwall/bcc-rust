//! Regressions for standard- and dialect-gated preprocessing found in review.

use std::{
    sync::mpsc,
    time::Duration,
};

use super::{
    language_modes::{
        mode,
        observe,
        observe_paths,
        spellings,
    },
    *,
};
use crate::configuration::MsvcFeature;

/// Runs `observe_paths` on another thread and fails, rather than hanging the
/// suite, when preprocessing does not finish.
fn observe_terminating(
    source: &str,
    config: CompilerConfiguration,
    directories: Vec<PathBuf>,
) -> (Vec<String>, Vec<String>) {
    let (sender, receiver) = mpsc::channel();
    let owned = source.to_owned();
    drop(std::thread::spawn(move || {
        drop(sender.send(observe_paths(
            &owned,
            config,
            PathBuf::from("<test>"),
            &directories,
        )));
    }));
    receiver
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("preprocessing did not terminate: {source}"))
}

fn language_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/language")
}

/// C23 §6.10.4.2p1: a limit is evaluated as a constant expression where it
/// appears, even when the query comes from a macro's replacement list.
#[test]
fn macro_sourced_embed_limit_evaluates_in_its_own_frame() {
    for source in [
        "#define HE __has_embed(<embed.bin> limit(1))\n#if HE\nyes\n#else\nno\n#endif\nafter\n",
        "#define ONE 1\n#define HE __has_embed(<embed.bin> limit(ONE))\n#if HE == \
         __STDC_EMBED_FOUND__\nyes\n#endif\nafter\n",
        "#define L limit(0)\n#if __has_embed(<embed.bin> L) == \
         __STDC_EMBED_EMPTY__\nyes\n#endif\nafter\n",
    ] {
        let (tokens, errors) =
            observe_terminating(source, mode(CStandard::C23), vec![language_fixtures()]);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        let tokens = spellings(&tokens);
        assert!(tokens.contains("identifier `yes`"), "{source}: {tokens}");
        assert!(!tokens.contains("identifier `no`"), "{source}: {tokens}");
        assert_eq!(
            tokens.matches("identifier `after`").count(),
            1,
            "{source}: {tokens}"
        );
    }
}

/// The spellings of the output tokens, separated by spaces.
fn texts(output: &[String]) -> String {
    output
        .iter()
        .map(|line| {
            if let Some((_, rest)) = line.split_once('`') {
                rest.split_once('`').map_or(rest, |(spelling, _)| spelling)
            } else {
                line.split_once("string literal ")
                    .map_or(&**line, |(_, rest)| rest)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// C23 §6.10.5.1p7: a `__VA_OPT__` result is a substitution; it is rescanned
/// with the rest of the replacement list but never substituted again.
#[test]
fn va_opt_results_are_not_substituted_again() {
    for (source, expected) in [
        (
            "#define CALL(f, ...) f(__VA_OPT__(__VA_ARGS__))\nCALL(g, f)\n",
            "g ( f )",
        ),
        (
            "#define P(x, ...) __VA_OPT__(x __VA_ARGS__)\nP(1, x)\n",
            "1 x",
        ),
    ] {
        for config in [
            mode(CStandard::C23),
            mode(CStandard::C17).with_gnu_extensions(true),
        ] {
            let (tokens, errors) = observe(source, config);
            assert!(errors.is_empty(), "{source}: {errors:?}");
            assert_eq!(texts(&tokens), expected, "{source}");
        }
    }
}

/// C23 §6.10.5.1 EXAMPLE 1 and EXAMPLE 2, pp. 179-180; PDF pp. 192-193.
#[test]
fn va_opt_follows_the_standard_examples() {
    let source = "#define LPAREN() (\n#define G(Q) 42\n#define F(R, X, ...) __VA_OPT__(G R X) \
                  )\nint x = F(LPAREN(), 0, <:-);\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(texts(&tokens), "int x = 42 ;");
    let definitions =
        "#define F(...) f(0 __VA_OPT__(,) __VA_ARGS__)\n#define G(X, ...) f(0, X __VA_OPT__(,) \
         __VA_ARGS__)\n#define SDEF(sname, ...) S sname __VA_OPT__(= { __VA_ARGS__ })\n#define \
         EMP\n#define H2(X, Y, ...) __VA_OPT__(X ## Y,) __VA_ARGS__\n#define H3(X, ...) \
         #__VA_OPT__(X##X X##X)\n#define H4(X, ...) __VA_OPT__(a X ## X) ## b\n#define H5A(...) \
         __VA_OPT__()/**/__VA_OPT__()\n#define H5B(X) a ## X ## b\n#define H5C(X) H5B(X)\n";
    for (use_, expected) in [
        ("F(a, b, c)", "f ( 0 , a , b , c )"),
        ("F()", "f ( 0 )"),
        ("F(EMP)", "f ( 0 )"),
        ("G(a, b, c)", "f ( 0 , a , b , c )"),
        ("G(a, )", "f ( 0 , a )"),
        ("G(a)", "f ( 0 , a )"),
        ("SDEF(foo);", "S foo ;"),
        ("SDEF(bar, 1, 2);", "S bar = { 1 , 2 } ;"),
        ("H2(a, b, c, d)", "ab , c , d"),
        ("H3(, 0)", "\"\""),
        ("H4(, 1)", "a b"),
        ("H5C(H5A())", "ab"),
    ] {
        let source = format!("{definitions}{use_}\n");
        let (tokens, errors) = observe(&source, mode(CStandard::C23));
        assert!(errors.is_empty(), "{use_}: {errors:?}");
        assert_eq!(texts(&tokens), expected, "{use_}");
    }
}

/// C23 §6.10.5.1p7: the operand of `#` is the result before rescanning, so a
/// function-like name it contains is not invoked.
#[test]
fn stringified_va_opt_is_not_rescanned() {
    let source = "#define G(x) [x]\n#define F(R, ...) #__VA_OPT__(R (1))\nF(G, z)\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(texts(&tokens), "\"G (1)\"");
}

/// C23 §6.10.5p5: `__VA_OPT__` outside a variadic macro is reported once, at
/// its definition, under the extension policy.
#[test]
fn va_opt_in_a_non_variadic_macro_is_reported_once() {
    for policy in [
        ExtensionPolicy::Allow,
        ExtensionPolicy::Warn,
        ExtensionPolicy::Deny,
    ] {
        let source = "#define F(x) __VA_OPT__(x)\nF(1) F(2)\nint z;\n";
        let (tokens, errors) = observe(source, CompilerConfiguration::new(CStandard::C23, policy));
        assert_eq!(errors.len(), 1, "{policy:?}: {errors:?}");
        assert!(
            errors[0].contains("`__VA_OPT__` can only appear"),
            "{errors:?}"
        );
        assert!(
            errors[0].starts_with(if policy == ExtensionPolicy::Deny {
                "Error:"
            } else {
                "Warning:"
            }),
            "{errors:?}"
        );
        assert!(texts(&tokens).ends_with("int z ;"), "{tokens:?}");
    }
}

/// MSVC comma elision needs the expanded variadic argument only where a
/// comma precedes `__VA_ARGS__`; an argument used only by `#` is never
/// macro-replaced (C99 §6.10.3.1p1).
#[test]
fn msvc_comma_elision_does_not_expand_unused_variadic_arguments() {
    let source = "#define S(x,...) #__VA_ARGS__, x\nconst char *p[] = { S(1, \
                  __has_include(\"a.h\")) };\n#define C(x, ...) f(x, __VA_ARGS__)\nC(1) C(2, 3)\n";
    let config = mode(CStandard::C17)
        .with_gnu_extensions(true)
        .with_msvc_feature(MsvcFeature::VaArgs, true);
    let (tokens, errors) = observe(source, config);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        texts(&tokens),
        "const char * p [ ] = { \"__has_include(\\\"a.h\\\")\" , 1 } ; f ( 1 ) f ( 2 , 3 )"
    );
}

/// A lexer extension diagnostic belongs to the source text, so it is
/// reported once however often phase 4 reads its token: in a macro body, a
/// prescanned argument, or after a lookahead rewinds. Skipped groups stay
/// silent (C99 §6.10.1p6).
#[test]
fn lexer_extension_diagnostics_are_reported_once_per_token() {
    for (standard, source, feature) in [
        (
            CStandard::C99,
            "#define ID(x) x\nint ID($a);\nint ID(ID($b));\n",
            "'$'",
        ),
        (
            CStandard::C89,
            "#define ONE 1 // c\nint a = ONE; int b = ONE;\n",
            "'//'",
        ),
        (CStandard::C99, "#define F(x) x\nint F\n$a;\n", "'$'"),
        (
            CStandard::C99,
            "#define F(x) x\nint F($c) + F($c);\n",
            "'$'",
        ),
    ] {
        let (tokens, errors) = observe(
            source,
            CompilerConfiguration::new(standard, ExtensionPolicy::Warn).with_gnu_extensions(true),
        );
        let expected = source.matches(&feature[1..feature.len() - 1]).count();
        let reported = errors
            .iter()
            .filter(|error| error.contains(feature))
            .count();
        assert_eq!(reported, expected, "{source}: {errors:?}");
        assert!(!texts(&tokens).is_empty(), "{source}");
    }
    let (_, errors) = observe(
        "#if 0\nint $a; // c\n#elif 0\n$b\n#endif\nint after;\n",
        CompilerConfiguration::new(CStandard::C89, ExtensionPolicy::Warn).with_gnu_extensions(true),
    );
    assert!(errors.is_empty(), "{errors:?}");
}

/// A header-name operand read from a replacement list ends with that list:
/// its new-line belongs to the macro's frame, and the conditional that
/// follows still selects its groups (C99 §6.10p2, C23 §6.10.2p7).
#[test]
fn unterminated_macro_resource_operand_stops_at_its_replacement_list() {
    for operand in [
        "__has_include(<foo.h",
        "__has_embed(<foo.bin",
        "__has_include(\"foo.h",
    ] {
        let source = format!("#define HI {operand}\n#if HI\na\n#else\nb\n#endif\nint after;\n");
        let (tokens, errors) = observe(&source, mode(CStandard::C23));
        assert!(!errors.is_empty(), "{source}");
        assert!(
            errors
                .iter()
                .all(|error| !error.contains("#endif") && !error.contains("#else")),
            "{source}: {errors:?}"
        );
        assert_eq!(texts(&tokens), "b int after ;", "{source}: {errors:?}");
    }
}

/// A failed `#embed` still ends at its new-line, so the next line starts a
/// directive (C99 §6.10p2).
#[test]
fn failed_embed_header_name_restores_the_line_start() {
    for directive in [
        "#embed <missing",
        "#embed \"missing",
        "#embed <missing> limit(",
        "#embed",
    ] {
        let source = format!("{directive}\n#define X 1\nint z = X;\n");
        let (tokens, errors) = observe(&source, mode(CStandard::C23));
        assert_eq!(errors.len(), 1, "{source}: {errors:?}");
        assert!(!errors[0].contains("stray"), "{source}: {errors:?}");
        assert_eq!(texts(&tokens), "int z = 1 ;", "{source}");
    }
}

/// C23 adds `#elifdef` and `#elifndef` (§6.10.2p16); GNU modes accept them
/// earlier as an extension, while strict C89-C17 leave them unrecognized, so
/// a skipped one is ignored and an active one is an unknown directive.
#[test]
fn elifdef_selects_groups_by_mode_on_skip_and_active_paths() {
    let skip = "#define YES\n#if 0\nskipped\n#elifdef YES\nyes\n#else\nno\n#endif\n#if \
                0\n#elifndef YES\nwrong\n#elifndef NO\nyes2\n#endif\nafter\n";
    let active = "#define YES\n#if 1\nfirst\n#elifdef YES\nwrong\n#elifndef \
                  YES\nwrong\n#else\nwrong\n#endif\nafter\n";
    for (standard, gnu) in [
        (CStandard::C89, false),
        (CStandard::C17, false),
        (CStandard::C89, true),
        (CStandard::C17, true),
        (CStandard::C23, false),
        (CStandard::C23, true),
    ] {
        let recognized = gnu || standard >= CStandard::C23;
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            let config = CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu);
            let case = format!("{standard:?} gnu={gnu} {policy:?}");
            let extension =
                recognized && standard < CStandard::C23 && policy != ExtensionPolicy::Allow;
            let severity = if policy == ExtensionPolicy::Deny {
                "Error:"
            } else {
                "Warning:"
            };

            let (tokens, errors) = observe(skip, config);
            if recognized {
                assert_eq!(texts(&tokens), "yes yes2 after", "{case}");
                // Each recognized directive that is reached is reported.
                assert_eq!(
                    errors.len(),
                    if extension { 3 } else { 0 },
                    "{case}: {errors:?}"
                );
                assert!(
                    errors
                        .iter()
                        .all(|error| error.starts_with(severity)
                            && error.contains("is a C23 extension")),
                    "{case}: {errors:?}"
                );
            } else {
                assert_eq!(texts(&tokens), "no after", "{case}");
                assert!(errors.is_empty(), "{case}: {errors:?}");
            }

            let (tokens, errors) = observe(active, config);
            if recognized {
                assert_eq!(texts(&tokens), "first after", "{case}");
                assert_eq!(errors.len(), usize::from(extension), "{case}: {errors:?}");
            } else {
                assert_eq!(texts(&tokens), "first wrong wrong after", "{case}");
                assert_eq!(errors.len(), 2, "{case}: {errors:?}");
                assert!(
                    errors
                        .iter()
                        .all(|error| error.contains("unknown preprocessing directive")),
                    "{case}: {errors:?}"
                );
            }
        }
    }
}

/// Diagnostics for `#elifdef` and `#elifndef` name the directive written.
#[test]
fn elifdef_diagnostics_name_their_directive() {
    for (source, expected) in [
        ("#if 0\n#elifdef 1\n#endif\n", "after `#elifdef`"),
        ("#if 0\n#elifndef 1\n#endif\n", "after `#elifndef`"),
        (
            "#if 0\n#elifdef A B\n#endif\n",
            "end of `#elifdef` directive",
        ),
        (
            "#if 0\n#elifndef A B\n#endif\n",
            "end of `#elifndef` directive",
        ),
        (
            "#if 1\n#else\n#elifdef A\n#endif\n",
            "`#elifdef` after `#else`",
        ),
        (
            "#if 0\n#else\n#elifndef A\n#endif\n",
            "`#elifndef` after `#else`",
        ),
        ("#elifdef A\n", "`#elifdef` without `#if`"),
        ("#elifndef A\n", "`#elifndef` without `#if`"),
    ] {
        let (_, errors) = observe(&format!("{source}int z;\n"), mode(CStandard::C23));
        assert_eq!(errors.len(), 1, "{source}: {errors:?}");
        assert!(errors[0].contains(expected), "{source}: {errors:?}");
    }
    let (_, errors) = observe(
        "#if 0\n#elifndef A\n#endif\n",
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Warn).with_gnu_extensions(true),
    );
    assert_eq!(errors, ["Warning: '#elifndef' is a C23 extension"]);
}

/// `#warning` is reported like `#error`: its message is trimmed and the
/// directive is named, even when the message is empty (C23 §6.10.7).
#[test]
fn warning_directive_formats_its_message_like_error() {
    let (tokens, errors) = observe(
        "#warning late  \n#warning\n#error  stop \nint z;\n",
        mode(CStandard::C23),
    );
    assert_eq!(texts(&tokens), "int z ;");
    assert_eq!(
        errors,
        [
            "Warning: #warning late",
            "Warning: #warning",
            "Error: #error stop"
        ]
    );
}

/// C23 §6.7.13.2p2 lists `_Noreturn` among the standard attributes, with
/// the value of `noreturn` (§6.7.13.7p1, p5).
#[test]
fn has_c_attribute_reports_noreturn_spellings() {
    for operand in ["noreturn", "_Noreturn", "__noreturn__"] {
        let source =
            format!("#if __has_c_attribute({operand}) == 202311L\nyes\n#else\nno\n#endif\n");
        let (tokens, errors) = observe(&source, mode(CStandard::C23));
        assert!(errors.is_empty(), "{operand}: {errors:?}");
        assert_eq!(texts(&tokens), "yes", "{operand}");
    }
}

/// C23 §6.10.1p1: an embed parameter's clause is optional in the grammar; a
/// query with a parameter it does not support evaluates to
/// `__STDC_EMBED_NOT_FOUND__` (C23 §6.10.2p10) without a diagnostic.
#[test]
fn has_embed_accepts_parameters_without_a_clause() {
    for parameters in [
        "vendor::flag",
        "vendor :: flag",
        "vendor::flag limit(1)",
        "vendor::flag(1) vendor::other",
    ] {
        let source = format!(
            "#if __has_embed(<embed.bin> {parameters}) == \
             __STDC_EMBED_NOT_FOUND__\nyes\n#endif\nafter\n"
        );
        let (tokens, errors) =
            observe_terminating(&source, mode(CStandard::C23), vec![language_fixtures()]);
        assert!(errors.is_empty(), "{parameters}: {errors:?}");
        assert_eq!(texts(&tokens), "yes after", "{parameters}");
    }
    let (_, errors) = observe_terminating(
        "#if __has_embed(<embed.bin> limit)\n#endif\nafter\n",
        mode(CStandard::C23),
        vec![language_fixtures()],
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
}

/// A named variadic parameter that is not last is reported under the
/// macro's name (GNU named variadic macros; C99 §6.10.3p12 for `...`).
#[test]
fn misplaced_named_variadic_parameter_names_its_macro() {
    let (tokens, errors) = observe(
        "#define F(a, args..., b) x\nint z;\n",
        mode(CStandard::C17).with_gnu_extensions(true),
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("`F`"), "{errors:?}");
    assert_eq!(texts(&tokens), "int z ;");
}
