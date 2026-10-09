//! Standard/dialect phase-1–7 regression tests and a mode/provenance snapshot.

use std::fmt::Write;

use super::*;
use crate::{
    cli::describe_token,
    configuration::{
        Feature,
        MsvcFeature,
    },
};

pub(super) fn observe(source: &str, config: CompilerConfiguration) -> (Vec<String>, Vec<String>) {
    observe_paths(source, config, PathBuf::from("<test>"), &[])
}

pub(super) fn observe_paths(
    source: &str,
    config: CompilerConfiguration,
    path: PathBuf,
    directories: &[PathBuf],
) -> (Vec<String>, Vec<String>) {
    observe_include_paths(source, config, path, &[], directories)
}

fn observe_include_paths(
    source: &str,
    config: CompilerConfiguration,
    path: PathBuf,
    quote_directories: &[PathBuf],
    system_directories: &[PathBuf],
) -> (Vec<String>, Vec<String>) {
    let tu = crate::util::bump::Bump::new();
    let pp = crate::util::bump::Bump::new();
    let mut context = Context::with_configuration(&tu, config);
    let quote_directories = SharedVec::from(quote_directories.to_vec().into_boxed_slice());
    let system_directories = SharedVec::from(system_directories.to_vec().into_boxed_slice());
    let mut preprocessor = Preprocessor::new(
        &pp,
        &mut context,
        path.into_boxed_path(),
        source,
        quote_directories,
        system_directories,
    );
    let tokens = preprocessor.preprocess_all(&mut context);
    let scratch = crate::util::bump::Bump::new();
    let output = tokens
        .into_iter()
        .map(|token| describe_token(token, &context, &scratch).to_owned())
        .collect();
    let mut errors = Vec::new();
    while let Some(error) = context.pop_pending_error() {
        errors.push(format!("{:?}: {error}", error.severity()));
    }
    (output, errors)
}

pub(super) fn mode(standard: CStandard) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
}
pub(super) fn spellings(output: &[String]) -> String {
    output.join("\n")
}

struct IncludeDirectory(PathBuf);

