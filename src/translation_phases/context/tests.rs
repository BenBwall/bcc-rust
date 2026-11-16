//! Context storage and provenance fixtures.
//!
//! C99: phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! Diagnostic locations: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.

use super::{
    ArgumentSubstitution,
    Context,
    segmented_vec::SegmentedVec,
};
use crate::translation_phases::provenance::{
    SourceArena,
    SourceVector,
};

#[test]
fn source_helpers_preserve_order_and_copies_survive_compaction() {
    let tu = crate::util::bump::Bump::new();
    let phase = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let position = super::SourcePosition {
        index:  7,
        line:   2,
        column: 3,
    };
    let source = context.create_source_vectors(position, 4, 2);
    let other = context.create_source_vectors(
        super::SourcePosition {
            index:  12,
            line:   3,
            column: 1,
        },
        5,
        1,
    );
    let source = context.merge_vectors(source, other);
    let expected = context.get_source_vectors(source).to_vec();
    let parser = context.retain_token_source(source);
    let retained = context.create_retained_source_vectors(position, 4, 2);
    for range in [source, parser, retained] {
        assert_eq!(context.first_source_vector_or_default(range), expected[0]);
        assert_eq!(context.diagnostic_position(range), position);
    }
    let copy = context.copy_source_vectors_in(source, &phase);
    let empty = super::SourceVectors::empty();
    assert_eq!(
        context.first_source_vector_or_default(empty),
        SourceVector::default()
    );
    assert_eq!(context.copy_source_vectors_in(empty, &phase), []);
    context.compact_preprocessor_vectors();
    assert_eq!(copy, expected);
    assert_eq!(context.get_source_vectors(parser), expected);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "diagnostic primary source range must be non-empty")]
fn diagnostic_positions_reject_empty_ranges_even_with_vectors_in_the_store() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    _ = context.create_source_vectors(super::SourcePosition::default(), 0, 1);
    _ = context.diagnostic_position(super::SourceVectors::empty());
}

#[test]
fn retained_end_anchors_keep_the_file_line_and_end_column() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    for length in [0, 4] {
        let vector = SourceVector {
            index: 10,
            column: 6,
            line: 3,
            source_file_index: 2,
            length,
        };
        let source = context.retain_source_end(&vector);
        assert_eq!(source.length(), 1);
        assert_eq!(
            context.get_source_vectors(source),
            &[SourceVector {
                index: 10 + length,
                column: 6 + length,
                length: 0,
                ..vector
            }]
        );
        assert_eq!(
            context.diagnostic_position(source),
            super::SourcePosition {
                index:  (10 + length) as usize,
                line:   3,
                column: 6 + length,
            }
        );
        context.compact_preprocessor_vectors();
        assert_eq!(context.get_source_vectors(source)[0].length, 0);
    }
}

#[test]
fn source_arena_accepts_the_last_representable_vector() {
    assert_eq!(
        Context::checked_source_append(
            SourceArena::Preprocessor,
            u32::MAX as usize - 1,
            u32::MAX as usize - 1,
            1,
        ),
        (u32::MAX - 1, u32::MAX)
    );
}

#[test]
#[should_panic(expected = "Preprocessor source arena exceeds u32::MAX vectors")]
fn source_arena_rejects_append_after_u32_max_vectors() {
    _ = Context::checked_source_append(
        SourceArena::Preprocessor,
        u32::MAX as usize,
        u32::MAX as usize,
        1,
    );
}

#[test]
#[should_panic(expected = "Retained source arena length overflows usize")]
fn source_arena_rejects_usize_overflow() {
    _ = Context::checked_source_append(SourceArena::Retained, 0, usize::MAX, 1);
}

#[test]
#[should_panic(expected = "Retained source range exceeds the 30-bit length limit")]
fn source_arena_rejects_a_range_longer_than_thirty_bits() {
    _ = Context::checked_source_append(
        SourceArena::Retained,
        0,
        crate::translation_phases::provenance::SourceVectors::MAX_LENGTH as usize,
        1,
    );
}

