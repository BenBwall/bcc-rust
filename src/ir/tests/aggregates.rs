//! Aggregate parameters in signatures: their textual form, the `abi
//! lowered` marker, and the verifier's profile and placement rules.

use super::*;

/// A pre-ABI module whose functions pass and return structures by value.
const PRE_ABI: &str = "\
function @pair(ptr result(size 16, align 8, [0: i64, 8: i64]), i32, ptr aggregate(size 12, align \
                       4, [0: i32, 4: f32, 8: f32])) external

function @packed(ptr aggregate(size 5, align 1, unaligned, [0: i8, 1: i32])) external

function @tail(ptr aggregate(size 4, align 4, flexible, [0: i32]), ...) -> i32 external

function @big(ptr aggregate(size 40, align 8)) external {
    slot0 = stack_slot 16, align 8
block0(v0: ptr):
    v1 = stack_addr slot0
    v2 = iconst.i32 7
    call @pair(v1, v2, v0)
    v3 = func_addr @pair
    call_indirect v3(v1, v2, v0) : (ptr result(size 16, align 8, [0: i64, 8: i64]), i32, ptr \
                       aggregate(size 12, align 4, [0: i32, 4: f32, 8: f32]))
    return
}
";

/// The post-ABI forms: memory arguments and results, and results in
/// registers.
const POST_ABI: &str = "\
abi lowered

function @memory(ptr sret(size 24, align 8), ptr byval(size 24, align 8)) external

function @registers(ptr registers(i64, f64, align 8), i64, f64) external

function @one(ptr registers(i32, align 4)) external
";

#[test]
fn register_pairs_must_have_the_eightbyte_return_area_layout() {
    for (first, second) in [
        ("i64", "i32"),
        ("i32", "i64"),
        ("f64", "f32"),
        ("f32", "f64"),
    ] {
        let arena = Bump::new();
        let text = format!(
            "abi lowered\nfunction @f(ptr registers({first}, {second}, align 4)) external\n"
        );
        let module = parse(&arena, &text);
        assert!(verify(&module, Profile::PostAbi).is_empty(), "{text}");
    }
    for (first, second) in [
        ("i32", "i32"),
        ("f32", "f32"),
        ("i128", "i64"),
        ("i64", "f80"),
        ("f128", "f128"),
    ] {
        let arena = Bump::new();
        let text = format!(
            "abi lowered\nfunction @f(ptr registers({first}, {second}, align 4)) external\n"
        );
        let module = parse(&arena, &text);
        assert_eq!(
            verify(&module, Profile::PostAbi),
            ["@f: parameter 0 has a register pair incompatible with the eightbyte return area"],
            "{text}"
        );
    }
}

#[test]
fn aggregate_parameters_round_trip() {
    assert_round_trip(PRE_ABI);
    assert_round_trip(POST_ABI);
    let arena = Bump::new();
    assert_eq!(parse(&arena, PRE_ABI).profile(), Profile::PreAbi);
    assert_eq!(parse(&arena, POST_ABI).profile(), Profile::PostAbi);
}

#[test]
fn each_profile_accepts_only_its_own_kinds() {
    let arena = Bump::new();
    let pre = parse(&arena, PRE_ABI);
    assert_eq!(verify(&pre, Profile::PreAbi), Vec::<String>::new());
    assert_eq!(
        verify(&pre, Profile::PostAbi),
        [
            "@pair: parameter 0 has an aggregate kind of the other profile",
            "@pair: parameter 2 has an aggregate kind of the other profile",
            "@packed: parameter 0 has an aggregate kind of the other profile",
            "@tail: parameter 0 has an aggregate kind of the other profile",
            "@big: parameter 0 has an aggregate kind of the other profile",
            "@big, inst4: parameter 0 has an aggregate kind of the other profile",
            "@big, inst4: parameter 2 has an aggregate kind of the other profile",
        ]
    );
    let post = parse(&arena, POST_ABI);
    assert_eq!(verify(&post, Profile::PostAbi), Vec::<String>::new());
    assert_eq!(verify(&post, Profile::PreAbi).len(), 4);
}

#[test]
fn aggregate_parameters_must_be_well_placed() {
    let cases: [(&str, &[&str]); 5] = [
        (
            "function @f(i64 aggregate(size 8, align 8)) external\n",
            &["@f: parameter 0 stands for an aggregate but is not a ptr"],
        ),
        (
            "function @f(i32, ptr result(size 8, align 8)) external\n",
            &[
                "@f: parameter 1 returns an aggregate but is not parameter 0 of a signature \
                 without a result",
            ],
        ),
        (
            "function @f(ptr result(size 8, align 8)) -> i32 external\n",
            &[
                "@f: parameter 0 returns an aggregate but is not parameter 0 of a signature \
                 without a result",
            ],
        ),
        (
            "function @f(ptr aggregate(size 6, align 2, [0: i16, 4: i32])) external\n",
            &["@f: parameter 0 has an aggregate scalar beyond the aggregate's size"],
        ),
        (
            "abi lowered\n\nfunction @f(ptr registers(ptr, align 8)) external\n",
            &["@f: parameter 0 returns a register of type i1 or ptr"],
        ),
    ];
    for (text, expected) in cases {
        let arena = Bump::new();
        let module = parse(&arena, text);
        let profile = module.profile();
        assert_eq!(verify(&module, profile), expected, "for\n{text}");
    }
}