impl IncludeDirectory {
    fn new(name: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("bcc-include-{name}-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for IncludeDirectory {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

fn token_descriptions(tokens: &[String]) -> Vec<&str> {
    tokens
        .iter()
        .map(|token| token.rsplit_once(": ").unwrap().1)
        .collect()
}

#[test]
fn include_next_uses_each_opening_search_origin() {
    let temp = IncludeDirectory::new("per-opening");
    let a = temp.0.join("a");
    let b = temp.0.join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    std::fs::write(
        a.join("wrapper.h"),
        "#ifdef LOCAL\n#include_next <payload.h>\n#else\ninitial\n#endif\n",
    )
    .unwrap();
    std::fs::write(a.join("neighbor.h"), "#include \"wrapper.h\"\n").unwrap();
    std::fs::write(a.join("payload.h"), "first\n").unwrap();
    std::fs::write(b.join("payload.h"), "wrong\n").unwrap();
    for config in [
        mode(CStandard::C99),
        mode(CStandard::C17).with_gnu_extensions(true),
        mode(CStandard::C23),
    ] {
        for (source, expected) in [
            (
                "#define LOCAL\n#include <neighbor.h>\nafter\n",
                vec!["identifier `first`", "identifier `after`"],
            ),
            (
                "#include <wrapper.h>\n#define LOCAL\n#include <neighbor.h>\nafter\n",
                vec![
                    "identifier `initial`",
                    "identifier `first`",
                    "identifier `after`",
                ],
            ),
        ] {
            let (tokens, errors) = observe_paths(
                source,
                config,
                temp.0.join("main.c"),
                &[a.clone(), b.clone()],
            );
            assert!(errors.is_empty(), "{source}: {errors:?}");
            assert_eq!(token_descriptions(&tokens), expected, "{source}");
        }
    }
}

#[test]
fn include_next_continues_across_quote_and_local_origins() {
    let temp = IncludeDirectory::new("quote-local");
    let q1 = temp.0.join("q1");
    let q2 = temp.0.join("q2");
    let system = temp.0.join("system");
    for path in [&q1, &q2, &system] {
        std::fs::create_dir(path).unwrap();
    }
    // Exercise the configured origin across many macro expansions and a
    // presumed filename change before switching from quote to angle form.
    let wrapper = format!(
        "first\n#define EMPTY\n{}#line 50 \"presumed.h\"\n#include_next <wrapper.h>\n",
        "EMPTY\n".repeat(300)
    );
    std::fs::write(q1.join("wrapper.h"), wrapper).unwrap();
    std::fs::write(
        q2.join("wrapper.h"),
        "second\n#include_next \"wrapper.h\"\n",
    )
    .unwrap();
    std::fs::write(system.join("wrapper.h"), "third\n").unwrap();
    std::fs::write(
        q1.join("local.h"),
        "#ifdef LOCAL\n#include_next <payload.h>\n#else\ninitial\n#endif\n",
    )
    .unwrap();
    std::fs::write(q1.join("neighbor.h"), "#include \"local.h\"\n").unwrap();
    std::fs::write(temp.0.join("root-local.h"), "#include_next <payload.h>\n").unwrap();
    std::fs::write(q1.join("payload.h"), "local_payload\n").unwrap();
    std::fs::write(q2.join("payload.h"), "wrong_quote\n").unwrap();
    std::fs::write(system.join("payload.h"), "wrong_system\n").unwrap();
    for config in [
        mode(CStandard::C99),
        mode(CStandard::C17).with_gnu_extensions(true),
        mode(CStandard::C23),
    ] {
        for (source, expected) in [
            (
                "#include \"wrapper.h\"\nafter\n",
                vec![
                    "identifier `first`",
                    "identifier `second`",
                    "identifier `third`",
                    "identifier `after`",
                ],
            ),
            (
                "#include \"root-local.h\"\nafter\n",
                vec!["identifier `local_payload`", "identifier `after`"],
            ),
            (
                "#include \"wrapper.h\"\n#define LOCAL\n#include \"neighbor.h\"\nafter\n",
                vec![
                    "identifier `first`",
                    "identifier `second`",
                    "identifier `third`",
                    "identifier `local_payload`",
                    "identifier `after`",
                ],
            ),
            (
                "#include \"local.h\"\n#define LOCAL\n#include \"neighbor.h\"\nafter\n",
                vec![
                    "identifier `initial`",
                    "identifier `local_payload`",
                    "identifier `after`",
                ],
            ),
        ] {
            let (tokens, errors) = observe_include_paths(
                source,
                config,
                temp.0.join("main.c"),
                &[q1.clone(), q2.clone()],
                std::slice::from_ref(&system),
            );
            assert!(errors.is_empty(), "{source}: {errors:?}");
            assert_eq!(token_descriptions(&tokens), expected, "{source}");
        }
    }
}

#[test]
fn line_directive_rejects_encoded_filenames_and_recovers() {
    for standard in [
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for gnu in [false, true] {
            for prefix in ["u", "U", "u8", "L"] {
                for expanded in [false, true] {
                    let filename = format!("{prefix}\"new.c\"");
                    let (definition, operand) = if expanded {
                        (format!("#define FILENAME {filename}\n"), "FILENAME")
                    } else {
                        (String::new(), filename.as_str())
                    };
                    let source = format!(
                        "#line 7 \"original.c\"\n{definition}#line 100 {operand}\n__LINE__ \
                         after\n__FILE__\n#line 200 \"valid.c\"\n__LINE__ after\n__FILE__\n"
                    );
                    let (tokens, errors) =
                        observe(&source, mode(standard).with_gnu_extensions(gnu));
                    assert_eq!(
                        errors.len(),
                        1,
                        "{standard:?} GNU={gnu}: {source}: {errors:?}"
                    );
                    assert!(errors[0].starts_with("Error:"), "{errors:?}");
                    assert!(errors[0].contains("`#line` file name"), "{errors:?}");
                    assert_eq!(
                        token_descriptions(&tokens),
                        [
                            "integer constant `100` = 100 (int)",
                            "identifier `after`",
                            "string literal \"original.c\"",
                            "integer constant `200` = 200 (int)",
                            "identifier `after`",
                            "string literal \"valid.c\"",
                        ],
                        "{source}"
                    );
                    assert_eq!(tokens[1], "original.c:100:10: identifier `after`");
                    assert_eq!(tokens[4], "valid.c:200:10: identifier `after`");
                }
            }
        }
    }
}

#[test]
fn comments_digraphs_and_trigraphs_obey_mode_boundaries() {
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for gnu in [false, true] {
            let config = mode(standard).with_gnu_extensions(gnu);
            let (output, _) = observe("a // hidden\nb <: :>\n??=define X yes\nX\n", config);
            let output = spellings(&output);
            assert_eq!(
                output.contains("identifier `hidden`"),
                !config.accepts(Feature::LineComments),
                "{standard:?} {gnu}: {output}"
            );
            assert_eq!(
                output.contains("punctuator `[`"),
                config.accepts(Feature::Digraphs),
                "{standard:?} {gnu}: {output}"
            );
            assert_eq!(
                output.contains("identifier `X`"),
                !config.accepts(Feature::Trigraphs),
                "{standard:?} {gnu}: {output}"
            );
        }
    }
}

#[test]
fn prefixed_literals_and_concatenation_preserve_encoding() {
    let source = "u\"abc\" U'\\U0001f600' u8\"utf8\" u'\\u00e9'\nu8'a'\n";
    let (old, _) = observe(source, mode(CStandard::C99));
    assert!(spellings(&old).contains("identifier `u`"));
    let (c11, errors) = observe(source, mode(CStandard::C11));
    assert!(errors.is_empty(), "{errors:?}");
    let c11 = spellings(&c11);
    assert!(c11.contains("char16_t string literal"), "{c11}");
    assert!(c11.contains("128512 (char32_t)"), "{c11}");
    assert!(c11.contains("identifier `u8`"), "{c11}");
    let (c23, errors) = observe("u8'a' u'a' U'a'\nu\"a\" \"b\"\n", mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    assert!(spellings(&c23).contains("97 (char8_t)"));
    assert!(spellings(&c23).contains("u\"ab\""));
    let (_, errors) = observe("u\"a\" U\"b\"\n", mode(CStandard::C23));
    assert!(errors.iter().any(|e| e.contains("incompatible string")));
}

#[test]
fn modern_numeric_forms_and_delimited_escapes_have_values_and_gates() {
    let (tokens, errors) = observe(
        "1'234 0xA'B 0b10'01 7wb 7uwb 8WBU 1'2.3'4e1'0\n",
        mode(CStandard::C23),
    );
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    for expected in [
        "= 1234",
        "= 171",
        "= 9",
        "= 7 (_BitInt(4))",
        "= 7 (unsigned _BitInt(3))",
        "= 8 (unsigned _BitInt(4))",
    ] {
        assert!(tokens.contains(expected), "{expected}: {tokens}");
    }
    let (_, errors) = observe("7wb 0b1\n", mode(CStandard::C17));
    assert_eq!(errors.len(), 2, "{errors:?}");
    let (tokens, errors) = observe("0o77 0O10 '\\x{41}' '\\o{101}'\n", mode(CStandard::C2y));
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    assert!(tokens.contains("= 63"));
    assert!(tokens.contains("= 8"));
    assert_eq!(tokens.matches("= 65 (int)").count(), 2);
    let (_, errors) = observe("0o77 '\\x{41}' '\\o{101}'\n", mode(CStandard::C23));
    assert_ne!(errors.len(), 0, "{errors:?}");
    for source in [
        "0b\n",
        "0o\n",
        "1'2'3f\n",
        "'\\x{}' sentinel\n",
        "'\\o{8}' sentinel\n",
        "u8'\\u00e9' sentinel\n",
    ] {
        let (_, errors) = observe(source, mode(CStandard::C2y));
        assert!(!errors.is_empty(), "{source}");
    }
}

#[test]
fn separator_nondigits_remain_one_preprocessing_number_for_diagnostics() {
    for spelling in ["1'e+2", "1'_", "0x1'e+1"] {
        let (tokens, errors) = observe(&format!("{spelling} after\n"), mode(CStandard::C23));
        assert_eq!(errors.len(), 1, "{spelling}: {errors:?}");
        let tokens = spellings(&tokens);
        assert!(tokens.contains(spelling), "{tokens}");
        assert!(tokens.contains("identifier `after`"), "{tokens}");
    }
}

#[test]
fn c89_pedantic_features_report_policy_without_losing_following_tokens() {
    let source = "#define F(a,...) a\nF(,)\n1LL 0x1p0\n// hidden\n$identifier\n";
    for policy in [
        ExtensionPolicy::Allow,
        ExtensionPolicy::Warn,
        ExtensionPolicy::Deny,
    ] {
        let (tokens, errors) = observe(
            source,
            CompilerConfiguration::new(CStandard::C89, policy).with_gnu_extensions(true),
        );
        assert!(spellings(&tokens).contains("identifier `$identifier`"));
        if policy == ExtensionPolicy::Allow {
            assert!(errors.is_empty(), "{errors:?}");
        } else {
            for feature in ["variadic macro", "long long", "hexadecimal", "//", "$"] {
                assert!(
                    errors.iter().any(|e| e.contains(feature)),
                    "{feature}: {errors:?}"
                );
            }
            assert!(
                errors
                    .iter()
                    .all(|e| e.starts_with(if policy == ExtensionPolicy::Warn {
                        "Warning:"
                    } else {
                        "Error:"
                    })),
                "{errors:?}"
            );
        }
    }
}

#[test]
fn conditional_directives_and_warning_are_gated_and_recover() {
    let source = "#define YES\n#if 0\nno\n#elifdef YES\nyes\n#elifndef \
                  NO\nno\n#else\nno\n#endif\n#if 0\n#elifndef NO\nyes2\n#endif\n#warning a \
                  message\nafter\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    let tokens = spellings(&tokens);
    assert!(tokens.contains("identifier `yes`"));
    assert!(tokens.contains("identifier `yes2`"));
    assert!(!tokens.contains("identifier `no`"));
    assert!(tokens.contains("identifier `after`"));
    assert_eq!(errors, ["Warning: #warning a message"]);
    let (_, errors) = observe("#warning message\n", mode(CStandard::C17));
    assert!(
        errors
            .iter()
            .any(|e| e.contains("unknown preprocessing directive")),
        "{errors:?}"
    );
}

#[test]
fn new_directives_and_invalid_optional_definitions_preserve_line_boundaries() {
    let source = "#define V(...) __VA_OPT__(##)\n#warning message\n#ident \"metadata\"\n#embed \
                  \"missing-lexpp-resource.bin\"\nint after;\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(spellings(&tokens).contains("identifier `after`"));
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors.iter().all(|e| !e.contains("stray")), "{errors:?}");
}

#[test]
fn va_opt_and_gnu_named_variadic_macros_expand_with_empty_and_nonempty_arguments() {
    let source = "#define EMPTY\n#define V(x,...) x __VA_OPT__(,) __VA_ARGS__\nV(a) V(b,) \
                  V(c,EMPTY) V(d,1,2)\n#define P(...) before ## __VA_OPT__(after)\nP() \
                  P(x)\n#define S(...) #__VA_OPT__(foo __VA_ARGS__)\nS() S(x)\n#define \
                  N(x,args...) x args\nN(named, one,two)\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    for word in [
        "a",
        "b",
        "c",
        "d",
        "before",
        "beforeafter",
        "named",
        "one",
        "two",
    ] {
        assert!(tokens.contains(&format!("identifier `{word}`")), "{tokens}");
    }
    assert_eq!(tokens.matches("punctuator `,`").count(), 3, "{tokens}");
    assert!(tokens.contains("foo x"), "{tokens}");
    let (_, errors) = observe("#define V(...) __VA_OPT__(x)\nV(1)\n", mode(CStandard::C17));
    assert_ne!(errors.len(), 0, "{errors:?}");
    for source in [
        "#define V(...) __VA_OPT__(__VA_OPT__(x))\nafter\n",
        "#define V(...) __VA_OPT__(## x)\nafter\n",
        "#define V(...) __VA_OPT__(x\nafter\n",
    ] {
        let (tokens, errors) = observe(source, mode(CStandard::C23));
        assert!(!errors.is_empty(), "{source}");
        assert!(spellings(&tokens).contains("identifier `after`"));
    }
}

#[test]
fn named_variadic_redefinitions_compare_original_names_and_bodies() {
    let source = "#define N(args...) args\n#define N(args...) args\nN(1)\n#define U(args...) \
                  2\n#define U(other...) 2\nU(0)\n#undef N\n#define N(other...) other\nN(3)\n";
    for gnu in [false, true] {
        let (tokens, errors) = observe(source, mode(CStandard::C23).with_gnu_extensions(gnu));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("redefined differently"), "{errors:?}");
        let tokens = spellings(&tokens);
        for value in ["= 1", "= 2", "= 3"] {
            assert!(tokens.contains(value), "{tokens}");
        }
    }
}

