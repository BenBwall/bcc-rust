//! Keyword classification must happen after macro replacement.
use std::path::PathBuf;

use crate::{
    translation_phases::{
        Context,
        preprocessing::{
            KeywordTokenType,
            Preprocessor,
            token::TokenType,
        },
    },
    util::shared::SharedVec,
};

#[test]
fn every_c99_keyword_and_near_miss_is_classified_after_expansion() {
    let spellings = "auto break case char const continue default do double else enum extern float \
                     for goto if inline int long register restrict return short signed sizeof \
                     static struct switch typedef union unsigned void volatile while _Bool \
                     _Complex _Imaginary";
    let source = format!("{spellings} integer Int _bool while_ defined identifier\n");
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut pp = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<keywords>").into_boxed_path(),
        &source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut tokens = Vec::new();
    pp.for_each_iterator_item(&mut context, |_, token| tokens.push(token));
    let mut tokens = tokens.into_iter();
    for spelling in spellings.split_whitespace() {
        let token = tokens.next().unwrap();
        let TokenType::Keyword(keyword) = token.kind else {
            panic!("{spelling}: {token:?}")
        };
        assert_eq!(keyword.spelling(), spelling);
    }
    for spelling in ["integer", "Int", "_bool", "while_", "defined", "identifier"] {
        let token = tokens.next().unwrap();
        assert_eq!(token.kind, TokenType::Identifier);
        assert_eq!(context.string_cache.at(token.contents), spelling);
    }
    assert!(tokens.next().is_none());
    assert!(context.take_pending_errors().is_empty());
}

#[test]
fn keywords_remain_macro_names_and_paste_results_until_phase_seven() {
    let source =
        "#define int renamed\nint\n#undef int\n#define CAT(a,b) a##b\nCAT(in,t) CAT(wh,ile)\n";
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut pp = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<keywords>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut tokens = Vec::new();
    pp.for_each_iterator_item(&mut context, |_, token| tokens.push(token));
    let mut tokens = tokens.into_iter();
    for expected in [
        TokenType::Identifier,
        TokenType::Keyword(KeywordTokenType::Int),
        TokenType::Keyword(KeywordTokenType::While),
    ] {
        assert_eq!(tokens.next().unwrap().kind, expected);
    }
    assert!(tokens.next().is_none());
    assert!(context.take_pending_errors().is_empty());
}

#[test]
fn keyword_ids_are_a_stable_prefix_across_contexts_and_cache_growth() {
    for _ in 0..2 {
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        for index in 0..1000 {
            _ = context.string_cache.intern(format!("name_{index}"));
        }
        for &keyword in KeywordTokenType::ALL {
            assert_eq!(
                context.string_cache.intern(keyword.spelling()),
                keyword.cache_id()
            );
            assert_eq!(
                KeywordTokenType::from_cache_id(keyword.cache_id()),
                Some(keyword)
            );
            assert_eq!(
                context.string_cache.at(keyword.cache_id()),
                keyword.spelling()
            );
        }
        let ordinary = context.string_cache.intern("identifier");
        assert!(KeywordTokenType::from_cache_id(ordinary).is_none());
        assert!(
            KeywordTokenType::from_cache_id(crate::util::string_cache::StringCacheId::from_u32(
                u32::MAX
            ))
            .is_none()
        );
        let mut cloned = context.string_cache.clone();
        assert_eq!(cloned.intern("int"), KeywordTokenType::Int.cache_id());
    }
}

#[test]
fn every_keyword_spelling_respects_its_gate_and_survives_expansion() {
    use crate::configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    };
    let source = KeywordTokenType::ALL
        .iter()
        .map(|keyword| keyword.spelling())
        .chain(KeywordTokenType::ALIASES.iter().copied())
        // __pragma is consumed as a phase-4 operator when enabled; its flag has
        // dedicated lexpp tests rather than phase-7 keyword expectations.
        .filter(|spelling| *spelling != "__pragma")
        .collect::<Vec<_>>()
        .join(" ")
        + "\n";
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
            for msvc in [false, true] {
                let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                    .with_gnu_extensions(gnu)
                    .with_msvc_extensions(msvc);
                let tu = crate::util::bump::Bump::new();
                let mut context = Context::with_configuration(&tu, configuration);
                let pp_arena = crate::util::bump::Bump::new();
                let mut pp = Preprocessor::new(
                    &pp_arena,
                    &mut context,
                    PathBuf::from("<keywords>").into_boxed_path(),
                    &source,
                    SharedVec::default(),
                    SharedVec::default(),
                );
                let tokens = pp.preprocess_all(&mut context);
                assert_eq!(tokens.len(), source.split_whitespace().count());
                for token in tokens {
                    let expected = KeywordTokenType::classify(token.contents, configuration)
                        .map_or(TokenType::Identifier, |keyword| {
                            TokenType::Keyword(keyword.kind)
                        });
                    assert_eq!(token.kind, expected);
                }
                assert!(context.take_pending_errors().is_empty());
            }
        }
    }
}

