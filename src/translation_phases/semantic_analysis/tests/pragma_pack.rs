//! Pragma state belongs to semantic record layout, in translation order.
//! C99: §6.10.6p1, p. 159; PDF p. 171 (implementation-defined pragma).

use super::*;
use crate::{
    configuration::{
        CompilerConfiguration,
        CompilerIdentity,
    },
    target::Target,
};

#[test]
fn pragma_pack_stack_controls_retained_record_layout() {
    let source = r"
#pragma pack(1)
struct A { char c; int i; };
#pragma pack()
#pragma pack(push, outer, 1)
struct B { char c; int i; };
#pragma pack(push, inner, 2)
struct C { char c; int i; };
#pragma pack(pop, outer)
struct D { char c; int i; };
#pragma pack(push)
#pragma pack(4)
struct E { char c; long long i; };
#pragma pack(pop)
struct F { char c; int i; } __attribute__((packed));
struct G { char c; int i; };
";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        with_configuration(
            source,
            CompilerConfiguration::default().with_target(target),
            |context, sema| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{:?}",
                    context.take_pending_errors()
                );
                for (tag, (size, align)) in sema
                    .types
                    .tags
                    .iter()
                    .filter(|t| t.kind == TagKind::Struct)
                    .zip([(5, 1), (5, 1), (6, 2), (8, 4), (12, 4), (5, 1), (8, 4)])
                {
                    assert_eq!(
                        tag.layout.get(),
                        Some(Layout { size, align }),
                        "{target:?}: {:?}",
                        tag.name
                    );
                }
            },
        );
    }
}

#[test]
fn pragma_pack_diagnostics_preserve_state_and_following_records() {
    let source = r"
#pragma pack(show)
#pragma pack(0x1u)
#pragma pack(3)
#pragma pack(push, outer, 2)
#pragma pack(push, inner, 4)
#pragma pack(pop, missing)
struct A { char c; int i; };
#pragma pack(pop, outer, 2)
struct B { char c; int i; };
#pragma pack()
#pragma pack(pop)
struct C { char c; int i; };
#pragma pack(push, unfinished, 1)
struct D { char c; int i; };
";
    with_source(source, |context, sema| {
        assert_eq!(context.error_count(), 0);
        let kinds: Vec<_> = context
            .take_pending_errors()
            .iter()
            .filter_map(|error| {
                if let TranslationError::Semantic(error) = error {
                    Some(error.kind)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            kinds,
            [
                SemanticErrorKind::PragmaPackShow(8),
                SemanticErrorKind::PragmaPackMalformed,
                SemanticErrorKind::PragmaPackPopNameAlignment,
                SemanticErrorKind::PragmaPackPopEmpty,
                SemanticErrorKind::PragmaPackUnterminated
            ]
        );
        let layouts: Vec<_> = sema
            .types
            .tags
            .iter()
            .map(|tag| tag.layout.get().unwrap().size)
            .collect();
        assert_eq!(layouts, [8, 6, 8, 5]);
    });
}

#[test]
fn pragma_pack_includes_keep_translation_order() {
    let directory = crate::test_support::TempDir::new("pragma-pack-header");
    let input = directory.join("main.c");
    std::fs::write(
        directory.join("pack.h"),
        "#pragma pack(push, header, 1)\nstruct Header {char c; int i;};\n",
    )
    .unwrap();
    let tu = Bump::new();
    let source = tu.alloc_str(
        "#pragma pack(2)\n#include \"pack.h\"\nstruct After {char c; int i;};\n#pragma pack(pop, \
         header)\nstruct Restored {char c; int i;};\n",
    );
    let mut context = Context::new(&tu);
    let parsed = crate::pipeline::parse_translation_unit(
        &mut context,
        &input,
        source,
        crate::headers::HeaderSearch::default(),
    );
    let sema = analyze(&mut context, &parsed);
    assert_eq!(
        context.pending_error_count(),
        0,
        "{:?}",
        context.take_pending_errors()
    );
    let layouts: Vec<_> = sema
        .types
        .tags
        .iter()
        .map(|tag| tag.layout.get().unwrap().size)
        .collect();
    assert_eq!(layouts, [5, 5, 6]);
}

/// Under `#pragma pack`, GCC (and TCC, tcc's `tests2/95_bitfields.c`)
/// starts a bit-field with an `aligned` request larger than the pack at the
/// pack's boundary; Clang leaves it at the bit cursor. The record alignment
/// is the capped request in both.
#[test]
fn pragma_pack_caps_a_bit_field_alignment_request_as_each_compiler_does() {
    let source = r"
#define A __attribute__((aligned(16)))
#pragma pack(push, 1)
struct A2 { int x: 12; char y: 6; long long z: 63; A char a: 4; long long b: 2; };
struct A3 { unsigned x: 5, y: 5, : 0, z: 5; char a: 5; A short b: 5; };
struct B1 { char c: 3; A int : 3; char d; };
#pragma pack(pop)
#pragma pack(push, 4)
struct C1 { char c: 3; A char a: 3; char d; };
struct C2 { char c: 3; __attribute__((aligned(2))) int a: 3; char d; };
#pragma pack(pop)
";
    // TCC gives an `aligned` bit-field its request as its alignment, even
    // below the type's and past a pack it does not exceed (`C1` is 3 bytes,
    // aligned to 1, and `C2` aligned to 2), which bcc does not model; only
    // its `pack(1)` records are checked.
    let layouts: [(CompilerIdentity, &[(u64, u64)]); 3] = [
        (
            CompilerIdentity::Gcc,
            &[(12, 1), (7, 1), (3, 1), (8, 4), (4, 4)],
        ),
        (CompilerIdentity::Tcc, &[(12, 1), (7, 1), (3, 1)]),
        (
            CompilerIdentity::Clang,
            &[(11, 1), (6, 1), (2, 1), (4, 4), (4, 4)],
        ),
    ];
    for (identity, layouts) in layouts {
        with_configuration(
            source,
            CompilerConfiguration::default().with_compiler_identity(identity),
            |context, sema| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{:?}",
                    context.take_pending_errors()
                );
                let tags = sema.types.tags.iter().filter(|t| t.kind == TagKind::Struct);
                for (tag, &(size, align)) in tags.zip(layouts) {
                    assert_eq!(
                        tag.layout.get(),
                        Some(Layout { size, align }),
                        "{identity:?}: {:?}",
                        tag.name
                    );
                }
            },
        );
    }
}