#[test]
fn named_variadic_parameters_preserve_their_spelling_and_argument_boundaries() {
    let source = "#define N(args...) args;\n#define O N\nO(1,2)\n#define S(args...) \
                  #args;\nS(3,4)\n#define P(args...) f(0, ## args);\nP() P(5,6)\n#define \
                  V(args...) __VA_OPT__(args);\nV() V(7,8)\n#define R(args...) __VA_ARGS__ \
                  args;\nR(9,10)\n";
    for standard in [
        CStandard::C89,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        let (tokens, errors) = observe(source, mode(standard).with_gnu_extensions(true));
        assert_eq!(errors.len(), 1, "{standard:?}: {errors:?}");
        assert!(errors[0].contains("__VA_ARGS__"), "{errors:?}");
        let tokens = spellings(&tokens);
        for value in 1..=10 {
            if value == 3 || value == 4 {
                continue;
            }
            assert!(tokens.contains(&format!("= {value} (int)")), "{tokens}");
        }
        assert!(tokens.contains("string literal \"3,4\""), "{tokens}");
        assert_eq!(
            tokens.matches("identifier `__VA_ARGS__`").count(),
            1,
            "{tokens}"
        );
        assert_eq!(tokens.matches("punctuator `,`").count(), 5, "{tokens}");
    }
}

