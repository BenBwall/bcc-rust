//! Context storage and provenance fixtures.
//!
//! C99: phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! Diagnostic locations: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.

use super::{
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