#[test]
fn newly_introduced_non_reserved_words_remain_identifiers_in_old_modes() {
    use crate::configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    };
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let c89 = CompilerConfiguration::new(CStandard::C89, ExtensionPolicy::Allow);
    let c23 = CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow);
    for spelling in [
        "inline",
        "restrict",
        "bool",
        "true",
        "false",
        "nullptr",
        "constexpr",
        "static_assert",
        "thread_local",
        "alignas",
        "alignof",
        "typeof",
        "typeof_unqual",
        "asm",
    ] {
        let id = context.string_cache.intern(spelling);
        assert!(KeywordTokenType::classify(id, c89).is_none(), "{spelling}");
        if spelling != "asm" {
            assert!(KeywordTokenType::classify(id, c23).is_some(), "{spelling}");
        }
    }
    for spelling in [
        "_Bool",
        "_Static_assert",
        "_Generic",
        "_Alignas",
        "_Noreturn",
        "_Thread_local",
        "_Atomic",
        "__attribute__",
        "__typeof__",
        "__asm__",
        "__extension__",
        "_Countof",
    ] {
        let id = context.string_cache.intern(spelling);
        assert!(KeywordTokenType::classify(id, c89).is_some(), "{spelling}");
    }
}

#[test]
fn extension_policy_reports_original_alias_spelling_and_native_origin() {
    use crate::{
        configuration::{
            CStandard,
            CompilerConfiguration,
            ExtensionPolicy,
        },
        translation_phases::{
            ErrorSeverity,
            GetSeverity,
        },
    };
    for (policy, severity) in [
        (ExtensionPolicy::Allow, None),
        (ExtensionPolicy::Warn, Some(ErrorSeverity::Warning)),
        (ExtensionPolicy::Deny, Some(ErrorSeverity::Error)),
    ] {
        let configuration = CompilerConfiguration::new(CStandard::C89, policy);
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::with_configuration(&tu, configuration);
        let pp_arena = crate::util::bump::Bump::new();
        let mut pp = Preprocessor::new(
            &pp_arena,
            &mut context,
            PathBuf::from("<keywords>").into_boxed_path(),
            "_Static_assert __const__\n",
            SharedVec::default(),
            SharedVec::default(),
        );
        let tokens = pp.preprocess_all(&mut context);
        assert_eq!(tokens.len(), 2);
        let errors = context.take_pending_errors();
        if let Some(severity) = severity {
            assert_eq!(errors.len(), 2);
            assert_eq!(errors[0].to_string(), "'_Static_assert' is a C11 extension");
            assert_eq!(errors[1].to_string(), "'__const__' is a GNU extension");
            for error in errors {
                assert_eq!(error.severity(), severity);
            }
        } else {
            assert!(errors.is_empty());
        }
    }
}

