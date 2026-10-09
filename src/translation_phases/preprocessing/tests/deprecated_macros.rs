//! Clang macro deprecation behavior through preprocessing and diagnostics.

use super::*;
use crate::diagnostics::{
    ColorChoice,
    Renderer,
    ToDiagnostic,
};

fn observe(
    source: &str,
    standard: CStandard,
    tokens_retained: bool,
) -> Vec<(u32, u32, u32, String)> {
    let tu = crate::util::bump::Bump::new();
    let pp = crate::util::bump::Bump::new();
    let mut context = Context::with_configuration(
        &tu,
        CompilerConfiguration::new(standard, ExtensionPolicy::Allow),
    );
    let mut preprocessor = Preprocessor::new(
        &pp,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    if tokens_retained {
        let _tokens = preprocessor.preprocess_all(&mut context);
    } else {
        preprocessor.for_each_iterator_item(&mut context, |_, _| {});
    }
    let mut warnings = Vec::new();
    for error in context.take_pending_errors() {
        if matches!(
            error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::DeprecatedMacro(..),
                ..
            })
        ) {
            let source = error.source_vectors(&mut context);
            let vector = &context.get_source_vectors(source)[0];
            warnings.push((
                vector.line,
                vector.column,
                vector.length,
                Renderer::new(ColorChoice::Plain)
                    .render(&error.to_diagnostic(&context, source), &context),
            ));
        } else {
            assert_ne!(error.severity(), ErrorSeverity::Error, "{error:?}");
        }
    }
    warnings
}

