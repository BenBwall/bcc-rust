//! `object_size` answered after inlining: the objects a pointer reaches,
//! the subobjects call sites name, and the unknown answers. Every answer is
//! checked by running the resolved function in the interpreter, and none may
//! exceed the bytes the pointer has left.
//! GNU extension: GCC manual, "Object Size Checking".

use super::{
    equivalence::returned,
    *,
};
use crate::{
    backend::interpreter::RuntimeValue,
    ir::Type,
};

/// Answers the queries of `text`, for a hosted unit, and returns the
/// resolved module text.
fn resolve(text: &str) -> String {
    resolve_for(text, true)
}

/// Answers the queries of `text`, whose allocation functions have their
/// library meaning if `library`, and returns the resolved module text.
fn resolve_for(text: &str, library: bool) -> String {
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse(&arena, text);
    assert_verifies(&module, "the input");
    resolve_object_sizes(&mut module, library, &scratch);
    assert_verifies(&module, "the output");
    let text = module.to_string();
    assert!(!text.contains("object_size"), "a query is left:\n{text}");
    assert!(!text.contains("subobject"), "a hint is left:\n{text}");
    text
}

/// What `@f` of the resolved `text` returns for the `i64` arguments `args`.
fn answer(text: &str, args: &[i64]) -> u64 {
    let args: Vec<RuntimeValue> = args
        .iter()
        .map(|&arg| RuntimeValue::int(Type::I64, u128::from(arg.cast_unsigned())))
        .collect();
    match returned(text, "f", &args) {
        | RuntimeValue::Int(bits) => u64::try_from(bits).expect("an i64 result"),
        | other => panic!("{other:?}\n{text}"),
    }
}

/// `@f` returning the query of mode `mode` of `pointer` after `prefix`
/// computed it in a 16-byte `slot0`.
fn query(prefix: &str, pointer: &str, mode: u8) -> String {
    let next = prefix.lines().count() + 1;
    format!(
        "\
function @f() -> i64 external {{
    slot0 = stack_slot 16, align 8
block0:
    v0 = stack_addr slot0
{prefix}    v{next} = object_size.i64 {pointer}, {mode}
    return v{next}
}}
"
    )
}

const UNKNOWN: u64 = u64::MAX;

/// The bytes left from an offset into an array, for each mode; a hint-less
/// pointer has no known subobject, so modes 1 and 3 give the unknown answer.
#[test]
fn an_array_answers_the_bytes_after_the_offset() {
    let prefix = "    v1 = iconst.i64 4\n    v2 = ptr_add inbounds v0, v1\n";
    for (mode, expected) in [(0, 12), (2, 12), (1, UNKNOWN), (3, 0)] {
        let text = resolve(&query(prefix, "v2", mode));
        assert_eq!(answer(&text, &[]), expected, "mode {mode}:\n{text}");
    }
}

/// A call site's hint names a member: modes 1 and 3 answer from the
/// member, after the callee's own offset, while modes 0 and 2 still answer
/// from the whole object.
#[test]
fn a_hinted_member_answers_modes_one_and_three() {
    // `&s.name[0]` of `struct { long id; char name[8]; } s` plus the
    // callee's `p + 2`.
    let prefix = "\
    v1 = iconst.i64 8
    v2 = ptr_add inbounds v0, v1
    v3 = subobject v2, v1
    v4 = iconst.i64 2
    v5 = ptr_add inbounds v3, v4
";
    for (mode, expected) in [(0, 6), (1, 6), (2, 6), (3, 6)] {
        let text = resolve(&query(prefix, "v5", mode));
        assert_eq!(answer(&text, &[]), expected, "mode {mode}:\n{text}");
    }
    // A hint narrower than the rest of the object: 4 bytes of member at
    // offset 0 of the 16-byte slot.
    let prefix = "    v1 = iconst.i64 4\n    v2 = subobject v0, v1\n";
    for (mode, expected) in [(0, 16), (1, 4), (2, 16), (3, 4)] {
        let text = resolve(&query(prefix, "v2", mode));
        assert_eq!(answer(&text, &[]), expected, "mode {mode}:\n{text}");
    }
}

/// The hint closest to the query wins: an outer call named the whole array,
/// an inner one a member of it.
#[test]
fn the_innermost_hint_names_the_subobject() {
    let prefix = "\
    v1 = iconst.i64 16
    v2 = subobject v0, v1
    v3 = iconst.i64 4
    v4 = subobject v2, v3
";
    let text = resolve(&query(prefix, "v4", 1));
    assert_eq!(answer(&text, &[]), 4, "{text}");
}