#[test]
fn bit_precise_preprocessing_constants_use_intmax_magnitude_checks() {
    for standard in [CStandard::C23, CStandard::C2y] {
        for gnu in [false, true] {
            for (literal, warnings) in [
                ("9223372036854775807wb", 0),
                ("9223372036854775808wb", 1),
                ("18446744073709551615WB", 1),
                ("18446744073709551615uwb", 0),
                ("18446744073709551615WBU", 0),
                ("0xffffffffffffffffwb", 0),
            ] {
                let source = format!("#if {literal} > 0\npositive\n#else\nwrong\n#endif\nafter\n");
                let (tokens, errors) = observe(&source, mode(standard).with_gnu_extensions(gnu));
                let tokens = spellings(&tokens);
                assert!(
                    tokens.contains("identifier `positive`"),
                    "{literal}: {tokens}; {errors:?}"
                );
                assert!(
                    !tokens.contains("identifier `wrong`"),
                    "{literal}: {tokens}"
                );
                assert!(tokens.contains("identifier `after`"), "{literal}: {tokens}");
                assert_eq!(errors.len(), warnings, "{literal}: {errors:?}");
                assert!(
                    errors
                        .iter()
                        .all(|error| error.contains("so large that it is unsigned")),
                    "{literal}: {errors:?}"
                );
            }
        }
    }
    let (tokens, errors) = observe("9223372036854775808wb;\n", mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    assert!(spellings(&tokens).contains("_BitInt(65)"), "{tokens:?}");
}

#[test]
fn gnu_builtins_can_be_overridden_without_changing_protected_iso_macros() {
    for standard in [
        CStandard::C89,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for gnu in [false, true] {
            let source = "#define __COUNTER__ 77\n__COUNTER__;\n#undef __has_attribute\n#define \
                          __has_attribute(x) 78\n__has_attribute(unused);\n#undef \
                          __has_builtin\n#ifdef __has_builtin\nwrong\n#else\nundefined\n#endif\n";
            let (tokens, errors) = observe(source, mode(standard).with_gnu_extensions(gnu));
            assert_eq!(errors.len(), 3, "{errors:?}");
            assert!(
                errors.iter().all(|error| {
                    error.contains("redefining builtin") || error.contains("undefining builtin")
                }),
                "{errors:?}"
            );
            let tokens = spellings(&tokens);
            assert!(tokens.contains("= 77 (int)"), "{tokens}");
            assert!(tokens.contains("= 78 (int)"), "{tokens}");
            assert!(tokens.contains("identifier `undefined`"), "{tokens}");
            assert!(!tokens.contains("identifier `wrong`"), "{tokens}");
        }
    }
    for name in [
        "__STDC__",
        "__has_include",
        "__has_embed",
        "__has_c_attribute",
    ] {
        let source =
            format!("#undef {name}\n#define {name} 0\n#if defined({name})\nprotected\n#endif\n");
        let (tokens, errors) = observe(&source, mode(CStandard::C23));
        assert_eq!(errors.len(), 2, "{name}: {errors:?}");
        assert!(
            errors.iter().all(|error| error.contains("cannot")),
            "{errors:?}"
        );
        assert!(
            spellings(&tokens).contains("identifier `protected`"),
            "{tokens:?}"
        );
    }
}

#[test]
fn gnu_builtin_redefinitions_handle_both_macro_shapes_and_malformed_input() {
    for standard in [
        CStandard::C89,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for gnu in [false, true] {
            let config = mode(standard).with_gnu_extensions(gnu);
            for name in ["__COUNTER__", "__has_attribute", "__has_builtin"] {
                for (definition, invocation) in [(" 79", ""), ("(x) 79", "(unused)")] {
                    let source =
                        format!("#define {name}{definition}\n{name}{invocation};\nafter\n");
                    let (tokens, errors) = observe(&source, config);
                    assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                    assert!(errors[0].contains("redefining builtin"), "{errors:?}");
                    let tokens = spellings(&tokens);
                    assert!(tokens.contains("= 79 (int)"), "{source}: {tokens}");
                    assert!(tokens.contains("identifier `after`"), "{source}: {tokens}");
                }
                for definition in ["(x,", "(x x) 79", " ##", "(x) ## x", "(x) x ##"] {
                    let (invocation, value) = match name {
                        | "__COUNTER__" => ("", 0),
                        | "__has_attribute" => ("(unused)", 1),
                        | _ => ("(__builtin_offsetof)", 1),
                    };
                    let source = format!(
                        "#define {name}{definition}\n#if \
                         defined({name})\npreserved\n#endif\n{name}{invocation};\nafter\n"
                    );
                    let (tokens, errors) = observe(&source, config);
                    assert!(errors.len() >= 2, "{source}: {errors:?}");
                    let tokens = spellings(&tokens);
                    assert!(
                        tokens.contains("identifier `preserved`"),
                        "{source}: {tokens}"
                    );
                    assert!(tokens.contains("identifier `after`"), "{source}: {tokens}");
                    assert!(
                        tokens.contains(&format!("= {value} (int)")),
                        "{source}: {tokens}"
                    );
                }
            }
        }
    }
}

#[test]
fn misplaced_variadic_parameters_reject_the_definition_and_preserve_old_macro() {
    for config in [
        mode(CStandard::C23),
        mode(CStandard::C17).with_gnu_extensions(true),
    ] {
        for parameters in ["..., x", "..., ...", "x, ..., y"] {
            let source = format!("#define M 81\n#define M({parameters}) x\nM;\nafter\n");
            let (tokens, errors) = observe(&source, config);
            assert!(
                errors.iter().any(|error| error.contains("last parameter")),
                "{source}: {errors:?}"
            );
            let tokens = spellings(&tokens);
            assert!(tokens.contains("= 81 (int)"), "{source}: {tokens}");
            assert!(tokens.contains("identifier `after`"), "{source}: {tokens}");
        }
    }
}

#[test]
fn failed_queries_recover_as_zero_without_missing_operand_cascades() {
    for config in [
        mode(CStandard::C23),
        mode(CStandard::C17).with_gnu_extensions(true),
    ] {
        for query in [
            "__has_include(",
            "__has_embed(<missing",
            "__has_c_attribute(",
            "__has_include(identifier)",
            "__has_embed()",
        ] {
            for directive in ["#if", "#if 0\nwrong\n#elif"] {
                let source =
                    format!("{directive} {query}\nwrong\n#else\nrecovered\n#endif\nafter\n");
                let (tokens, errors) = observe(&source, config);
                assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                let tokens = spellings(&tokens);
                assert!(
                    tokens.contains("identifier `recovered`"),
                    "{source}: {tokens}"
                );
                assert!(tokens.contains("identifier `after`"), "{source}: {tokens}");
                assert!(!tokens.contains("identifier `wrong`"), "{source}: {tokens}");
            }
        }
    }
    let (tokens, errors) = observe(
        "__pragma(\nafter\n",
        mode(CStandard::C23).with_msvc_feature(MsvcFeature::Pragma, true),
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    let tokens = spellings(&tokens);
    assert!(tokens.contains("identifier `after`"), "{tokens}");
    assert!(!tokens.contains("integer constant"), "{tokens}");
}

#[test]
fn c89_integer_type_candidates_include_native_unsigned_long_decimals() {
    for standard in [CStandard::C89, CStandard::C95, CStandard::C99] {
        for gnu in [false, true] {
            for literal in ["9223372036854775808", "9223372036854775808L"] {
                let (tokens, errors) = observe(
                    &format!("{literal};\n"),
                    mode(standard).with_gnu_extensions(gnu),
                );
                let tokens = spellings(&tokens);
                if standard < CStandard::C99 {
                    assert!(errors.is_empty(), "{standard:?}: {errors:?}");
                    assert!(tokens.contains("(unsigned long)"), "{tokens}");
                } else {
                    assert_eq!(errors.len(), 1, "{errors:?}");
                    assert!(tokens.contains("(unsigned long long)"), "{tokens}");
                }
                let source = format!("#if {literal} > 0\npositive\n#else\nwrong\n#endif\n");
                let (tokens, errors) = observe(&source, mode(standard).with_gnu_extensions(gnu));
                assert!(
                    spellings(&tokens).contains("identifier `positive`"),
                    "{tokens:?}"
                );
                assert_eq!(
                    errors.len(),
                    usize::from(standard >= CStandard::C99),
                    "{errors:?}"
                );
            }
        }
    }
}

#[test]
fn optional_replacements_follow_the_standard_paste_and_stringification_examples() {
    let source = "#define H2(X,Y,...) __VA_OPT__(X ## Y,) __VA_ARGS__\nH2(a,b,c,d)\n#define \
                  H3(X,...) #__VA_OPT__(X##X X##X)\nH3(,0)\n#define H4(X,...) __VA_OPT__(a X ## \
                  X) ## b\nH4(,1)\n#define H5A(...) __VA_OPT__()/**/__VA_OPT__()\n#define H5B(X) \
                  a ## X ## b\n#define H5C(X) H5B(X)\nH5C(H5A())\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    assert_eq!(tokens.matches("identifier `ab`").count(), 2, "{tokens}");
    assert!(tokens.contains("string literal \"\""), "{tokens}");
    assert!(tokens.contains("identifier `a`"));
    assert!(tokens.contains("identifier `b`"));
}

#[test]
fn pasted_prefixes_and_digraphs_use_the_selected_mode() {
    let source = "#define P(x,y) x ## y\nP(u,\"a\") P(U,'b') P(u8,'c') P(<,:)\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    assert!(tokens.contains("u\"a\""));
    assert!(tokens.contains("98 (char32_t)"));
    assert!(tokens.contains("99 (char8_t)"));
    assert!(tokens.contains("punctuator `[`"));
    let (_, errors) = observe(source, mode(CStandard::C89));
    assert_eq!(errors.len(), 4, "{errors:?}");
}

