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