/// A hint that the subobject is unknown, as lowering passes for a pointer
/// a member access formed, stops the hints further out: `&p->a[1]` of a
/// `struct { char a[4]; char b[12]; }` the caller hinted as a whole gets
/// the unknown answers for modes 1 and 3, never the 15 bytes left in the
/// structure, constant or dynamic. Modes 0 and 2 still see the object.
#[test]
fn an_unknown_subobject_hint_stops_the_outer_hints() {
    let prefix = "\
    v1 = iconst.i64 16
    v2 = subobject v0, v1
    v3 = iconst.i64 1
    v4 = ptr_add inbounds v2, v3
    v5 = iconst.i64 -1
    v6 = subobject v4, v5
";
    for (mode, expected) in [(0, 15), (1, UNKNOWN), (2, 15), (3, 0)] {
        let text = resolve(&query(prefix, "v6", mode));
        assert_eq!(answer(&text, &[]), expected, "mode {mode}:\n{text}");
    }
    let dynamic = resolve(
        "\
function @f(i64) -> i64 external {
    slot0 = stack_slot 16, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    v2 = iconst.i64 16
    v3 = subobject v1, v2
    v4 = ptr_add inbounds v3, v0
    v5 = iconst.i64 -1
    v6 = subobject v4, v5
    v7 = object_size.i64 dynamic v6, 1
    v8 = object_size.i64 dynamic v6, 3
    v9 = xor.i64 v7, v8
    return v9
}
",
    );
    assert_eq!(answer(&dynamic, &[1]), UNKNOWN, "{dynamic}");
}

/// Without the C library's meaning (`-ffreestanding`), `malloc` is just a
/// function that returns some pointer.
#[test]
fn freestanding_allocations_are_unknown() {
    let text = "\
function @malloc(i64) -> ptr external

function @f() -> i64 external {
block0:
    v0 = iconst.i64 16
    v1 = call @malloc(v0)
    v2 = object_size.i64 v1, 0
    return v2
}
";
    assert_eq!(answer(&resolve_for(text, false), &[]), UNKNOWN);
    assert_eq!(answer(&resolve_for(text, true), &[]), 16);
}

/// Positions past the end or before the start leave nothing; before a
/// hinted subobject's start, mode 1 asks about an enclosing object that is
/// not known, and mode 3 gets nothing.
#[test]
fn positions_outside_the_object_never_overstate() {
    let past = "    v1 = iconst.i64 20\n    v2 = ptr_add v0, v1\n";
    let before = "    v1 = iconst.i64 -4\n    v2 = ptr_add v0, v1\n";
    for (mode, expected) in [(0, 0), (2, 0)] {
        assert_eq!(answer(&resolve(&query(past, "v2", mode)), &[]), expected);
        assert_eq!(answer(&resolve(&query(before, "v2", mode)), &[]), expected);
    }
    let hinted = "\
    v1 = iconst.i64 8
    v2 = ptr_add inbounds v0, v1
    v3 = subobject v2, v1
    v4 = iconst.i64 -2
    v5 = ptr_add v3, v4
";
    for (mode, expected) in [(0, 10), (1, UNKNOWN), (3, 0)] {
        let text = resolve(&query(hinted, "v5", mode));
        assert_eq!(answer(&text, &[]), expected, "mode {mode}:\n{text}");
    }
}

/// A pointer that may point into either of two objects gets the larger
/// remainder for modes 0 and 1 and the smaller for 2 and 3, through a
/// `select` and through a join's block parameter alike.
#[test]
fn either_of_two_objects_combines_their_remainders() {
    for (mode, expected) in [(0, 16), (2, 8)] {
        let text = resolve(&format!(
            "\
function @f(i64) -> i64 external {{
    slot0 = stack_slot 16, align 8
    slot1 = stack_slot 8, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = iconst.i64 0
    v4 = icmp.i64 eq v0, v3
    v5 = select.ptr v4, v1, v2
    v6 = object_size.i64 v5, {mode}
    brif v4, block1(v1), block2(v2)
block1(v7: ptr):
    jump block3(v7)
block2(v8: ptr):
    jump block3(v8)
block3(v9: ptr):
    v10 = object_size.i64 v9, {mode}
    v11 = iadd.i64 v6, v10
    return v11
}}
"
        ));
        assert_eq!(answer(&text, &[0]), 2 * expected, "mode {mode}:\n{text}");
    }
}