#[test]
fn imaginary_suffix_orders_and_invalid_long_suffixes_do_not_recurse() {
    let (tokens, errors) = observe("2j 3Lj 4ULi 1.0fi 2.0if 3.0Li\n", mode(CStandard::C99));
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    for component in [
        "int _Complex",
        "long _Complex",
        "unsigned long _Complex",
        "float _Complex",
        "long double _Complex",
    ] {
        assert!(tokens.contains(component), "{tokens}");
    }
    let (tokens, errors) = observe(
        &format!("1{} after\n", "j".repeat(8_000)),
        mode(CStandard::C23),
    );
    assert_ne!(errors.len(), 0);
    assert!(spellings(&tokens).contains("identifier `after`"));
}

#[test]
fn lexical_and_operator_truncations_recover_in_every_revision() {
    let source = "#define V(...) __VA_OPT__(__VA_ARGS__)\nV(1)\nu8\"hello\" 1'234 0o7 \
                  '\\x{41}'\n#if __has_include(\"missing.h\")\n#endif\n__pragma(STDC FP_CONTRACT \
                  ON)\n";
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for end in 0..source.len() {
            drop(observe(
                &source[..end],
                mode(standard).with_msvc_feature(MsvcFeature::Pragma, true),
            ));
        }
    }
    let source = format!(
        "#if {}0{})\n#endif\nafter\n",
        "__has_attribute(".repeat(128),
        ")".repeat(127)
    );
    let (tokens, errors) = observe(&source, mode(CStandard::C23));
    assert_ne!(errors.len(), 0);
    assert!(spellings(&tokens).contains("identifier `after`"));
}

