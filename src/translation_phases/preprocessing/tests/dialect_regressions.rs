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
        texts,
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

/// C23 §6.10.4.2p3: limits use conditional-inclusion rules, except `defined`.
#[test]
fn embed_limits_allow_conditional_queries() {
    for limit in [
        "__has_include(<embed.bin>)",
        "__has_embed(<embed.bin> limit(1))",
        "(__has_c_attribute(maybe_unused) != 0)",
        "QUERY",
        "IDENTITY(QUERY)",
    ] {
        let source = format!(
            "#define IDENTITY(x) x\n#define QUERY __has_include(<embed.bin>)\n#embed <embed.bin> \
             limit({limit})\n#define AFTER 7\nAFTER\n"
        );
        let (tokens, errors) =
            observe_terminating(&source, mode(CStandard::C23), vec![language_fixtures()]);
        assert!(errors.is_empty(), "{limit}: {errors:?}");
        assert_eq!(texts(&tokens), "0 7", "{limit}");
    }
    for limit in ["defined(QUERY)", "DEFINED"] {
        let source = format!(
            "#define QUERY 1\n#define DEFINED defined(QUERY)\n#embed <embed.bin> \
             limit({limit})\nint after;\n"
        );
        let (tokens, errors) =
            observe_terminating(&source, mode(CStandard::C23), vec![language_fixtures()]);
        assert_eq!(
            errors,
            ["Error: defined is not permitted in an embed limit"]
        );
        assert_eq!(texts(&tokens), "int after ;");
    }
    let (tokens, errors) = observe_terminating(
        "#embed <embed.bin> limit(__has_include(<embed.bin>))\n__has_include(<embed.bin>)\nint \
         after;\n",
        mode(CStandard::C23),
        vec![language_fixtures()],
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("require a preprocessing conditional expression"));
    assert_eq!(texts(&tokens), "0 0 int after ;");
    let (_, errors) = observe_terminating(
        "#embed <embed.bin> prefix(__has_include(<embed.bin>),) limit(1)\nint after;\n",
        mode(CStandard::C23),
        vec![language_fixtures()],
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("require a preprocessing conditional expression"));
}

/// Query header-name characters are not limit expression operators.
#[test]
fn embed_limit_queries_preserve_header_name_tokens() {
    for query in [
        "__has_include(<defined>)",
        "__has_include(<open(>)",
        "__has_embed(<missing[resource{>)",
    ] {
        let source = format!("#embed <embed.bin> limit({query})\n#define AFTER 7\nAFTER\n");
        let (tokens, errors) =
            observe_terminating(&source, mode(CStandard::C23), vec![language_fixtures()]);
        assert!(errors.is_empty(), "{query}: {errors:?}");
        assert_eq!(texts(&tokens), "7", "{query}");
    }
}