#[test]
#[should_panic(expected = "ParserTokens source range cannot start at index u32::MAX")]
fn source_arena_rejects_a_range_starting_at_u32_max() {
    _ = Context::checked_source_append(
        SourceArena::ParserTokens,
        u32::MAX as usize,
        u32::MAX as usize,
        0,
    );
}

#[test]
fn source_line_indices_are_lazy_reused_and_replaced_with_the_text() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(std::path::Path::new("example.c"));
    assert_eq!(context.source_line_starts(file), None);
    context.record_arena_source_text(file, "a\r\nb\rc\n");
    assert!(
        context.source_texts[file as usize]
            .as_ref()
            .unwrap()
            .line_starts
            .get()
            .is_none()
    );
    let starts = context.source_line_starts(file).unwrap();
    assert_eq!(starts, &[0, 3, 5, 7]);
    let used = tu.used();
    for _ in 0..1_000 {
        assert!(std::ptr::eq(
            context.source_line_starts(file).unwrap(),
            starts
        ));
    }
    assert_eq!(tu.used(), used);

    context.record_arena_source_text(file, "x\ny");
    assert_eq!(context.source_line_starts(file), Some(&[0, 2][..]));
    // A previously borrowed index still lives as long as the TU arena.
    assert_eq!(starts, &[0, 3, 5, 7]);
    context.record_arena_source_text(file, "");
    assert_eq!(context.source_line_starts(file), Some(&[0][..]));
}

#[test]
fn segmented_vectors_keep_order_across_segments_and_reuse_them() {
    let arena = crate::util::bump::Bump::new();
    let mut values = SegmentedVec::new_in(&arena);
    // 100 values span the first three segments (16, 32, and 64).
    for value in (0..200).step_by(2) {
        values.push(value);
    }
    values.insert(50, 99);
    let mut expected: Vec<i32> = (0..100).step_by(2).collect();
    expected.push(99);
    expected.extend((100..200).step_by(2));
    assert_eq!(values.iter().copied().collect::<Vec<_>>(), expected);
    assert_eq!(values.partition_point(|&value| value < 100), 51);
    assert_eq!(values.last(), Some(&198));

    values.discard_front(10);
    assert_eq!((values.len(), values[0]), (91, 20));
    assert_eq!(values.partition_point(|&value| value < 100), 41);
    assert_eq!(values.iter().count(), 91);
    values.for_each_mut(|value| *value += 1);
    assert_eq!(values[0], 21);

    // Clearing keeps the segments, so refilling takes no arena memory.
    let used = arena.used();
    values.clear();
    assert!(values.is_empty());
    for value in 0..100 {
        values.push(value);
    }
    assert_eq!(arena.used(), used);
    assert_eq!(
        values.iter().copied().collect::<Vec<_>>(),
        (0..100).collect::<Vec<_>>()
    );
}

#[test]
fn repeated_literal_values_keep_one_identity_after_arena_growth() {
    use crate::translation_phases::preprocessing::LiteralUnit;

    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let original = [LiteralUnit::Character('é'), LiteralUnit::Numeric(0)];
    let id = context.intern_literal(&original);
    for value in 0..2_000 {
        _ = context.intern_literal(&[LiteralUnit::Numeric(value)]);
    }
    assert_eq!(context.intern_literal(&original), id);
    assert_eq!(context.literal_units(id), original);
}

#[test]
fn source_paths_keep_indices_and_synthetic_inputs_keep_distinct_text() {
    use std::path::PathBuf;

    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let path = PathBuf::from("included/header.h");
    let first = context.intern_source_file(&path);
    for index in 0..2_000 {
        _ = context.intern_source_file(&PathBuf::from(format!("included/{index}.h")));
    }
    assert_eq!(context.intern_source_file(&path), first);
    assert_eq!(context.get_source_file(first), path);
    let synthetic = context.add_synthetic_source_file(&path, "second input");
    assert_ne!(synthetic, first);
    assert_eq!(context.source_text(synthetic), Some("second input"));
}

