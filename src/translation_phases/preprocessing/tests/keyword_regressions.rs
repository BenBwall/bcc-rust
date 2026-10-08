//! Keyword classification must happen after macro replacement.
use std::path::PathBuf;

use super::{
    Context,
    Preprocessor,
    SharedVec,
    TokenType,
};
use crate::translation_phases::preprocessing::KeywordTokenType;

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
