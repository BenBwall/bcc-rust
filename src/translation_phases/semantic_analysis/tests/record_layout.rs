//! Record layouts checked against the platform compilers: MinGW-w64 GCC and
//! Clang for `x86_64-w64-windows-gnu`, `cl.exe` and Clang for
//! `x86_64-pc-windows-msvc`, GCC and Clang for Linux. Where the references
//! disagree the test names whom bcc follows. `scripts/layout_fuzz.py`
//! compares random records the same way.

use super::*;
use crate::{
    configuration::{
        CompilerConfiguration,
        CompilerIdentity,
    },
    target::Target,
};

/// One record: `(size, align, [(field, bit offset)])`.
type Expected = (u64, u64, &'static [(&'static str, u64)]);

fn check(target: Target, source: &str, records: &[(&str, Expected)]) {
    check_as(CompilerIdentity::Clang, target, source, records);
}

/// [`check`] under `identity`'s rules.
fn check_as(
    identity: CompilerIdentity,
    target: Target,
    source: &str,
    records: &[(&str, Expected)],
) {
    with_configuration(
        source,
        CompilerConfiguration::default()
            .with_target(target)
            .with_compiler_identity(identity),
        |context, s| {
            let errors = context.take_pending_errors();
            assert!(errors.is_empty(), "{identity:?} {target:?}: {errors:?}");
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
                    "{identity:?} {target:?} {name}"
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
                        "{identity:?} {target:?} {name}.{field}"
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
    // MinGW GCC and Clang ignore zero-width union members; GCC aligns the
    // union for its bit-field `a`, Clang does not.
    check(
        Target::WindowsGnu,
        source,
        &[("U1", (1, 1, &[])), ("U2", (2, 1, &[]))],
    );
    check_as(
        CompilerIdentity::Gcc,
        Target::WindowsGnu,
        source,
        &[("U1", (1, 1, &[])), ("U2", (2, 2, &[]))],
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

/// `ms_struct` unions on a System V target. GCC 14 aligns the union for its
/// nonzero bit-fields, named or not, and packing shrinks one to its bits;
/// Clang 23 ignores a union bit-field's alignment and packing.
#[test]
fn ms_struct_unions_follow_the_identity() {
    let source = "union __attribute__((ms_struct)) U { unsigned char a : 4; unsigned int b : 4; \
                  char c; };
                  union __attribute__((ms_struct)) A { char c; int : 3; };
                  union __attribute__((ms_struct, packed)) C { char c; int b : 12; };
                  #pragma pack(push, 2)
                  union __attribute__((ms_struct)) D { char c; long long b : 12; };
                  #pragma pack(pop)
                  union __attribute__((ms_struct)) G { char c; int : 0; };";
    check_as(
        CompilerIdentity::Gcc,
        Target::LinuxGnu,
        source,
        &[
            ("U", (4, 4, &[("c", 0)])),
            ("A", (4, 4, &[])),
            ("C", (2, 1, &[])),
            ("D", (2, 2, &[])),
            ("G", (1, 1, &[])),
        ],
    );
    check_as(
        CompilerIdentity::Clang,
        Target::LinuxGnu,
        source,
        &[
            ("U", (4, 1, &[("c", 0)])),
            ("A", (4, 1, &[])),
            ("C", (4, 1, &[])),
            ("D", (8, 1, &[])),
            ("G", (1, 1, &[])),
        ],
    );
}

/// `ms_struct` or `gcc_struct` on a declaration before the definition: GCC
/// 14 reads only the definition's, and takes the first of two there;
/// Clang 23 keeps an earlier declaration's, even a reference's, and
/// `gcc_struct` on any declaration wins.
#[test]
fn record_layout_requests_on_earlier_declarations_follow_the_identity() {
    let source = "struct __attribute__((ms_struct)) F;
                  struct F { unsigned char a : 4; unsigned int b : 4; char c; };
                  struct __attribute__((ms_struct)) R *p;
                  struct R { unsigned char a : 4; unsigned int b : 4; char c; };
                  struct __attribute__((gcc_struct)) H;
                  struct __attribute__((ms_struct)) H { unsigned char a : 4; unsigned int b : 4; \
                  char c; };";
    check_as(
        CompilerIdentity::Gcc,
        Target::LinuxGnu,
        &format!(
            "{source}\nstruct __attribute__((ms_struct, gcc_struct)) M {{ unsigned char a : 4; \
             unsigned int b : 4; char c; }};"
        ),
        &[
            ("F", (4, 4, &[("c", 8)])),
            ("R", (4, 4, &[("c", 8)])),
            ("H", (12, 4, &[("c", 64)])),
            ("M", (12, 4, &[("c", 64)])),
        ],
    );
    check_as(
        CompilerIdentity::Clang,
        Target::LinuxGnu,
        source,
        &[
            ("F", (12, 4, &[("c", 64)])),
            ("R", (12, 4, &[("c", 64)])),
            ("H", (4, 4, &[("c", 8)])),
        ],
    );
}

/// A zero-width bit-field ending an `ms_struct` unit in a packed record or
/// under `#pragma pack` on a System V target: GCC 14 caps its boundary
/// with the pack and drops it in a packed record; Clang 23 aligns the next
/// field and the record to the declared type's size regardless. A
/// zero-width field that ends no unit counts only with an explicit
/// alignment, which GCC caps with the pack and keeps out of the record's.
#[test]
fn packed_ms_struct_zero_width_fields_follow_the_identity() {
    let source = "struct __attribute__((ms_struct, packed)) ZP { unsigned char a : 3; int : 0; \
                  char b; };
                  #pragma pack(push, 1)
                  struct __attribute__((ms_struct)) ZK { unsigned char a : 3; int : 0; char b; };
                  struct __attribute__((ms_struct)) ZT { unsigned char a : 3; int : 0; };
                  #pragma pack(2)
                  struct __attribute__((ms_struct)) ZA { char x; int : 0 \
                  __attribute__((aligned(8))); char b; };
                  #pragma pack(pop)
                  struct __attribute__((ms_struct)) ZN { char x; int : 0 \
                  __attribute__((aligned(8))); char b; };";
    check_as(
        CompilerIdentity::Gcc,
        Target::LinuxGnu,
        source,
        &[
            ("ZP", (4, 4, &[("b", 8)])),
            ("ZK", (2, 1, &[("b", 8)])),
            ("ZT", (1, 1, &[])),
            ("ZA", (3, 1, &[("b", 16)])),
            ("ZN", (9, 1, &[("b", 64)])),
        ],
    );
    check_as(
        CompilerIdentity::Clang,
        Target::LinuxGnu,
        source,
        &[
            ("ZP", (8, 4, &[("b", 32)])),
            ("ZK", (8, 4, &[("b", 32)])),
            ("ZT", (4, 4, &[])),
            ("ZA", (16, 8, &[("b", 64)])),
            ("ZN", (16, 8, &[("b", 64)])),
        ],
    );
}

/// Clang 23 has no System V record layout for the Microsoft ABI: under its
/// rules a `gcc_struct` record on an MSVC target, by its definition or an
/// earlier declaration, is an error. On MinGW and Linux it is laid out.
#[test]
fn gcc_struct_on_msvc_targets_is_an_error_under_clang() {
    let source = "struct __attribute__((gcc_struct)) S { unsigned char a : 4; unsigned int b : 4; \
                  char c; };
                  struct __attribute__((gcc_struct)) F;
                  struct F { int x; };
                  struct __attribute__((ms_struct)) M { int x; };";
    for (identity, target, expected) in [
        (CompilerIdentity::Clang, Target::WindowsMsvc, 2),
        (CompilerIdentity::ClangCl, Target::WindowsMsvc, 2),
        (CompilerIdentity::Clang, Target::WindowsGnu, 0),
        (CompilerIdentity::Gcc, Target::WindowsGnu, 0),
        (CompilerIdentity::Clang, Target::LinuxGnu, 0),
    ] {
        with_configuration(
            source,
            CompilerConfiguration::default()
                .with_target(target)
                .with_compiler_identity(identity),
            |context, _| {
                let errors = context.take_pending_errors();
                let layout = errors
                    .iter()
                    .filter(|error| {
                        matches!(error, TranslationError::Semantic(error)
                            if error.kind == SemanticErrorKind::ItaniumLayoutOnMicrosoftAbi)
                    })
                    .count();
                assert_eq!(layout, expected, "{identity:?} {target:?}: {errors:?}");
            },
        );
    }
}

/// A bit-field in a union occupies its width in whole bytes, and the
/// union's alignment rounds the size up, which shows when a typedef lowers
/// the declared type's alignment: GCC 13 and Clang 23 agree.
#[test]
fn union_bit_fields_occupy_their_bytes() {
    let source = "typedef long L2 __attribute__((aligned(2)));
                  union U { char z; L2 b : 33; };
                  union V { long b : 33; };";
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        check_as(
            identity,
            Target::LinuxGnu,
            source,
            &[("U", (6, 2, &[("b", 0)])), ("V", (8, 8, &[("b", 0)]))],
        );
    }
}
