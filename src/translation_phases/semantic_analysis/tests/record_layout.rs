//! Record layouts checked against the platform compilers: MinGW-w64 GCC and
//! Clang for `x86_64-w64-windows-gnu`, `cl.exe` and Clang for
//! `x86_64-pc-windows-msvc`, GCC and Clang for Linux. Where the references
//! disagree the test names whom bcc follows. `scripts/layout_fuzz.py`
//! compares random records the same way.

use super::*;
use crate::{
    configuration::CompilerConfiguration,
    target::Target,
};

/// One record: `(size, align, [(field, bit offset)])`.
type Expected = (u64, u64, &'static [(&'static str, u64)]);

fn check(target: Target, source: &str, records: &[(&str, Expected)]) {
    with_configuration(
        source,
        CompilerConfiguration::default().with_target(target),
        |context, s| {
            let errors = context.take_pending_errors();
            assert!(errors.is_empty(), "{target:?}: {errors:?}");
            for &(name, (size, align, fields)) in records {
                let tag = s
                    .types
                    .tags
                    .iter()
                    .find(|tag| tag.name.is_some_and(|n| context.string_cache.at(n) == name))
                    .unwrap_or_else(|| panic!("no record {name}"));
                assert_eq!(
                    tag.layout.get(),
                    Some(Layout { size, align }),
                    "{target:?} {name}"
                );
                for &(field, bit) in fields {
                    let found = tag
                        .fields
                        .get()
                        .iter()
                        .find(|f| context.string_cache.at(f.name.name) == field)
                        .unwrap_or_else(|| panic!("no field {name}.{field}"));
                    assert_eq!(
                        found.offset * 8 + u64::from(found.bit_offset),
                        bit,
                        "{target:?} {name}.{field}"
                    );
                }
            }
        },
    );
}

/// Microsoft bit-field layout (both Windows targets): a zero-width bit-field
/// after a nonzero one ends its allocation unit, so the next member starts
/// after the whole unit, aligned for the zero-width field's type (#55).
const ZERO_WIDTH: &str = "
struct N1 { int a : 5; short : 0; char b : 6; };
struct N2 { long long a : 15; unsigned char : 0; int b; };
#pragma pack(push, 1)
struct P1 { char a; unsigned b : 18; unsigned : 0; unsigned c : 4; };
struct P2 { long long a : 27; char : 0; };
#pragma pack(4)
struct P3 { long long a : 9; _Bool : 0; short b; };
#pragma pack(2)
struct P4 { unsigned short a : 13; short b : 6; unsigned long long : 0; };
struct P5 { long long a; unsigned long long b : 15; int : 0; char c; };
#pragma pack(pop)
struct Z1 { char a; int : 0; char b; };
struct Z2 { char a : 3; long long : 0; char b; };
";

/// Records on which every reference agrees, on both Windows targets.
const AGREED: &[(&str, Expected)] = &[
    ("N1", (8, 4, &[("b", 32)])),
    ("N2", (16, 8, &[("b", 64)])),
    ("P2", (8, 1, &[])),
    ("P3", (12, 4, &[("b", 64)])),
    ("Z1", (2, 1, &[("b", 8)])),
    ("Z2", (16, 8, &[("b", 64)])),
];

#[test]
fn zero_width_bit_fields_end_the_microsoft_unit() {
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        check(target, ZERO_WIDTH, AGREED);
        // `#pragma pack` caps the zero-width field's alignment. MSVC, MinGW
        // GCC and Clang's MSVC layout agree; Clang's MinGW layout ignores the
        // cap (P1: 8/4, P4: 8/8, P5: 20/4) and so overlaps P1's fields, so
        // the MinGW target follows GCC.
        check(
            target,
            ZERO_WIDTH,
            &[
                ("P1", (9, 1, &[("c", 40)])),
                ("P4", (4, 2, &[("b", 16)])),
                ("P5", (18, 2, &[("c", 128)])),
            ],
        );
    }
}

#[test]
fn packed_mingw_records_keep_the_zero_width_alignment() {
    // MinGW GCC: packing removes the padding before the next unit, but the
    // zero-width field's type still aligns the record (Clang: A1 8/4 with
    // `c` past the end of the record, A2 places `b` at bit 64).
    check(
        Target::WindowsGnu,
        "struct A1 { char a; unsigned b : 18; unsigned : 0; unsigned c : 4; } \
         __attribute__((packed));
         struct A2 { _Bool a : 1; unsigned long long : 0; unsigned long long b; } \
         __attribute__((packed));",
        &[("A1", (12, 4, &[("c", 40)])), ("A2", (16, 8, &[("b", 8)]))],
    );
}

#[test]
fn msvc_unions_reserve_a_closing_zero_width_type() {
    let source = "union U1 { char a : 1; _Bool : 1; short : 0; unsigned : 0; };
                  union U2 { short a : 15; unsigned : 0; unsigned char b; };";
    // `cl.exe` and Clang's MSVC layout: only a zero-width field that follows
    // a nonzero one counts, by its type's size but not its alignment.
    check(
        Target::WindowsMsvc,
        source,
        &[("U1", (2, 1, &[])), ("U2", (4, 1, &[]))],
    );
    // MinGW GCC and Clang ignore zero-width union members (U2's alignment:
    // GCC 2, Clang 1, bcc Clang's).
    check(
        Target::WindowsGnu,
        source,
        &[("U1", (1, 1, &[])), ("U2", (2, 1, &[]))],
    );
}

#[test]
fn msvc_required_alignment_survives_pragma_pack() {
    // `cl.exe` and Clang: `_Alignas` inside a nested record still aligns it,
    // and the containing record, under `#pragma pack(1)`. MinGW does not.
    let source = "struct In { _Alignas(16) char c; };
                  #pragma pack(push, 1)
                  struct Out { char a; struct In in; char b; };
                  struct OutArr { char a; struct In in[2]; };
                  #pragma pack(pop)";
    check(
        Target::WindowsMsvc,
        source,
        &[
            ("Out", (48, 16, &[("in", 128), ("b", 256)])),
            ("OutArr", (48, 16, &[("in", 128)])),
        ],
    );
    check(
        Target::WindowsGnu,
        source,
        &[
            ("Out", (18, 1, &[("in", 8), ("b", 136)])),
            ("OutArr", (33, 1, &[("in", 8)])),
        ],
    );
}

#[test]
fn system_v_aligned_bit_fields_follow_gcc_and_clang() {
    // GCC and Clang agree: an `aligned` request no larger than the pack
    // aligns the field's start; without a pack an unstraddled field is
    // aligned only as requested; `#pragma pack` overrides `packed`.
    check(
        Target::LinuxGnu,
        "#pragma pack(push, 4)
         struct L1 { int a; _Bool b : 1; char c : 1 __attribute__((aligned(1))); unsigned short d \
         : 14; };
         #pragma pack(8)
         struct L2 { int a : 6 __attribute__((aligned(2))); char b : 8 \
         __attribute__((aligned(8))); };
         #pragma pack(pop)
         struct L3 { char a; int b : 4 __attribute__((aligned(2))); };
         #pragma pack(push, 4)
         struct L4 { char a; int b : 4 __attribute__((packed)); };
         #pragma pack(pop)",
        &[
            ("L1", (8, 4, &[("c", 40), ("d", 41)])),
            ("L2", (16, 8, &[("b", 64)])),
            ("L3", (4, 4, &[("b", 16)])),
            ("L4", (4, 4, &[("b", 8)])),
        ],
    );
}