#[test]
fn deprecated_macro_warning_uses_outermost_invocation_and_expansion_notes() {
    for retained in [false, true] {
        let warnings = observe(
            "#define W(x) ATOMIC_VAR_INIT(x)\n#include <stdatomic.h>\natomic_int x = W(1);\n",
            CStandard::C17,
            retained,
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!((warnings[0].0, warnings[0].1, warnings[0].2), (3, 16, 1));
        assert!(warnings[0].3.contains("expanded from macro `W`"));

        let warnings = observe(
            "#define OLD 1\n#pragma clang deprecated(OLD)\n#define INNER OLD\n#define OUTER \
             INNER\nOUTER\n",
            CStandard::C17,
            retained,
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!((warnings[0].0, warnings[0].1, warnings[0].2), (5, 1, 5));
        assert!(warnings[0].3.contains("expanded from macro `OUTER`"));
        assert!(warnings[0].3.contains("expanded from macro `INNER`"));
        assert!(warnings[0].3.contains("macro marked deprecated here"));
        assert!(!warnings[0].3.contains("ATOMIC_VAR_INIT"));
    }
}

#[test]
fn deprecated_macro_in_a_system_replacement_warns_at_the_user_invocation() {
    let warnings = observe(
        "#include <stdatomic.h>\n#pragma clang \
         deprecated(__CLANG_ATOMIC_INT_LOCK_FREE)\nATOMIC_INT_LOCK_FREE\n",
        CStandard::C17,
        false,
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!((warnings[0].0, warnings[0].1, warnings[0].2), (3, 1, 20));
}

#[test]
fn deprecated_macro_uses_in_system_headers_remain_suppressed() {
    let directory = crate::test_support::TempDir::new("deprecated-system-header");
    std::fs::write(
        directory.path().join("wrapper.h"),
        "#pragma GCC system_header\n#define WRAP OLD\nOLD\n#ifdef OLD\n#endif\n#if \
         defined(OLD)\n#endif\n",
    )
    .unwrap();
    let tu = crate::util::bump::Bump::new();
    let pp = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let mut preprocessor = Preprocessor::new(
        &pp,
        &mut context,
        directory.path().join("use.c").into_boxed_path(),
        "#define OLD 1\n#pragma clang deprecated(OLD)\n#include \"wrapper.h\"\nWRAP\n",
        SharedVec::default(),
        SharedVec::default(),
    );
    preprocessor.for_each_iterator_item(&mut context, |_, _| {});
    let errors = context.take_pending_errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let source = errors[0].source_vectors(&mut context);
    let vector = &context.get_source_vectors(source)[0];
    assert_eq!((vector.line, vector.column, vector.length), (4, 1, 4));
}

#[test]
fn deprecated_macro_arguments_and_cross_frame_invocations_use_clang_locations() {
    let warnings = observe(
        "#define OLD 1\n#pragma clang deprecated(OLD)\n#define ID(x) x\nID(OLD)\n#define USE(x) \
         OLD + x\nID(USE(1))\n#define CALL USE\nCALL(2)\n",
        CStandard::C17,
        false,
    );
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert_eq!((warnings[0].0, warnings[0].1), (4, 4));
    assert_eq!((warnings[1].0, warnings[1].1), (6, 4));
    assert_eq!((warnings[2].0, warnings[2].1), (8, 1));
    assert!(!warnings[0].3.contains("expanded from macro `ID`"));
    assert!(warnings[2].3.contains("expanded from macro `CALL`"));
}

#[test]
fn deprecated_macro_mark_survives_redefinitions_until_undef() {
    for retained in [false, true] {
        let warnings = observe(
            "#include <stdatomic.h>\n#define ATOMIC_VAR_INIT(value) (value)\natomic_int x = \
             ATOMIC_VAR_INIT(1);\n",
            CStandard::C17,
            retained,
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let warnings = observe(
            "#define OLD 1\n#pragma clang deprecated(OLD)\nOLD\n#define OLD 1\nOLD\n#define OLD \
             2\nOLD\n#undef OLD\n#define OLD 1\nOLD\n",
            CStandard::C17,
            retained,
        );
        assert_eq!(warnings.iter().map(|w| w.0).collect::<Vec<_>>(), [3, 5, 7]);
        let warnings = observe(
            "#define OLD(x) (x)\n#pragma clang deprecated(OLD)\nOLD(1)\n#define OLD(x) \
             (x)\nOLD(2)\nOLD\n",
            CStandard::C17,
            retained,
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }
}

#[test]
fn deprecated_macro_definition_tests_warn_only_in_evaluated_groups() {
    let source = concat!(
        "#define OLD 1\n",
        "#pragma clang deprecated(OLD)\n",
        "#ifdef OLD\n#endif\n",
        "#ifndef OLD\n#endif\n",
        "#if defined(OLD)\n#endif\n",
        "#if defined OLD\n#endif\n",
        "#if 0\n#elifdef OLD\n#endif\n",
        "#if 0\n#elifndef OLD\n#endif\n",
        "#if 1 || defined(OLD)\n#endif\n",
        "#if 1\n#elifdef OLD\n#endif\n",
        "#if 0\n#ifdef OLD\n#endif\n#if defined(OLD)\n#endif\n#endif\n",
    );
    for retained in [false, true] {
        let warnings = observe(source, CStandard::C23, retained);
        let locations: Vec<_> = warnings.iter().map(|w| (w.0, w.1)).collect();
        assert_eq!(
            locations,
            [
                (3, 8),
                (5, 9),
                (7, 13),
                (9, 13),
                (12, 10),
                (15, 11),
                (17, 18)
            ]
        );
    }
}

#[test]
fn deprecated_macro_optional_messages_follow_clang_string_literal_rules() {
    for retained in [false, true] {
        let warnings = observe(
            "#define OLD 1\n#pragma clang deprecated(OLD, \"use \" \
             \"NEW\\n\\\"quoted\\\"\")\nOLD\n#pragma clang deprecated(OLD, \
             \"\")\nOLD\n_Pragma(\"clang deprecated(OLD, \\\"operator\\\")\")\nOLD\n#pragma clang \
             deprecated(OLD, \"old\\0new\")\nOLD\n",
            CStandard::C17,
            retained,
        );
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings[0].3.contains("deprecated: use NEW\n\"quoted\""));
        assert!(!warnings[1].3.contains("deprecated:"));
        assert!(warnings[2].3.contains("deprecated: operator"));
        assert!(warnings[3].3.contains("old<U+0000>new"));
        assert!(warnings.iter().all(|w| !w.3.contains('\0')));
    }
}

#[test]
fn deprecated_macro_malformed_messages_preserve_following_input() {
    for directive in [
        "#pragma clang deprecated(OLD, L\"wide\")",
        "#pragma clang deprecated(OLD, u8\"utf8\")",
        "#pragma clang deprecated(OLD, \"ok\" L\"wide\")",
        "#pragma clang deprecated(OLD, 2)",
        "#pragma clang deprecated(OLD,)",
        "#pragma clang deprecated(OLD, \"message\" junk)",
        "#pragma clang deprecated(OLD, \"message\"",
        "#pragma clang deprecated OLD",
        "#pragma clang deprecated()",
        "#pragma clang deprecated(MISSING)",
    ] {
        let source = format!("#define OLD 1\n{directive}\nafter\n");
        preprocess_with_configuration(
            &source,
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow),
            |identifiers, errors| {
                assert_eq!(identifiers, ["after"], "{directive}");
                assert!(
                    matches!(
                        errors,
                        [TranslationError::Preprocessing(PreprocessorError {
                            error_type: PreprocessorErrorType::LanguageConstraint(_),
                            ..
                        })]
                    ),
                    "{directive}: {errors:?}"
                );
            },
        );
    }
}
