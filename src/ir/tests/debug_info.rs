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