#[test]
fn dialect_comma_elision_distinguishes_omitted_and_empty() {
    let source = "#define G(x,...) f(x, ## __VA_ARGS__)\nG(a) G(b,) G(c,d)\n#define M(x,...) f(x, \
                  __VA_ARGS__)\nM(e) M(f,) M(g,h)\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(spellings(&tokens).matches("punctuator `,`").count(), 5);
    let (tokens, errors) = observe(
        source,
        mode(CStandard::C23).with_msvc_feature(MsvcFeature::VaArgs, true),
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(spellings(&tokens).matches("punctuator `,`").count(), 3);
    let source = "#define G(...) f(0, ## __VA_ARGS__)\nG() G(1)\n";
    for (gnu, commas) in [(false, 2), (true, 1)] {
        let (tokens, errors) = observe(source, mode(CStandard::C23).with_gnu_extensions(gnu));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(spellings(&tokens).matches("punctuator `,`").count(), commas);
    }
}

#[test]
fn pasted_stringification_retains_unicode_encoding() {
    let source = "#define P(a,b) a ## b\n#define S(a) #a\n#define E(a,b) P(a,b)\nE(u,S(x)); \
                  E(U,S(y)); E(u8,S(z));\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    for literal in ["u\"x\"", "U\"y\"", "u8\"z\""] {
        assert!(tokens.contains(literal), "{tokens}");
    }
}

#[test]
fn counter_queries_and_ms_pragma_are_per_translation_unit_and_opt_in() {
    let source = "__COUNTER__ __COUNTER__\n#if __has_builtin(__builtin_offsetof) && \
                  __has_attribute(unused) && \
                  __has_c_attribute(nodiscard)\nyes\n#endif\n__pragma(STDC FP_CONTRACT ON) after\n";
    let (tokens, errors) = observe(
        source,
        mode(CStandard::C23).with_msvc_feature(MsvcFeature::Pragma, true),
    );
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    assert!(tokens.contains("= 0"));
    assert!(tokens.contains("= 1"));
    assert!(tokens.contains("identifier `yes`"));
    assert!(!tokens.contains("__pragma"));
    assert!(tokens.contains("identifier `after`"));
    let (tokens, _) = observe("__pragma(ignored)\n", mode(CStandard::C23));
    assert!(spellings(&tokens).contains("identifier `__pragma`"));
}

#[test]
fn c23_queries_and_resource_directives_are_gated_in_earlier_strict_modes() {
    let source =
        "__has_include(\"missing.h\") __has_embed(\"missing.bin\") __has_c_attribute(nodiscard)\n";
    for (standard, gnu, enabled) in [
        (CStandard::C17, false, false),
        (CStandard::C17, true, true),
        (CStandard::C23, false, true),
    ] {
        let config = mode(standard).with_gnu_extensions(gnu);
        let source = if enabled {
            "#if !__has_include(\"missing.h\") && !__has_embed(\"missing.bin\") && \
             __has_c_attribute(nodiscard) == 202311\nfound\n#endif\n"
        } else {
            source
        };
        let (tokens, errors) = observe(source, config);
        assert!(errors.is_empty(), "{errors:?}");
        let tokens = spellings(&tokens);
        assert_eq!(
            tokens.contains("identifier `__has_include`"),
            !enabled,
            "{tokens}"
        );
        assert_eq!(tokens.contains("identifier `found`"), enabled, "{tokens}");
        let (_, errors) = observe("#embed \"missing.bin\"\nafter\n", config);
        assert!(
            errors.iter().any(|e| e.contains(if enabled {
                "cannot find embedded resource `missing.bin`"
            } else {
                "unknown preprocessing directive"
            })),
            "{errors:?}"
        );
    }
}

