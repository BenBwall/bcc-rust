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
    source: &'static str,
    config: CompilerConfiguration,
    directories: Vec<PathBuf>,
) -> (Vec<String>, Vec<String>) {
    let (sender, receiver) = mpsc::channel();
    drop(std::thread::spawn(move || {
        drop(sender.send(observe_paths(
            source,
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