#[cfg(windows)]
#[test]
fn source_paths_preserve_unpaired_utf16_surrogates() {
    use std::{
        ffi::OsString,
        os::windows::ffi::OsStringExt,
        path::PathBuf,
    };

    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let path = PathBuf::from(OsString::from_wide(&[u16::from(b'x'), 0xD800]));
    let id = context.intern_source_file(&path);
    assert_eq!(context.get_source_file(id), path);
    assert_eq!(context.intern_source_file(&path), id);
}

#[test]
fn macro_locations_survive_compaction_without_retaining_temporary_metadata() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let mut saved = Vec::new();
    for index in 0..2_000 {
        let source = context.push_source_vectors(&[SourceVector {
            length: 1,
            ..SourceVector::default()
        }]);
        let site = SourceVector {
            index,
            line: index + 1,
            length: 1,
            ..SourceVector::default()
        };
        context.record_expansion_end(source, site.clone());
        let token = context.retain_token_source(source);
        let retained = context.retain_preprocessor_range(source);
        saved.push((token, retained, site));
        context.compact_preprocessor_vectors();
        let temporary = &context.expansion_sites[SourceArena::Preprocessor as usize];
        assert!(temporary.entries.is_empty());
        assert!(temporary.ends.is_empty());
    }
    for (token, retained, site) in saved {
        assert_eq!(context.user_source_end(token), Some(site.clone()));
        assert_eq!(context.user_source_end(retained), Some(site));
    }
}

#[test]
fn diagnostic_pragmas_govern_retained_copies_of_tokens_by_output_order() {
    use crate::{
        configuration::{
            DiagnosticGroup,
            PragmaClass,
        },
        translation_phases::{
            ErrorSeverity,
            SourceVectors,
        },
    };
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let group = DiagnosticGroup::InitializerOverrides;
    let spelled = SourceVector {
        length: 1,
        ..SourceVector::default()
    };
    let token = |context: &mut Context<'_>| {
        let source = context.push_source_vectors(std::slice::from_ref(&spelled));
        context.retain_token_source(source)
    };
    let before = token(&mut context);
    context.pragma_diagnostic_class("initializer-overrides", PragmaClass::Ignored, false);
    let quiet = token(&mut context);
    let next = token(&mut context);
    context.pragma_diagnostic_class("initializer-overrides", PragmaClass::Error, false);
    let promoted = token(&mut context);
    let severity =
        |context: &Context<'_>, source| context.configuration_at(source).diagnostic_severity(group);
    // Directly, by the token's place in the output.
    assert_eq!(severity(&context, before), Some(ErrorSeverity::Warning));
    assert_eq!(severity(&context, quiet), None);
    assert_eq!(severity(&context, promoted), Some(ErrorSeverity::Error));
    // A merged range that had to be copied reads its first token's place.
    let merged = context.merge_vectors(quiet, promoted);
    assert_eq!(severity(&context, merged), None);
    let reversed = context.merge_vectors(promoted, before);
    assert_eq!(severity(&context, reversed), Some(ErrorSeverity::Error));
    // And so does a copy of a copy.
    let again = context.merge_vectors(reversed, next);
    assert_eq!(severity(&context, again), Some(ErrorSeverity::Error));
    let early = context.merge_vectors(before, merged);
    assert_eq!(severity(&context, early), Some(ErrorSeverity::Warning));
    // A location with no output token yet reads the latest change.
    assert_eq!(
        severity(&context, SourceVectors::default()),
        Some(ErrorSeverity::Error)
    );
}