#[test]
fn malformed_aggregate_descriptions_do_not_parse() {
    for (text, expected) in [
        (
            "function @f(ptr aggregate(align 8)) external\n",
            "1:27: expected `size`, found `align`",
        ),
        (
            "function @f(ptr aggregate(size 8, align 8, packed)) external\n",
            "1:44: expected `unaligned`, `flexible`, `first` or `[`, found `packed`",
        ),
        (
            "function @f(ptr registers(i64)) external\n",
            "1:30: expected `,`, found `)`",
        ),
        ("abi raised\n", "1:5: expected `lowered`, found `raised`"),
    ] {
        let arena = Bump::new();
        match parse_module(&arena, text) {
            | Ok(module) => panic!("parsed {text}as\n{module}"),
            | Err(error) => assert_eq!(error.to_string(), expected),
        }
    }
}

/// Transparent union arguments as System V lowering records them (#68):
/// the union's size and alignment with its first member's size, scalars
/// and, for a narrow integer or `_Bool` member, extension.
const FIRST_MEMBERS: &str = "\
function @floats(ptr aggregate(size 32, align 32, first(size 8), [0: f32, 4: f32])) external

function @pointer(ptr aggregate(size 16, align 16, first(size 8), [0: ptr])) external

function @signed(ptr aggregate(size 1, align 1, first(size 1, signed), [0: i8])) external

function @short(ptr aggregate(size 2, align 2, first(size 2, unsigned), [0: i16])) external

function @flag(ptr aggregate(size 1, align 1, first(size 1, bool), [0: i8])) external

function @empty(ptr aggregate(size 0, align 1, first(size 0))) external

function @call(ptr) external {
block0(v0: ptr):
    call_indirect v0(v0) : (ptr aggregate(size 1, align 1, first(size 1, signed), [0: i8]))
    return
}
";

#[test]
fn first_members_round_trip_and_verify() {
    assert_round_trip(FIRST_MEMBERS);
    let arena = Bump::new();
    let module = parse(&arena, FIRST_MEMBERS);
    assert_eq!(verify(&module, Profile::PreAbi), Vec::<String>::new());
}

#[test]
fn first_members_must_describe_their_union() {
    let misplaced = "@f: parameter 0 records a first member outside a pre-ABI aggregate argument";
    let size = "@f: parameter 0 has a first member larger than its aggregate";
    let outside = "@f: parameter 0 has an aggregate scalar beyond its first member's size";
    let extension =
        "@f: parameter 0 extends a first member that is not one narrow integer or _Bool scalar";
    for (text, expected) in [
        // A result, or a post-ABI copy, is the whole union.
        (
            "function @f(ptr result(size 8, align 8, first(size 8), [0: f64])) external\n",
            misplaced,
        ),
        (
            "abi lowered\nfunction @f(ptr byval(size 8, align 8, first(size 8))) external\n",
            misplaced,
        ),
        (
            "abi lowered\nfunction @f(ptr sret(size 8, align 8, first(size 8))) external\n",
            misplaced,
        ),
        (
            "abi lowered\nfunction @f(ptr indirect(size 8, align 8, first(size 8))) external\n",
            misplaced,
        ),
        // A valid first member must not bypass another layout invariant.
        (
            "function @f(ptr aggregate(size 8, align 8, complex, first(size 8), [0: f64])) \
             external\n",
            "@f: parameter 0 has an invalid complex scalar layout",
        ),
        // The member fits the union and holds its scalars.
        (
            "function @f(ptr aggregate(size 8, align 8, first(size 16), [0: f64])) external\n",
            size,
        ),
        (
            "function @f(ptr aggregate(size 0, align 1, first(size 1))) external\n",
            size,
        ),
        (
            "function @f(ptr aggregate(size 8, align 8, first(size 1), [0: f64])) external\n",
            outside,
        ),
        (
            "function @f(ptr aggregate(size 8, align 8, first(size 4), [0: i32, 4: i32])) \
             external\n",
            outside,
        ),
        // An extension is one narrow integer or `_Bool` scalar, all of it.
        (
            "function @f(ptr aggregate(size 4, align 4, first(size 4, signed), [0: i32])) \
             external\n",
            extension,
        ),
        (
            "function @f(ptr aggregate(size 4, align 4, first(size 4, signed), [0: f32])) \
             external\n",
            extension,
        ),
        (
            "function @f(ptr aggregate(size 2, align 1, first(size 2, signed), [0: i8, 1: i8])) \
             external\n",
            extension,
        ),
        (
            "function @f(ptr aggregate(size 2, align 2, first(size 2, signed), [1: i8])) \
             external\n",
            extension,
        ),
        (
            "function @f(ptr aggregate(size 2, align 2, first(size 2, bool), [0: i16])) external\n",
            extension,
        ),
        (
            "function @f(ptr aggregate(size 2, align 2, first(size 2, signed), [0: i8])) \
             external\n",
            extension,
        ),
        (
            "function @f(ptr aggregate(size 1, align 1, first(size 1, signext), [0: i8])) \
             external\n",
            extension,
        ),
        (
            "function @f(ptr aggregate(size 8, align 8, first(size 8, signed))) external\n",
            extension,
        ),
    ] {
        let arena = Bump::new();
        let module = parse(&arena, text);
        let profile = module.profile();
        assert_eq!(verify(&module, profile), [expected], "for\n{text}");
    }
}

#[test]
fn a_repeated_first_member_does_not_parse() {
    let text = "function @f(ptr aggregate(size 8, align 8, first(size 8), first(size 4), [0: \
                f64])) external\n";
    let arena = Bump::new();
    match parse_module(&arena, text) {
        | Ok(module) => panic!("parsed {text}as\n{module}"),
        | Err(error) => assert_eq!(error.to_string(), "1:59: `first` is defined twice"),
    }
}
