//! Classification: the System V eightbyte classes of aggregate layouts, and
//! the lowered signatures each target gives aggregate parameters and results
//! and narrow integers.

use sysv::{
    Class,
    Classification,
    classify,
};

use super::*;
use crate::ir::{
    AggregateFlags,
    AggregateLayout,
    AggregateScalar,
    Align,
    Type,
};

/// A layout of `size` bytes aligned to `align` made of `scalars`.
fn layout(
    size: u64,
    align: u64,
    flags: AggregateFlags,
    scalars: &[(u32, Type)],
) -> AggregateLayout<'static> {
    let scalars: Vec<AggregateScalar> = scalars
        .iter()
        .map(|&(offset, ty)| AggregateScalar { offset, ty })
        .collect();
    AggregateLayout {
        size,
        align: Align::from_bytes(align).unwrap(),
        flags,
        scalars: Vec::leak(scalars),
        first: None,
    }
}

fn classes(size: u64, align: u64, scalars: &[(u32, Type)]) -> Classification {
    classify(layout(size, align, AggregateFlags::empty(), scalars))
}

#[test]
fn integer_float_and_mixed_eightbytes() {
    use Class::*;
    use Type::*;
    let cases: [(&str, Classification); 10] = [
        ("int pair", classes(8, 4, &[(0, I32), (4, I32)])),
        ("long pair", classes(16, 8, &[(0, I64), (8, I64)])),
        ("double pair", classes(16, 8, &[(0, F64), (8, F64)])),
        ("long and double", classes(16, 8, &[(0, I64), (8, F64)])),
        ("float and int", classes(8, 4, &[(0, F32), (4, I32)])),
        (
            "three floats",
            classes(12, 4, &[(0, F32), (4, F32), (8, F32)]),
        ),
        ("char and double", classes(16, 8, &[(0, I8), (8, F64)])),
        (
            "int[2] and float[2]",
            classes(16, 4, &[(0, I32), (4, I32), (8, F32), (12, F32)]),
        ),
        ("pointer", classes(8, 8, &[(0, Ptr)])),
        ("empty", classes(0, 1, &[])),
    ];
    let expected = [
        [Integer, NoClass],
        [Integer, Integer],
        [Sse, Sse],
        [Integer, Sse],
        [Integer, NoClass],
        [Sse, Sse],
        [Integer, Sse],
        [Integer, Sse],
        [Integer, NoClass],
        [NoClass, NoClass],
    ];
    for ((name, actual), expected) in cases.into_iter().zip(expected) {
        assert_eq!(actual, Classification::Classes(expected), "{name}");
    }
}

#[test]
fn wide_scalars_span_both_eightbytes() {
    use Class::*;
    assert_eq!(
        classes(16, 16, &[(0, Type::I128)]),
        Classification::Classes([Integer, Integer])
    );
    assert_eq!(
        classes(16, 16, &[(0, Type::F128)]),
        Classification::Classes([Sse, SseUp])
    );
    assert_eq!(
        classes(16, 16, &[(0, Type::F80)]),
        Classification::Classes([X87, X87Up])
    );
    // A union of `__int128` and `__float128`: INTEGER wins the merge.
    assert_eq!(
        classes(16, 16, &[(0, Type::F128), (0, Type::I128)]),
        Classification::Classes([Integer, Integer])
    );
}

#[test]
fn memory_by_size_alignment_flexibility_and_x87_merges() {
    let memory = [
        ("three longs", classes(24, 8, &[])),
        (
            "packed char and int",
            classify(layout(
                5,
                1,
                AggregateFlags::UNALIGNED,
                &[(0, Type::I8), (1, Type::I32)],
            )),
        ),
        (
            "flexible array member",
            classify(layout(4, 4, AggregateFlags::FLEXIBLE, &[(0, Type::I32)])),
        ),
        (
            "union of long double and int",
            classes(16, 16, &[(0, Type::F80), (0, Type::I32)]),
        ),
        (
            "long double after a double",
            classes(16, 8, &[(0, Type::F64), (8, Type::F80)]),
        ),
    ];
    for (name, classification) in memory {
        assert_eq!(classification, Classification::Memory, "{name}");
    }
}

#[test]
fn unions_merge_their_members() {
    use Class::*;
    assert_eq!(
        classes(4, 4, &[(0, Type::F32), (0, Type::I32), (0, Type::I8)]),
        Classification::Classes([Integer, NoClass])
    );
    assert_eq!(
        classes(8, 8, &[(0, Type::F64), (0, Type::F32)]),
        Classification::Classes([Sse, NoClass])
    );
}

/// Functions taking and returning one aggregate each, for both targets.
const AGGREGATES: &str = "\
function @ints(ptr result(size 8, align 4, [0: i32, 4: i32]), ptr aggregate(size 8, align 4, [0: \
                          i32, 4: i32])) external
function @longs(ptr result(size 16, align 8, [0: i64, 8: i64]), ptr aggregate(size 16, align 8, \
                          [0: i64, 8: i64])) external
function @mixed(ptr result(size 16, align 8, [0: i64, 8: f64]), ptr aggregate(size 16, align 8, \
                          [0: i64, 8: f64])) external
function @floats(ptr result(size 12, align 4, [0: f32, 4: f32, 8: f32]), ptr aggregate(size 12, \
                          align 4, [0: f32, 4: f32, 8: f32])) external