/// A zero-width location for syntax the input left out copies no provenance,
/// so without its own note it would read the settings of whatever retained
/// copy came before it, or the latest change.
#[test]
fn diagnostic_pragmas_govern_anchors_for_missing_syntax() {
    use crate::{
        configuration::{
            DiagnosticGroup,
            PragmaClass,
        },
        translation_phases::{
            ErrorSeverity,
            provenance::SourcePosition,
        },
    };
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let group = DiagnosticGroup::InitializerOverrides;
    let spelled = SourceVector {
        length: 1,
        ..SourceVector::default()
    };
    let token = |context: &mut Context<'_>| {
        let source = context.push_source_vectors(std::slice::from_ref(&spelled));
        context.retain_token_source(source)
    };
    let before = token(&mut context);
    context.pragma_diagnostic_class("initializer-overrides", PragmaClass::Ignored, false);
    let quiet = token(&mut context);
    context.pragma_diagnostic_class("initializer-overrides", PragmaClass::Error, false);
    let promoted = token(&mut context);
    let copy = context.merge_vectors(quiet, promoted);
    let anchor = |context: &mut Context<'_>, token| {
        let anchor = context.create_retained_source_vectors(SourcePosition::default(), 0, 0);
        context.record_retained_pragma_anchor(anchor, token);
        anchor
    };
    let severity =
        |context: &Context<'_>, source| context.configuration_at(source).diagnostic_severity(group);
    assert_eq!(severity(&context, copy), None);
    let at_before = anchor(&mut context, Some(before));
    assert_eq!(severity(&context, at_before), Some(ErrorSeverity::Warning));
    let at_quiet = anchor(&mut context, Some(quiet));
    assert_eq!(severity(&context, at_quiet), None);
    let at_end = anchor(&mut context, None);
    assert_eq!(severity(&context, at_end), Some(ErrorSeverity::Error));
    // Later anchors leave the earlier ones where they were.
    assert_eq!(severity(&context, at_before), Some(ErrorSeverity::Warning));
}

#[test]
fn expansion_locations_outlive_compaction_and_follow_copies() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let spelled = SourceVector {
        length: 1,
        ..SourceVector::default()
    };
    let site = SourceVector {
        index: 40,
        line: 3,
        length: 7,
        ..SourceVector::default()
    };
    let expanded = context.push_source_vectors(std::slice::from_ref(&spelled));
    context.record_expansion_start(expanded, site.clone(), site.clone(), None);
    let plain = context.push_source_vectors(std::slice::from_ref(&spelled));
    let token = context.retain_token_source(expanded);
    let plain_token = context.retain_token_source(plain);
    context.compact_preprocessor_vectors();
    // A later parser merge copies both tokens into the retained arena.
    let merged = context.merge_vectors(plain_token, token);
    let merged_expanded = context.merge_vectors(token, plain_token);
    assert_eq!(context.expansion_location(token), &site);
    assert_eq!(context.expansion_location(plain_token), &spelled);
    assert_eq!(context.expansion_location(merged), &spelled);
    assert_eq!(context.expansion_location(merged_expanded), &site);
    let shown = context.file_location(token);
    assert_eq!(context.get_source_vectors(shown)[0].index, site.index);
    assert_eq!(context.file_location(plain_token), plain_token);
}

/// An argument's token belongs to its invocation's expansion location, but
/// keeps its own file location, as Clang's line tables give it, when the
/// invocation spells it; a definition's token takes the invocation's.
#[test]
fn file_vectors_keep_arguments_the_invocation_spells() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let site = SourceVector {
        index: 40,
        line: 3,
        length: 4,
        ..SourceVector::default()
    };
    let in_definition = SourceVector {
        length: 1,
        ..SourceVector::default()
    };
    let argument = SourceVector {
        index: 52,
        line: 4,
        column: 9,
        length: 1,
        ..SourceVector::default()
    };
    let replaced = context.push_source_vectors(std::slice::from_ref(&in_definition));
    context.record_expansion_start(replaced, site.clone(), site.clone(), None);
    let written = context.push_source_vectors(std::slice::from_ref(&argument));
    context.record_expansion_start(written, site.clone(), site.clone(), None);
    let replaced = context.retain_token_source(replaced);
    let written = context.retain_token_source(written);
    context.compact_preprocessor_vectors();
    assert_eq!(context.expansion_location(replaced), &site);
    assert_eq!(context.expansion_location(written), &site);
    assert_eq!(context.file_vector(replaced), &site);
    assert_eq!(context.file_vector(written), &argument);
    assert_eq!(context.file_location(written), written);
}