/// C23 §6.10.1p1: parameter clauses balance parentheses, brackets and braces.
#[test]
fn embed_parameter_clauses_reject_mismatched_delimiters() {
    for body in ["]", "}", "[)", "{)", "([)]", "[", "{", "{[}]"] {
        for parameter in ["prefix", "suffix", "if_empty"] {
            for query in [false, true] {
                let clause = format!("{parameter}({body})");
                let source = if query {
                    format!(
                        "#if __has_embed(<embed.bin> \
                         {clause})\nwrong\n#else\nrecovered\n#endif\n#define AFTER 7\nAFTER\n"
                    )
                } else {
                    format!("#embed <embed.bin> {clause}\n#define AFTER 7\nAFTER\n")
                };
                let (tokens, errors) =
                    observe_terminating(&source, mode(CStandard::C23), vec![language_fixtures()]);
                assert!(!errors.is_empty(), "{source}: {tokens:?}");
                assert_eq!(
                    texts(&tokens),
                    if query { "recovered 7" } else { "7" },
                    "{source}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn embed_parameter_clauses_accept_nested_mixed_delimiters() {
    let source = "#if __has_embed(<embed.bin> prefix([{(9)}]))\nyes\n#endif\n#embed <embed.bin> \
                  limit(1) prefix([{(9)}],) suffix(,[{(8)}])\n#embed <embed.bin> limit(0) \
                  if_empty([{(7)}])\nafter\n";
    let (tokens, errors) =
        observe_terminating(source, mode(CStandard::C23), vec![language_fixtures()]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        texts(&tokens),
        "yes [ { ( 9 ) } ] , 0 , [ { ( 8 ) } ] [ { ( 7 ) } ] after"
    );
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

/// C23 §6.10.4.1p7 and §6.10.4.3-5: resource elements and surrounding
/// parameters retain their exact order and multiplicity after expansion.
#[test]
fn embed_initializers_preserve_order_and_multiplicity() {
    let source = "#define PREFIX 9,\n#define SUFFIX ,8\n#define EMPTY 7\nunsigned char \
                  bytes[]={\n#embed <embed.bin> limit(3) prefix(PREFIX) \
                  suffix(SUFFIX)\n};\nunsigned char empty[]={\n#embed <embed.bin> limit(0) \
                  prefix(PREFIX) suffix(SUFFIX) if_empty(EMPTY)\n};\n";
    let (tokens, errors) =
        observe_terminating(source, mode(CStandard::C23), vec![language_fixtures()]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        texts(&tokens),
        "unsigned char bytes [ ] = { 9 , 0 , 65 , 255 , 8 } ; unsigned char empty [ ] = { 7 } ;"
    );
}

/// GNU/MSVC operators in ordinary text follow the whitespace rule for
/// invocations (C99 §6.10.3p10), including nested pragma parentheses.
#[test]
fn preprocessing_operator_arguments_accept_newlines_in_ordinary_text() {
    for (source, expected) in [
        (
            "__pragma(\n    warning(disable: 4068)\n)\nint x;\n",
            "int x ;",
        ),
        ("__pragma\n(\nSTDC\nFP_CONTRACT\nON\n)\nint x;\n", "int x ;"),
        (
            "int x = __has_builtin(\n__builtin_va_arg\n);\nint after;\n",
            "int x = 1 ; int after ;",
        ),
        (
            "int x = __has_builtin\n(\n__builtin_va_arg\n);\nint after;\n",
            "int x = 1 ; int after ;",
        ),
        (
            "int x = __has_attribute(\nunused\n);\nint after;\n",
            "int x = 1 ; int after ;",
        ),
    ] {
        for config in [
            mode(CStandard::C17).with_msvc_feature(MsvcFeature::Pragma, true),
            mode(CStandard::C17)
                .with_gnu_extensions(true)
                .with_msvc_feature(MsvcFeature::Pragma, true),
            mode(CStandard::C23).with_msvc_feature(MsvcFeature::Pragma, true),
        ] {
            let (tokens, errors) = observe(source, config);
            assert!(errors.is_empty(), "{source}: {errors:?}");
            assert_eq!(texts(&tokens), expected, "{source}");
        }
    }
}

/// C99 §6.10p2: a directive ends at its first new-line even inside an
/// operator; recovery must leave the enclosing conditional intact.
#[test]
fn preprocessing_operator_arguments_keep_directive_newline_boundaries() {
    let config = mode(CStandard::C17).with_gnu_extensions(true);
    for directive in ["#if", "#if 0\nwrong\n#elif"] {
        for operator in ["__has_builtin(", "__has_builtin", "__has_attribute("] {
            let source = format!(
                "{directive} {operator}\nunused\n)\n#else\nint recovered;\n#endif\nint after;\n"
            );
            let (tokens, errors) = observe(&source, config);
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            assert!(errors[0].contains("preprocessing operator"), "{errors:?}");
            assert_eq!(texts(&tokens), "int recovered ; int after ;", "{source}");
        }
    }
    // #line expands its operand too, but is not a conditional expression.
    let (tokens, errors) = observe(
        "#line __has_builtin(\n__builtin_va_arg\n)\nint after;\n",
        config,
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("unterminated preprocessing operator")),
        "{errors:?}"
    );
    assert_eq!(texts(&tokens), "__builtin_va_arg ) int after ;");
}

/// A malformed macro-sourced operand keeps its existing line recovery
/// rather than consuming the following declaration (C99 §6.10p2).
#[test]
fn preprocessing_operator_arguments_keep_macro_newline_recovery() {
    let (tokens, errors) = observe(
        "#define Q __has_builtin(\nQ\nint after;\n",
        mode(CStandard::C17).with_gnu_extensions(true),
    );
    assert_eq!(errors, ["Error: unterminated preprocessing operator"]);
    assert_eq!(texts(&tokens), "0 int after ;");
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

/// C23 §6.10.5.1p7: a `__VA_OPT__` result is a single operand of a `##`
/// chain, and a chain inside it pastes every parameter's argument (C99
/// §6.10.3.3p3). Expected spellings come from `clang -std=c2x -E -P`.
#[test]
fn va_opt_results_paste_within_and_beside_paste_chains() {
    for (source, expected) in [
        (
            "#define V(a, ...) a ## __VA_OPT__(x ## a) ## b\nV(p,1) V(p) V(,1)\n",
            "pxpb pb xb",
        ),
        (
            "#define V(a, ...) [__VA_OPT__(a##a##a)]\nV(q,1) V(,1)\n",
            "[ qqq ] [ ]",
        ),
    ] {
        let (tokens, errors) = observe(source, mode(CStandard::C23));
        assert!(errors.is_empty(), "{source}: {errors:?}");
        assert_eq!(texts(&tokens), expected, "{source}");
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

/// C23 §6.10.5.2p3: stringizing removes boundary whitespace and placemarkers
/// while collapsing whitespace between significant tokens.
#[test]
fn stringified_va_opt_trims_whitespace_and_placemarkers() {
    for (source, expected) in [
        ("#define F(...) #__VA_OPT__(   abc   )\nF(x)\n", "\"abc\""),
        (
            "#define F(...) #__VA_OPT__(   abc   def   )\nF(x)\n",
            "\"abc def\"",
        ),
        (
            "#define F(X, ...) #__VA_OPT__( X ## X   abc   X ## X )\nF(, x)\n",
            "\"abc\"",
        ),
        ("#define F(...) #__VA_OPT__(   )\nF(x)\n", "\"\""),
        ("#define F(...) #__VA_OPT__(   abc   )\nF()\n", "\"\""),
    ] {
        for config in [
            mode(CStandard::C23),
            mode(CStandard::C23).with_gnu_extensions(true),
            mode(CStandard::C17).with_gnu_extensions(true),
        ] {
            let (tokens, errors) = observe(source, config);
            assert!(errors.is_empty(), "{source}: {errors:?}");
            assert_eq!(texts(&tokens), expected, "{source}");
        }
    }
}

/// GNU comma-paste is prepared before ordinary pasting in a selected
/// optional replacement, just as in the outer replacement list.
#[test]
fn va_opt_applies_gnu_comma_paste_before_substitution() {
    for (source, expected) in [
        (
            "#define F(...) f(0 __VA_OPT__(, ## __VA_ARGS__))\nF(1,2)\n",
            "f ( 0 , 1 , 2 )",
        ),
        (
            "#define ONE 1\n#define F(...) f(0 __VA_OPT__(, ## __VA_ARGS__))\nF(ONE,2)\n",
            "f ( 0 , 1 , 2 )",
        ),
        (
            "#define F(...) #__VA_OPT__(, ## __VA_ARGS__)\nF(1,2)\n",
            "\",1,2\"",
        ),
        (
            "#define F(...) f(0 __VA_OPT__(, ## __VA_ARGS__))\nF()\n",
            "f ( 0 )",
        ),
        (
            "#define EMPTY\n#define F(...) f(0 __VA_OPT__(, ## __VA_ARGS__))\nF(EMPTY)\n",
            "f ( 0 )",
        ),
    ] {
        for config in [
            mode(CStandard::C23),
            mode(CStandard::C23).with_gnu_extensions(true),
            mode(CStandard::C17).with_gnu_extensions(true),
        ] {
            let (tokens, errors) = observe(source, config);
            assert!(errors.is_empty(), "{source}: {errors:?}");
            assert_eq!(texts(&tokens), expected, "{source}");
        }
    }

    for gnu in [false, true] {
        for (policy, severity) in [
            (ExtensionPolicy::Warn, "Warning"),
            (ExtensionPolicy::Deny, "Error"),
        ] {
            let config =
                CompilerConfiguration::new(CStandard::C23, policy).with_gnu_extensions(gnu);
            let source = "#define F(...) f(0 __VA_OPT__(, ## __VA_ARGS__))\nF(1,2)\n";
            let (tokens, errors) = observe(source, config);
            assert_eq!(texts(&tokens), "f ( 0 , 1 , 2 )", "{errors:?}");
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert_eq!(
                errors[0],
                format!("{severity}: ', ## __VA_ARGS__' is a GNU extension")
            );
        }
    }
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

/// `__STRICT_ANSI__` is not one of the predefined macro names of C99
/// §6.10.8, so like GCC it may be undefined and then defined again.
#[test]
fn strict_ansi_may_be_undefined() {
    let source = "#undef __STRICT_ANSI__\n#ifdef \
                  __STRICT_ANSI__\nwrong\n#else\nok\n#endif\n#define __STRICT_ANSI__ 2\nint z = \
                  __STRICT_ANSI__;\n";
    let (tokens, errors) = observe(source, mode(CStandard::C99));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(texts(&tokens), "ok int z = 2 ;");
    let (_, errors) = observe("#undef __STDC__\n", mode(CStandard::C99));
    assert_eq!(errors.len(), 1, "{errors:?}");
}

/// The GNU imaginary suffix comes off a constant without changing its other
/// characters: in `0x1if` the `f` is a suffix, not a hexadecimal digit, and
/// an integer takes no `f` suffix (C99 §6.4.4.1p1).
#[test]
fn imaginary_suffix_does_not_turn_a_suffix_into_a_digit() {
    let config = mode(CStandard::C17).with_gnu_extensions(true);
    for invalid in ["0x1if", "0X1IF", "0x1Fif", "1if"] {
        let (_, errors) = observe(&format!("{invalid};\n"), config);
        assert_eq!(errors.len(), 1, "{invalid}: {errors:?}");
    }
    for valid in [
        "0x1i",
        "0x1fi",
        "0x1p0if",
        "0x1.8p1Fi",
        "1.0if",
        "1i",
        "0x1il",
    ] {
        let (tokens, errors) = observe(&format!("{valid};\n"), config);
        assert!(errors.is_empty(), "{valid}: {errors:?}");
        assert_eq!(tokens.len(), 2, "{valid}: {tokens:?}");
    }
}

/// The characters of a written `<...>` header name are not identifiers
/// (C99 §6.4.7), so `$` in one is no extension, as in `__has_include`.
#[test]
fn dollar_in_an_angle_header_name_is_not_an_identifier_extension() {
    for config in [
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
        CompilerConfiguration::new(CStandard::C89, ExtensionPolicy::Warn).with_gnu_extensions(true),
    ] {
        let (_, errors) = observe("#include <$sdk/x.h>\nint $a;\n", config);
        assert_eq!(
            errors.iter().filter(|error| error.contains("'$'")).count(),
            1,
            "{errors:?}"
        );
        assert!(
            errors.iter().any(|error| error.contains("x.h")),
            "{errors:?}"
        );
    }
}

/// C23 §6.4.8p1: a digit separator continues a pp-number only before a
/// digit or a `nondigit`, which is ASCII. Before a universal character name,
/// another character, or `$`, the `'` starts a character constant. After
/// `' nondigit`, `e sign` still continues the pp-number, so `0x1'e+1` is one
/// (invalid) pp-number, as in GCC.
#[test]
fn digit_separators_follow_the_pp_number_grammar() {
    for (source, expected) in [
        (r"1'\u00e9' after", r"1 '\u00e9' after"),
        ("1'\\u00e9' after", "1 '\\u00e9' after"),
        ("1'$' after", "1 '$' after"),
        ("1'2 after", "1'2 after"),
    ] {
        let (tokens, _) = observe(&format!("{source}\n"), mode(CStandard::C23));
        assert_eq!(texts(&tokens), expected, "{source}");
    }
    let (tokens, errors) = observe("0x1'e+1 after\n", mode(CStandard::C23));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(spellings(&tokens).contains("0x1'e+1"), "{tokens:?}");
}

/// Before C23, GNU modes accept `typeof` as GCC's own keyword, so its
/// diagnostic names the GNU origin rather than the later standard.
#[test]
fn typeof_before_c23_is_a_gnu_extension() {
    for standard in [CStandard::C89, CStandard::C17] {
        let config =
            CompilerConfiguration::new(standard, ExtensionPolicy::Warn).with_gnu_extensions(true);
        let (_, errors) = observe("typeof(int) x;\n", config);
        assert_eq!(
            errors,
            ["Warning: 'typeof' is a GNU extension"],
            "{standard:?}"
        );
    }
    let config =
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Warn).with_gnu_extensions(true);
    let (_, errors) = observe("typeof(int) x; typeof_unqual(int) y;\n", config);
    assert!(errors.is_empty(), "{errors:?}");
}

/// File-system and definition-time constraints of the C23 directives have
/// their own diagnostics, which name what failed.
#[test]
fn embed_and_va_opt_constraints_have_specific_diagnostics() {
    for (source, expected) in [
        (
            "#embed \"missing.bin\"\n",
            "Error: cannot find embedded resource `missing.bin`",
        ),
        (
            "#embed <missing.bin> limit(1)\n",
            "Error: cannot find embedded resource `missing.bin`",
        ),
        (
            "#define V(...) __VA_OPT__ x\n",
            "Error: expected `(` after `__VA_OPT__`",
        ),
        (
            "#define V(...) __VA_OPT__(__VA_OPT__(x))\n",
            "Error: `__VA_OPT__` cannot be nested",
        ),
        (
            "#define V(...) __VA_OPT__(x\n",
            "Error: unterminated `__VA_OPT__` replacement",
        ),
        (
            "#define V(...) __VA_OPT__(x ##)\n",
            "Error: `##` cannot begin or end a `__VA_OPT__` replacement",
        ),
    ] {
        let (tokens, errors) = observe(&format!("{source}int after;\n"), mode(CStandard::C23));
        assert_eq!(errors, [expected], "{source}");
        assert_eq!(texts(&tokens), "int after ;", "{source}");
    }
    let (_, errors) = observe("#define V(...) __VA_OPT__(x)\n", mode(CStandard::C17));
    assert_eq!(
        errors,
        ["Error: `__VA_OPT__` is not available in this mode"]
    );
}