/// Pointers whose object is not known (a parameter, an integer, a loaded
/// pointer, a pointer stepped around a loop, an external or weak global)
/// get the unknown answers: all ones for modes 0 and 1, zero for 2 and 3.
/// A defined global is known.
#[test]
fn unknown_objects_get_the_unknown_answers() {
    let header = "\
global @defined external size 32, align 8 = zero
global @declared external size 32, align 8
global @replaceable weak size 32, align 8 = zero

function @g(ptr) -> i64 internal {
    slot0 = stack_slot 8, align 8
block0(v0: ptr):
";
    let cases = [
        ("", "v0", None),
        (
            "    v1 = iconst.i64 64\n    v2 = inttoptr.ptr v1\n",
            "v2",
            None,
        ),
        (
            "    v1 = stack_addr slot0\n    v2 = load.ptr v1, align 8\n",
            "v2",
            None,
        ),
        ("    v1 = global_addr @declared\n", "v1", None),
        ("    v1 = global_addr @replaceable\n", "v1", None),
        ("    v1 = global_addr @defined\n", "v1", Some(32)),
    ];
    for (mode, unknown) in [(0, UNKNOWN), (1, UNKNOWN), (2, 0), (3, 0)] {
        for (prefix, pointer, known) in cases {
            let next = prefix.lines().count() + 1;
            let text = resolve(&format!(
                "{header}{prefix}    v{next} = object_size.i64 {pointer}, {mode}
    return v{next}
}}

function @f() -> i64 external {{
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = call @g(v0)
    return v1
}}
"
            ));
            let expected = known.filter(|_| mode & 1 == 0).unwrap_or(unknown);
            assert_eq!(answer(&text, &[]), expected, "mode {mode}:\n{text}");
        }
        let looped = resolve(&format!(
            "\
function @f(i64) -> i64 external {{
    slot0 = stack_slot 8, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    v2 = iconst.i64 1
    jump block1(v1, v0)
block1(v3: ptr, v4: i64):
    v5 = ptr_add inbounds v3, v2
    v6 = isub.i64 v4, v2
    v7 = iconst.i64 0
    v8 = icmp.i64 sgt v6, v7
    brif v8, block1(v5, v6), block2
block2:
    v9 = object_size.i64 v3, {mode}
    return v9
}}
"
        ));
        assert_eq!(answer(&looped, &[3]), unknown, "mode {mode}:\n{looped}");
    }
}

/// A loop that passes its pointer on unchanged keeps its object, and the
/// test glibc's wrappers make of an answer folds once it is answered.
#[test]
fn answers_fold_the_tests_that_guard_checked_calls() {
    let text = resolve(
        "\
function @check(ptr, i64) -> i64 external

function @f(i64) -> i64 external {
    slot0 = stack_slot 16, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    jump block1(v1, v0)
block1(v2: ptr, v3: i64):
    v4 = iconst.i64 1
    v5 = isub.i64 v3, v4
    v6 = iconst.i64 0
    v7 = icmp.i64 sgt v5, v6
    brif v7, block1(v2, v5), block2
block2:
    v8 = object_size.i64 v2, 0
    v9 = iconst.i64 -1
    v10 = icmp.i64 ne v8, v9
    brif v10, block3, block4
block3:
    v11 = call @check(v2, v8)
    return v11
block4:
    return v9
}
",
    );
    assert!(text.contains("call @check(v"), "{text}");
    assert!(!text.contains("icmp.i64 ne"), "{text}");
    assert!(text.contains("iconst.i64 16"), "{text}");
}

/// `malloc`, `calloc` and `realloc` with constant sizes name their objects;
/// a dynamic query of a runtime size or offset computes its answer.
#[test]
fn allocations_and_runtime_sizes() {
    let text = resolve(
        "\
function @malloc(i64) -> ptr external
function @calloc(i64, i64) -> ptr external

function @f(i64) -> i64 external {
block0(v0: i64):
    v1 = iconst.i64 10
    v2 = call @malloc(v1)
    v3 = iconst.i64 3
    v4 = call @calloc(v3, v1)
    v5 = ptr_add inbounds v4, v3
    v6 = object_size.i64 v2, 0
    v7 = object_size.i64 v5, 2
    v8 = iadd.i64 v6, v7
    return v8
}
",
    );
    assert_eq!(answer(&text, &[0]), 10 + 27, "{text}");
    let dynamic = resolve(
        "\
function @f(i64, i64) -> i64 external {
block0(v0: i64, v1: i64):
    v2 = dyn_alloca.i64 v0, align 16
    v3 = ptr_add inbounds v2, v1
    v4 = object_size.i64 dynamic v3, 0
    v5 = object_size.i64 v3, 0
    v6 = object_size.i64 v3, 2
    v7 = xor.i64 v5, v6
    v8 = iconst.i64 -1
    v9 = icmp.i64 eq v7, v8
    v10 = iconst.i64 999
    v11 = select.i64 v9, v4, v10
    return v11
}
",
    );
    // The static queries of a runtime size are unknown (-1 and 0); the
    // dynamic one is the bytes after the offset, and none past the end.
    for (size, offset, expected) in [(24, 4, 20), (24, 24, 0), (24, 30, 0), (8, -1, 0)] {
        assert_eq!(answer(&dynamic, &[size, offset]), expected, "{dynamic}");
    }
}