/// Clang underlines a range whose end a macro's list wrote out to that end
/// of the whole invocation, `Min(a, b)`, and leaves an argument the
/// invocation spells, and the file's tokens, where they are. The ends follow
/// a range copied by a merge.
#[test]
fn highlights_cover_the_invocations_their_ends_come_from() {
    let tu = crate::util::bump::Bump::new();
    let scratch = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let at = |index: u32, length: u32| SourceVector {
        index,
        line: 3,
        length,
        ..SourceVector::default()
    };
    // `#define Min(x, y) ...` on line 1; `b < Min(a, c) + 1` on line 3.
    let (before, site, argument, end, after) =
        (at(30, 1), at(40, 3), at(44, 1), at(49, 1), at(53, 1));
    let in_definition = SourceVector {
        length: 1,
        ..SourceVector::default()
    };
    let plain_before = context.push_source_vectors(std::slice::from_ref(&before));
    let body = context.push_source_vectors(std::slice::from_ref(&in_definition));
    context.record_expansion_start(body, site.clone(), end.clone(), None);
    let written = context.push_source_vectors(std::slice::from_ref(&argument));
    let substitution = Some(ArgumentSubstitution::FIRST);
    context.record_expansion_start(written, site.clone(), end.clone(), substitution);
    let plain = context.push_source_vectors(std::slice::from_ref(&after));
    let [plain_before, body, written, plain] =
        [plain_before, body, written, plain].map(|source| context.retain_token_source(source));
    context.compact_preprocessor_vectors();
    assert_eq!(
        context.highlight_vectors(body, &scratch),
        [site.clone(), end.clone()]
    );
    assert_eq!(
        context.highlight_vectors(written, &scratch),
        std::slice::from_ref(&argument)
    );
    assert_eq!(
        context.highlight_vectors(plain, &scratch),
        std::slice::from_ref(&after)
    );
    let operand = context.merge_vector_list(&[body, written, plain]);
    assert_eq!(
        context.highlight_vectors(operand, &scratch),
        [site, argument, after]
    );
    // Not adjacent, so copied: its last token keeps its invocation.
    let copied = context.merge_vectors(plain, body);
    let copied = context.merge_vectors(plain_before, Context::last_source_vector(copied));
    assert_eq!(context.highlight_vectors(copied, &scratch), [before, end]);
}

/// A concatenated string literal is one phase-6 token over several vectors.
/// Each keeps the invocation of the token it came from once the token is
/// retained, so a format specification a macro wrote in the middle of
/// `"%s " FMT " %s"` is drawn as `FMT`.
#[test]
fn retained_tokens_keep_the_invocation_of_every_vector() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let at = |index: u32, length: u32| SourceVector {
        index,
        line: 3,
        length,
        ..SourceVector::default()
    };
    // `#define FMT "%d"` on line 1; `"%s " FMT " %s"` on line 3.
    let (first, site, last) = (at(30, 5), at(36, 3), at(40, 5));
    let in_definition = SourceVector {
        index: 12,
        length: 4,
        ..SourceVector::default()
    };
    let first = context.push_source_vectors(std::slice::from_ref(&first));
    let written = context.push_source_vectors(std::slice::from_ref(&in_definition));
    context.record_expansion_start(written, site.clone(), site.clone(), None);
    let last = context.push_source_vectors(std::slice::from_ref(&last));
    let literal = context.merge_vector_list(&[first, written, last]);
    let literal = context.retain_token_source(literal);
    context.compact_preprocessor_vectors();
    let span = |context: &Context<'_>, index| {
        context
            .invocation_span(Context::source_vector_at(literal, index))
            .map(|(start, end)| (start.clone(), end.clone()))
    };
    assert_eq!(span(&context, 0), None);
    assert_eq!(span(&context, 1), Some((site.clone(), site)));
    assert_eq!(span(&context, 2), None);
    assert_eq!(Context::source_vector_at(literal, 3).length(), 0);
}