#[test]
fn conditional_queries_diagnose_invalid_context_and_preserve_following_tokens() {
    for name in ["__has_include", "__has_embed", "__has_c_attribute"] {
        let (tokens, errors) = observe(&format!("{name}(x) after\n"), mode(CStandard::C23));
        assert!(
            errors.iter().any(|e| e.contains("conditional expression")),
            "{errors:?}"
        );
        assert!(spellings(&tokens).contains("identifier `after`"));
    }
    let (tokens, errors) = observe("#if __has_include\n#endif\nafter\n", mode(CStandard::C23));
    assert!(
        errors.iter().any(|e| e.contains("expected '('")),
        "{errors:?}"
    );
    assert!(spellings(&tokens).contains("identifier `after`"));
}

#[test]
fn c23_allows_basic_and_control_universal_characters_inside_literals() {
    let source = "u'\\u0041' U'\\U00000001'\n";
    let (tokens, errors) = observe(source, mode(CStandard::C23));
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    assert!(tokens.contains("= 65"), "{tokens}");
    assert!(tokens.contains("= 1"), "{tokens}");
    let (_, errors) = observe(source, mode(CStandard::C11));
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn resource_queries_embed_parameters_and_include_next_use_real_search_paths() {
    struct Directory(PathBuf);
    impl Directory {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }
    let temp =
        Directory(std::env::temp_dir().join(format!("bcc-lexpp-resources-{}", std::process::id())));
    std::fs::create_dir_all(temp.path()).unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    std::fs::write(a.join("wrapper.h"), "first\n#include_next <wrapper.h>\n").unwrap();
    std::fs::write(b.join("wrapper.h"), "second\n").unwrap();
    std::fs::write(a.join("data.bin"), [0, 255, 65]).unwrap();
    std::fs::write(a.join("empty.bin"), []).unwrap();
    std::fs::write(a.join("literal-name.h"), "").unwrap();
    std::fs::write(a.join("%data%"), [77]).unwrap();
    std::fs::write(a.join("$header.h"), []).unwrap();
    std::fs::write(a.join("open("), []).unwrap();
    std::fs::write(temp.path().join("main.c"), []).unwrap();
    let source =
        "#define literal wrong\n#include <wrapper.h>\n#if __has_include(<wrapper.h>) && \
         !__has_include(<missing.h>)\nfound\n#endif\n#if __has_embed(<data.bin>) == \
         __STDC_EMBED_FOUND__ && __has_embed(<empty.bin>) == \
         __STDC_EMBED_EMPTY__\nresources\n#endif\n#embed <data.bin> limit(1+1) prefix(start,) \
         suffix(,end)\n#embed <empty.bin> if_empty(empty)\n#embed <data.bin> limit(0) \
         if_empty(zero)\n#if (5 + __has_embed(<data.bin> limit(1+1))) == 6\nnested\n#endif\n#if \
         __has_include(<literal-name.h>)\nliteral_header\n#endif\n#if __has_embed(<data.bin> \
         vendor::unsupported(1)) == __STDC_EMBED_NOT_FOUND__\nunsupported\n#endif\n#if \
         __has_embed(<%data%>) == __STDC_EMBED_FOUND__ && \
         __has_include(<$header.h>)\nspecial_header\n#endif\n#embed <%data%>\n#define RH \
         <%data%>\n#if __has_embed(RH) == __STDC_EMBED_FOUND__\nmacro_header\n#endif\n#define PH \
         <open(>\n#if __has_include(PH)\nparen_header\n#endif\n#if \
         __has_include(__FILE__)\ncurrent_file\n#endif\nafter\n";
    let (tokens, errors) = observe_paths(
        source,
        mode(CStandard::C23),
        temp.path().join("main.c"),
        &[a.clone(), b],
    );
    assert!(errors.is_empty(), "{errors:?}");
    let tokens = spellings(&tokens);
    for word in [
        "first",
        "second",
        "found",
        "resources",
        "start",
        "end",
        "empty",
        "zero",
        "after",
        "nested",
        "literal_header",
        "unsupported",
        "special_header",
        "macro_header",
        "paren_header",
        "current_file",
    ] {
        assert!(tokens.contains(&format!("identifier `{word}`")), "{tokens}");
    }
    assert!(tokens.contains("= 255"));
    assert!(tokens.contains("= 77"));
    assert!(!tokens.contains("= 65"));
    let (_, errors) = observe_paths(
        "#if __has_include(<$header.h>)\n#endif\n",
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Warn),
        temp.path().join("main.c"),
        &[a],
    );
    assert!(
        errors.is_empty(),
        "header characters are not identifiers: {errors:?}"
    );
    for source in [
        "#embed\nafter\n",
        "#embed \"unterminated\nafter\n",
        "#if __has_include(<missing>>)\n#endif\nafter\n",
    ] {
        let (tokens, errors) = observe(source, mode(CStandard::C23));
        assert!(!errors.is_empty(), "{source}");
        assert!(spellings(&tokens).contains("identifier `after`"));
    }
}

