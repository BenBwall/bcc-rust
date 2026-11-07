//! The debug tables and instruction locations: the located text round-trips,
//! the plain text leaves them out, and the verifier and parser refuse a
//! location that leads nowhere.

use super::*;

const LOCATED: &str = "\
debug file file0 @a.c
debug file file1 @\"dir/in c.h\"
debug unit file0 language c99 level 2 dwarf 4 optimized
debug scope scope0 @inner name @inner file file1 line 3 scope_line 4
debug scope scope1 @outer name @outer file file0 line 7 scope_line 8
debug loc loc0 file1 line 5 column 3 scope scope0
debug loc loc1 file0 line 9 column 10 scope scope1
debug loc loc2 file1 line 5 column 3 scope scope0 at loc1

function @inner(i32) -> i32 internal {
block0(v0: i32):
    return v0 dbg loc0
}

function @outer(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0 dbg loc2
    v2 = call @inner(v1) dbg loc1
    return v2
}
";

const PLAIN: &str = "\
function @inner(i32) -> i32 internal {
block0(v0: i32):
    return v0
}

function @outer(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    v2 = call @inner(v1)
    return v2
}
";

#[test]
fn the_located_form_round_trips_and_the_plain_form_leaves_it_out() {
    let arena = Bump::new();
    let module = parse(&arena, LOCATED);
    assert_eq!(verify(&module, Profile::PreAbi), Vec::<String>::new());
    pretty_assertions::assert_eq!(module.display_located().to_string(), LOCATED);
    pretty_assertions::assert_eq!(module.to_string(), PLAIN);
}

#[test]
fn a_module_without_debug_information_prints_the_same_in_both_forms() {
    let arena = Bump::new();
    let module = parse(&arena, PLAIN);
    assert!(
        module
            .function(module.functions().next().unwrap().0)
            .body
            .as_ref()
            .is_some_and(|body| !body.has_locations())
    );
    pretty_assertions::assert_eq!(module.display_located().to_string(), PLAIN);
}

#[test]
fn the_tables_parse_after_the_target_and_before_the_functions() {
    let text = format!(
        "target triple = \"x86_64-unknown-linux-gnu\"\ntarget datalayout = \"\"\n\n{LOCATED}"
    );
    let arena = Bump::new();
    let module = parse(&arena, &text);
    pretty_assertions::assert_eq!(module.display_located().to_string(), text);
}

/// A table of each kind `-g` fills in, with a structure that points at
/// itself, which is why a type may name a later type.
const DATA: &str = "\
debug file file0 @a.c
debug unit file0 language c11 level 2
debug type type0 base @int size 32 encoding signed
debug type type1 pointer type2
debug type type2 struct @node file file0 line 3 size 128 align 64 members (@next type1 line 3 \
                    offset 0, anonymous type5 line 4 offset 64, @bits type0 line 5 offset 96 bits \
                    3)
debug type type3 union anonymous file file0 line 6 size 0 align 0 incomplete
debug type type4 const type0
debug type type5 struct anonymous file file0 line 4 size 32 align 32 members ()
debug type type6 array type4 count 3 size 96 align 32
debug type type7 array type0 size 0 align 32
debug type type8 vector type0 count 4 size 128 align 128
debug type type9 enum @color file file0 line 7 size 32 align 32 base type0 values (@RED 0, @MINUS \
                    -1)
debug type type10 typedef @my_t type type0 file file0 line 8
debug type type11 function type10 (type0, type1) variadic prototyped
debug type type12 pointer void
debug type type13 atomic void
debug scope scope0 @f name @f file file0 line 9 scope_line 10 type type11
debug scope scope1 block scope0 @f file file0 line 11 column 3
debug variable var0 @x scope scope0 file file0 line 9 type type10 argument 1
debug variable var1 @y scope scope1 file file0 line 12 type type2
debug variable var2 @unknown scope scope1 file file0 line 13 type void
debug global @shared name @shared file file0 line 1 type type4
debug global @later name @later file file0 line 14 type type12 scope scope1
debug loc loc0 file0 line 11 column 3 scope scope0
debug loc loc1 file0 line 12 column 5 scope scope1

global @shared external size 4, align 4
global @later internal size 8, align 8

function @f(i32) -> i32 external {
    slot0 = stack_slot 16, align 8
    debug declare var0 v0 loc0
    debug declare var1 slot0 loc1
block0(v0: i32):
    return v0 dbg loc0
}
";

#[test]
fn the_data_tables_round_trip_and_verify() {
    let arena = Bump::new();
    let module = parse(&arena, DATA);
    assert_eq!(verify(&module, Profile::PreAbi), Vec::<String>::new());
    pretty_assertions::assert_eq!(module.display_located().to_string(), DATA);
}

#[test]
fn a_missing_type_or_place_does_not_verify() {
    for text in [
        DATA.replace(
            "debug type type1 pointer type2",
            "debug type type1 pointer type99",
        ),
        DATA.replace("type type10 argument 1", "type type99 argument 1"),
        DATA.replace("debug declare var1 slot0", "debug declare var1 slot5"),
        DATA.replace("debug declare var0 v0", "debug declare var0 v9"),
    ] {
        let arena = Bump::new();
        let module = parse(&arena, &text);
        assert!(!verify(&module, Profile::PreAbi).is_empty(), "{text}");
    }
}

#[test]
fn a_location_from_another_function_does_not_verify() {
    let text = LOCATED.replace(
        "v2 = call @inner(v1) dbg loc1",
        "v2 = call @inner(v1) dbg loc0",
    );
    let arena = Bump::new();
    let module = parse(&arena, &text);
    let errors = verify(&module, Profile::PreAbi);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("debug location"), "{errors:?}");
}

#[test]
fn a_table_entry_out_of_order_or_a_location_not_listed_does_not_parse() {
    for text in [
        "debug file file1 @a.c\n".to_owned(),
        LOCATED.replace("debug loc loc1", "debug loc loc7"),
        LOCATED.replace("dbg loc2", "dbg loc9"),
        LOCATED.replace(
            "line 5 column 3 scope scope0\n",
            "line 5 column 3 scope scope9\n",
        ),
        LOCATED.replace("language c99", "language pascal"),
        LOCATED.replace("level 2", "level 4"),
    ] {
        let arena = Bump::new();
        assert!(parse_module(&arena, &text).is_err(), "{text}");
    }
}