/// Two tokens one invocation produced from its arguments are in one
/// argument expansion only when one substitution produced both: Clang
/// leaves a range within `ID("%" "d")` where the invocation spells it, and
/// draws one over `JOIN(" ", "d")` (`a b`) as the whole invocation. The
/// substitutions survive retention and compaction.
#[test]
fn highlights_cover_the_invocation_for_two_argument_expansions() {
    let tu = crate::util::bump::Bump::new();
    let scratch = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let at = |index: u32, length: u32| SourceVector {
        index,
        line: 3,
        length,
        ..SourceVector::default()
    };
    // `JOIN("%", "d")` on line 3, from its name to its `)`.
    let (site, first, second, end) = (at(40, 4), at(45, 3), at(50, 3), at(53, 1));
    let one = ArgumentSubstitution::FIRST;
    let other = one.next();
    let mut record = |vector: &SourceVector, argument| {
        let source = context.push_source_vectors(std::slice::from_ref(vector));
        context.record_expansion_start(source, site.clone(), end.clone(), Some(argument));
        source
    };
    let tokens = [
        record(&first, one),
        record(&second, other),
        record(&second, one),
    ];
    let [first_token, split, same] = tokens.map(|source| context.retain_token_source(source));
    context.compact_preprocessor_vectors();
    assert_eq!(context.argument_substitution(first_token), Some(one));
    assert_eq!(context.argument_substitution(split), Some(other));
    assert_eq!(context.argument_substitution(same), Some(one));
    // The arguments between the invocation's ends stay, drawn within it.
    let split = context.merge_vectors(first_token, split);
    assert_eq!(
        context.highlight_vectors(split, &scratch),
        [site, first.clone(), second.clone(), end]
    );
    let same = context.merge_vectors(first_token, same);
    assert_eq!(context.highlight_vectors(same, &scratch), [first, second]);
}

/// A token `##` formed from two tokens comes from no argument, although the
/// invocation spells its operands: Clang places it at the `##` expression
/// of the list, so `CAT(u8, "%d")` is underlined whole. Each of its vectors
/// keeps the invocation, so a range that starts within it, at `%d`, finds
/// the invocation too.
#[test]
fn highlights_draw_a_pasted_token_as_its_invocation() {
    let tu = crate::util::bump::Bump::new();
    let scratch = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let at = |index: u32, length: u32| SourceVector {
        index,
        line: 3,
        length,
        ..SourceVector::default()
    };
    // `CAT(u8, "%d")` on line 3, from its name to its `)`.
    let (site, prefix, string, end) = (at(40, 3), at(44, 2), at(48, 4), at(52, 1));
    let pasted = context.push_source_vectors(&[prefix.clone(), string.clone()]);
    context.record_expansion_start(pasted, site.clone(), end.clone(), None);
    let pasted = context.retain_token_source(pasted);
    context.compact_preprocessor_vectors();
    let inner = Context::source_vector_at(pasted, 1);
    assert_eq!(
        context
            .invocation_span(inner)
            .map(|(start, end)| (start.clone(), end.clone())),
        Some((site.clone(), end.clone()))
    );
    assert_eq!(context.argument_substitution(inner), None);
    assert_eq!(
        context.highlight_vectors(pasted, &scratch),
        [site, prefix, string, end]
    );
}