/// A dynamic query of a hinted subobject at a runtime offset: the bytes
/// left in the member, never more than in the object.
#[test]
fn a_dynamic_subobject_query_at_a_runtime_offset() {
    let text = resolve(
        "\
function @f(i64) -> i64 external {
    slot0 = stack_slot 16, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    v2 = iconst.i64 8
    v3 = ptr_add inbounds v1, v2
    v4 = iconst.i64 6
    v5 = subobject v3, v4
    v6 = ptr_add inbounds v5, v0
    v7 = object_size.i64 dynamic v6, 1
    v8 = object_size.i64 dynamic v6, 3
    v9 = iconst.i64 32
    v10 = shl.i64 v7, v9
    v11 = or.i64 v10, v8
    return v11
}
",
    );
    for (offset, most, least) in [(0, 6, 6), (5, 1, 1), (7, 0, 0), (-1, 0xFFFF_FFFF, 0)] {
        let bits = answer(&text, &[offset]);
        assert_eq!(bits >> 32, most, "offset {offset}:\n{text}");
        assert_eq!(bits & 0xFFFF_FFFF, least, "offset {offset}:\n{text}");
    }
}

/// A query whose offset depends on another query's answer waits for it:
/// `__builtin_object_size (p + ((__builtin_object_size (p, 1) + 1) & 1), 1)`
/// of a 4-byte array is 3, never the 4 the unresolved offset would allow.
#[test]
fn an_offset_computed_from_another_answer_is_folded_first() {
    let text = resolve(
        "\
function @f() -> i64 external {
    slot0 = stack_slot 4, align 1
block0:
    v0 = stack_addr slot0
    v1 = iconst.i64 4
    v2 = subobject v0, v1
    v3 = object_size.i64 v2, 1
    v4 = iconst.i64 1
    v5 = iadd.i64 v3, v4
    v6 = and.i64 v5, v4
    v7 = ptr_add inbounds v2, v6
    v8 = object_size.i64 v7, 1
    return v8
}
",
    );
    assert_eq!(answer(&text, &[]), 3, "{text}");
}

/// An `always_inline` wrapper's query of its parameter reaches the caller's
/// object once expanded; a wrapper calling it is expanded too, and the
/// LLVM back end's copies keep their bodies.
#[test]
fn wrappers_are_expanded_before_their_queries_are_answered() {
    let text = resolve(
        "\
function @inner.always_inline(ptr) -> i64 internal alwaysinline inline_copy_of @inner {
block0(v0: ptr):
    v1 = object_size.i64 v0, 1
    return v1
}

function @inner(ptr) -> i64 external alwaysinline

function @outer(ptr) -> i64 internal alwaysinline {
block0(v0: ptr):
    v1 = call @inner.always_inline(v0)
    return v1
}

function @f() -> i64 external {
    slot0 = stack_slot 12, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i64 12
    v2 = subobject v0, v1
    v3 = call @outer(v2)
    return v3
}
",
    );
    assert_eq!(answer(&text, &[]), 12, "{text}");
    assert!(
        text.contains("function @inner.always_inline(ptr)"),
        "{text}"
    );
}

/// The textual form of both operations round-trips, and an unresolved
/// query runs in the interpreter with the unknown answer.
#[test]
fn queries_print_parse_and_run_unresolved() {
    let text = "\
function @f() -> i64 external {
    slot0 = stack_slot 16, align 8
block0:
    v0 = stack_addr slot0
    v1 = iconst.i64 4
    v2 = subobject v0, v1
    v3 = object_size.i64 dynamic v2, 3
    v4 = object_size.i64 v2, 1
    v5 = iadd.i64 v3, v4
    return v5
}
";
    let arena = Bump::new();
    let module = parse(&arena, text);
    assert_verifies(&module, "the text");
    pretty_assertions::assert_eq!(module.to_string(), text);
    assert_eq!(answer(text, &[]), UNKNOWN);
}
