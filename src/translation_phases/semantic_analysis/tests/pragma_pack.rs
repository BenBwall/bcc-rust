//! Pragma state belongs to semantic record layout, in translation order.
//! C99: §6.10.6p1, p. 159; PDF p. 171 (implementation-defined pragma).

use super::*;
use crate::{
    configuration::CompilerConfiguration,
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