/// Each alternate keyword spelling, written out independently of the
/// classifier's table: the keyword it spells, the modes where it is one, and
/// the origin its extension diagnostics name.
#[test]
fn every_alias_spelling_has_its_keyword_gate_and_origin() {
    use KeywordTokenType as K;

    use crate::configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
        FeatureOrigin,
        MsvcFeature,
    };
    #[derive(Clone, Copy, PartialEq)]
    enum Gate {
        C23,
        Always,
        Gnu,
        MsAsm,
    }
    let c23 = FeatureOrigin::Standard(CStandard::C23);
    let gnu = FeatureOrigin::Gnu;
    let expected = [
        ("bool", K::Bool, Gate::C23, c23),
        ("alignas", K::Alignas, Gate::C23, c23),
        ("alignof", K::Alignof, Gate::C23, c23),
        ("static_assert", K::StaticAssert, Gate::C23, c23),
        ("thread_local", K::ThreadLocal, Gate::C23, c23),
        ("__inline", K::Inline, Gate::Always, gnu),
        ("__inline__", K::Inline, Gate::Always, gnu),
        ("__restrict", K::Restrict, Gate::Always, gnu),
        ("__restrict__", K::Restrict, Gate::Always, gnu),
        ("__const", K::Const, Gate::Always, gnu),
        ("__const__", K::Const, Gate::Always, gnu),
        ("__volatile", K::Volatile, Gate::Always, gnu),
        ("__volatile__", K::Volatile, Gate::Always, gnu),
        ("__signed", K::Signed, Gate::Always, gnu),
        ("__signed__", K::Signed, Gate::Always, gnu),
        ("__alignof", K::Alignof, Gate::Always, gnu),
        ("__alignof__", K::Alignof, Gate::Always, gnu),
        ("__complex", K::Complex, Gate::Always, gnu),
        ("__complex__", K::Complex, Gate::Always, gnu),
        ("__real", K::Real, Gate::Always, gnu),
        ("__imag", K::Imag, Gate::Always, gnu),
        ("__typeof", K::Typeof, Gate::Always, gnu),
        ("__typeof__", K::Typeof, Gate::Always, gnu),
        ("__typeof_unqual", K::TypeofUnqual, Gate::Always, gnu),
        ("__typeof_unqual__", K::TypeofUnqual, Gate::Always, gnu),
        ("__attribute", K::Attribute, Gate::Always, gnu),
        ("asm", K::Asm, Gate::Gnu, gnu),
        (
            "_asm",
            K::MsAsm,
            Gate::MsAsm,
            FeatureOrigin::Msvc(MsvcFeature::Asm),
        ),
    ];
    assert_eq!(
        KeywordTokenType::ALIASES.len(),
        expected.len(),
        "every alias needs an expectation"
    );
    for (standard, gnu_mode, msvc) in [
        (CStandard::C17, false, false),
        (CStandard::C17, true, false),
        (CStandard::C23, false, false),
        (CStandard::C17, false, true),
    ] {
        let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
            .with_gnu_extensions(gnu_mode)
            .with_msvc_extensions(msvc);
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::with_configuration(&tu, configuration);
        for (spelling, kind, gate, origin) in expected {
            let id = context.string_cache.intern(spelling);
            let enabled = match gate {
                | Gate::C23 => standard >= CStandard::C23,
                | Gate::Always => true,
                | Gate::Gnu => gnu_mode,
                | Gate::MsAsm => msvc,
            };
            let classification = KeywordTokenType::classify(id, configuration);
            assert_eq!(classification.is_some(), enabled, "{spelling} {standard:?}");
            if let Some(classification) = classification {
                assert_eq!(classification.kind, kind, "{spelling}");
                assert_eq!(classification.origin, Some(origin), "{spelling}");
                assert_eq!(classification.spelling, spelling);
            }
        }
    }
}

#[test]
fn assembly_role_metadata_distinguishes_only_the_ambiguous_reserved_alias() {
    use crate::configuration::{
        CompilerConfiguration,
        MsvcFeature,
    };
    for gnu in [false, true] {
        for msvc in [false, true] {
            let tu = crate::util::bump::Bump::new();
            let config = CompilerConfiguration::default()
                .with_gnu_extensions(gnu)
                .with_msvc_feature(MsvcFeature::Asm, msvc);
            let mut context = Context::with_configuration(&tu, config);
            for spelling in ["__asm", "__asm__", "_asm", "asm", "ordinary"] {
                let id = context.string_cache.intern(spelling);
                let metadata = KeywordTokenType::classify(id, config);
                assert_eq!(
                    metadata.is_some_and(|m| m.ambiguous_asm),
                    spelling == "__asm",
                    "{spelling}"
                );
                if spelling == "__asm" {
                    assert_eq!(
                        metadata.unwrap().kind,
                        if msvc {
                            KeywordTokenType::MsAsm
                        } else {
                            KeywordTokenType::Asm
                        }
                    );
                }
            }
        }
    }
}