#[test]
fn mode_snapshot_preserves_token_values_spellings_and_extension_diagnostics() {
    let mut snapshot = String::new();
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for gnu in [false, true] {
            let config = CompilerConfiguration::new(standard, ExtensionPolicy::Warn)
                .with_gnu_extensions(gnu);
            let (tokens, errors) = observe(
                "u8\"x\" u'a' u8'a'\n0b11 0o7 8wb 1LL 0x1p0 2j\n// comment\n<: \
                 :>\n$foo\n__COUNTER__\n",
                config,
            );
            writeln!(snapshot, "=== {standard:?} GNU={gnu}").unwrap();
            for line in errors.iter().chain(&tokens) {
                snapshot.push_str(line);
                snapshot.push('\n');
            }
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lexing/language_modes_lexpp.snap");
    if std::env::var_os("BLESS").is_some_and(|v| v == "1") {
        std::fs::write(path, snapshot).unwrap();
        return;
    }
    pretty_assertions::assert_eq!(std::fs::read_to_string(path).unwrap(), snapshot);
}

#[test]
fn unterminated_feature_queries_preserve_directive_boundaries() {
    for config in [
        mode(CStandard::C23),
        mode(CStandard::C2y),
        mode(CStandard::C17).with_gnu_extensions(true),
    ] {
        for operand in [
            "__has_include(",
            "__has_include(<missing",
            "__has_include(\"missing\"",
            "__has_embed(",
            "__has_embed(<missing",
            "__has_c_attribute(",
            "__has_attribute(",
            "__has_builtin(",
        ] {
            for directive in ["#if", "#if 0\nskipped\n#elif"] {
                let source = format!("{directive} {operand}\n#endif\nint after;\n");
                let (tokens, errors) = observe(&source, config);
                assert!(!errors.is_empty(), "{source}");
                assert!(
                    spellings(&tokens).contains("identifier `after`"),
                    "lost following input: {source}: {tokens:?}; {errors:?}"
                );
                assert!(
                    !errors
                        .iter()
                        .any(|error| error.contains("unterminated `#if`")),
                    "lost #endif boundary: {source}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn conditional_boolean_identifiers_follow_revision_after_macro_expansion() {
    let source = "#define BOOL true\n#if true && BOOL && !false\nyes\n#else\nno\n#endif\n";
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for gnu in [false, true] {
            let (output, errors) = observe(source, mode(standard).with_gnu_extensions(gnu));
            let output = spellings(&output);
            let modern = standard >= CStandard::C23;
            assert_eq!(
                output.contains("identifier `yes`"),
                modern,
                "{standard:?}: {output}"
            );
            assert_eq!(
                output.contains("identifier `no`"),
                !modern,
                "{standard:?}: {output}"
            );
            if modern {
                assert!(errors.is_empty(), "{errors:?}");
            }
        }
    }
    for source in [
        "#define true 0\n#if true\nwrong\n#else\nright\n#endif\n",
        "#define true false\n#if true\nwrong\n#else\nright\n#endif\n",
    ] {
        let (output, errors) = observe(source, mode(CStandard::C23));
        assert!(errors.is_empty(), "{errors:?}");
        assert!(spellings(&output).contains("identifier `right`"));
    }
}

#[test]
fn va_opt_is_reserved_for_variadic_replacement_lists() {
    for config in [
        mode(CStandard::C23),
        mode(CStandard::C2y),
        mode(CStandard::C17).with_gnu_extensions(true),
    ] {
        for source in [
            "#define __VA_OPT__ 83\nint value = __VA_OPT__;\nint after;\n",
            "#define M(__VA_OPT__) __VA_OPT__\nint after;\n",
            "#define M(x) __VA_OPT__(x)\nint after;\n",
            "__VA_OPT__;\nint after;\n",
        ] {
            let (output, errors) = observe(source, config);
            assert!(
                errors.iter().any(|error| error.contains(
                    "`__VA_OPT__` can only appear in the replacement list of a variadic macro"
                )),
                "{source}: {errors:?}"
            );
            assert!(
                spellings(&output).contains("identifier `after`"),
                "{source}"
            );
        }
        let (output, errors) = observe(
            "#define M(...) __VA_OPT__(83)\nM() M(x)\nint after;\n",
            config,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(spellings(&output).matches("83 (int)").count(), 1);
    }
    let (_, errors) = observe(
        "#define M(args...) __VA_OPT__(args)\nM() M(83)\n",
        mode(CStandard::C23).with_gnu_extensions(true),
    );
    assert!(
        errors.is_empty(),
        "named variadic optional replacement: {errors:?}"
    );
}

#[test]
fn reserved_variadic_marker_diagnostics_follow_pedantic_policy() {
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for gnu in [false, true] {
            for policy in [
                ExtensionPolicy::Allow,
                ExtensionPolicy::Warn,
                ExtensionPolicy::Deny,
            ] {
                let config = CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu);
                for marker in ["__VA_ARGS__", "__VA_OPT__"] {
                    let source = format!("#define {marker} 83\nint after = {marker};\n");
                    let (tokens, errors) = observe(&source, config);
                    assert!(!errors.is_empty(), "{source}");
                    let expected = if policy == ExtensionPolicy::Deny {
                        "Error:"
                    } else {
                        "Warning:"
                    };
                    assert!(
                        errors.iter().all(|error| error.starts_with(expected)),
                        "{standard:?} GNU={gnu} {policy:?}: {errors:?}"
                    );
                    let tokens = spellings(&tokens);
                    assert!(tokens.contains("identifier `after`"), "{tokens}");
                    assert!(tokens.contains("= 83 (int)"), "{tokens}");
                }
            }
        }
    }
}