function @chars(ptr result(size 3, align 1, [0: i8, 1: i8, 2: i8]), ptr aggregate(size 3, align 1, \
                          [0: i8, 1: i8, 2: i8])) external
function @big(ptr result(size 24, align 8), ptr aggregate(size 24, align 8)) external
function @packed(ptr aggregate(size 5, align 1, unaligned, [0: i8, 1: i32])) external
function @flexible(ptr aggregate(size 4, align 4, flexible, [0: i32])) external
function @empty(ptr result(size 0, align 1), ptr aggregate(size 0, align 1), i32) external
function @ld(ptr result(size 16, align 16, [0: f80]), ptr aggregate(size 16, align 16, [0: f80])) \
                          external
function @quad(ptr result(size 16, align 16, [0: f128]), ptr aggregate(size 16, align 16, [0: \
                          f128])) external
function @pointer(ptr result(size 8, align 8, [0: ptr]), ptr aggregate(size 8, align 8, [0: ptr])) \
                          external
";

#[test]
fn system_v_signatures() {
    pretty_assertions::assert_eq!(
        lowered_signatures(AGGREGATES, Target::LinuxGnu),
        "\
@ints(ptr registers(i64, align 4), i64)
@longs(ptr registers(i64, i64, align 8), i64, i64)
@mixed(ptr registers(i64, f64, align 8), i64, f64)
@floats(ptr registers(f64, f32, align 4), f64, f32)
@chars(ptr registers(i32, align 1), i32)
@big(ptr sret(size 24, align 8), ptr byval(size 24, align 8))
@packed(ptr byval(size 5, align 8, unaligned))
@flexible(ptr byval(size 4, align 8, flexible))
@empty(i32)
@ld(ptr registers(f80, align 16), ptr byval(size 16, align 16))
@quad(ptr registers(f128, align 16), f128)
@pointer(ptr registers(i64, align 8), ptr)"
    );
}

#[test]
fn win64_signatures() {
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        pretty_assertions::assert_eq!(
            lowered_signatures(AGGREGATES, target),
            "\
@ints(ptr registers(i64, align 4), i64)
@longs(ptr sret(size 16, align 8), ptr)
@mixed(ptr sret(size 16, align 8), ptr)
@floats(ptr sret(size 12, align 4), ptr)
@chars(ptr sret(size 3, align 1), ptr)
@big(ptr sret(size 24, align 8), ptr)
@packed(ptr)
@flexible(ptr)
@empty(ptr sret(size 0, align 1), ptr, i32)
@ld(ptr sret(size 16, align 16), ptr)
@quad(ptr sret(size 16, align 16), ptr)
@pointer(ptr registers(i64, align 8), i64)",
            "{target:?}"
        );
    }
}

/// System V gives an aggregate registers only if all it needs are free;
/// otherwise it goes to the stack whole and later scalars still take the
/// registers left. A memory result takes the first general register.
#[test]
fn system_v_counts_free_registers() {
    let text = "\
function @gp(i64, i64, i64, i64, i64, ptr aggregate(size 16, align 8, [0: i64, 8: i64]), i64) \
                external
function @sse(f64, f64, f64, f64, f64, f64, f64, ptr aggregate(size 16, align 8, [0: f64, 8: \
                f64]), f64) external
function @sret(ptr result(size 32, align 8), i64, i64, i64, i64, i64, ptr aggregate(size 8, align \
                8, [0: i64])) external
function @fits(ptr result(size 32, align 8), i64, i64, i64, i64, ptr aggregate(size 8, align 8, \
                [0: i64])) external
";
    pretty_assertions::assert_eq!(
        lowered_signatures(text, Target::LinuxGnu),
        "\
@gp(i64, i64, i64, i64, i64, ptr byval(size 16, align 8), i64)
@sse(f64, f64, f64, f64, f64, f64, f64, ptr byval(size 16, align 8), f64)
@sret(ptr sret(size 32, align 8), i64, i64, i64, i64, i64, ptr byval(size 8, align 8))
@fits(ptr sret(size 32, align 8), i64, i64, i64, i64, i64)"
    );
}

/// Narrow integers keep their place among the lowered parameters and take
/// the target's extension: System V widens each of them, Win64 only
/// `_Bool`.
#[test]
fn narrow_integers_take_the_targets_extension() {
    let text = "\
function @narrow(i8 signed, i16 unsigned, i8 bool, i32) -> i16 signed external
function @mixed(ptr result(size 16, align 8, [0: i64, 8: i64]), i8 unsigned, ptr aggregate(size \
                16, align 8, [0: i64, 8: i64]), i16 signed) external
function @truth(ptr aggregate(size 0, align 1), i8 bool) -> i8 bool external
";
    pretty_assertions::assert_eq!(
        lowered_signatures(text, Target::LinuxGnu),
        "\
@narrow(i8 signext, i16 zeroext, i8 zeroext, i32) -> i16 signext
@mixed(ptr registers(i64, i64, align 8), i8 zeroext, i64, i64, i16 signext)
@truth(i8 zeroext) -> i8 zeroext"
    );
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        pretty_assertions::assert_eq!(
            lowered_signatures(text, target),
            "\
@narrow(i8, i16, i8 zeroext, i32) -> i16
@mixed(ptr sret(size 16, align 8), i8, ptr, i16)
@truth(ptr, i8 zeroext) -> i8 zeroext",
            "{target:?}"
        );
    }
}
