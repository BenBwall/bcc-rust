//! Constructors, destructors, aliases and weak symbols around hosted main.

use super::*;
use crate::{
    ir::{
        parse_module,
        verify_module,
    },
    util::bump::Bump,
};

/// Runs hosted main of `text` with constructors and destructors, returning
/// how it ended and what it printed.
fn run_program(text: &str) -> (Result<Termination, Trap>, String) {
    let arena = Bump::new();
    let module = parse_module(&arena, text).unwrap_or_else(|error| panic!("{error}\n{text}"));
    let errors: Vec<String> = verify_module(&module, module.profile(), &arena)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{text}");
    let mut host = DefaultHost::new_in(&arena);
    let result = run_main(&module, &mut host).map(|outcome| outcome.termination);
    (result, String::from_utf8_lossy(host.output()).into_owned())
}

/// A function printing `letter` with putchar.
fn printer(name: &str, letter: char) -> String {
    format!(
        "function @{name}() internal {{\nblock0:\n    v0 = iconst.i32 {}\n    v1 = call \
         @putchar(v0)\n    return\n}}\n",
        u32::from(letter)
    )
}

const PUTCHAR: &str = "function @putchar(i32) -> i32 external\nfunction @exit(i32) external\n";

#[test]
fn constructors_and_destructors_follow_priority_and_registration_order() {
    // Bundled Clang and GCC on both targets print "BCDAm" then "zwyx" for
    // this registration order (see the end-to-end attribute tests).
    let mut text = String::from(PUTCHAR);
    for (name, letter) in [
        ("c300", 'D'),
        ("c200", 'B'),
        ("cdef", 'A'),
        ("c200b", 'C'),
        ("d300", 'y'),
        ("d200", 'x'),
        ("ddef", 'z'),
        ("d200b", 'w'),
    ] {
        text.push_str(&printer(name, letter));
    }
    text.push_str(
        "constructor @c300 priority 300\nconstructor @c200 priority 200\nconstructor @cdef \
         priority 65535\nconstructor @c200b priority 200\ndestructor @d300 priority \
         300\ndestructor @d200 priority 200\ndestructor @ddef priority 65535\ndestructor @d200b \
         priority 200\n",
    );
    text.push_str(
        "function @main() -> i32 external {\nblock0:\n    v0 = iconst.i32 109\n    v1 = call \
         @putchar(v0)\n    v2 = iconst.i32 7\n    return v2\n}\n",
    );
    // Constructors sort ascending with ties in order, so c200, c200b, c300,
    // cdef: "BCDA"; destructors descending with ties reversed: ddef, d300,
    // d200b, d200: "zywx".
    let (result, output) = run_program(&text);
    assert_eq!(
        result,
        Ok(Termination::Returned(Some(RuntimeValue::Int(7))))
    );
    assert_eq!(output, "BCDAmzywx");
}

#[test]
fn exit_runs_destructors_and_abort_does_not() {
    let mut text = String::from(PUTCHAR);
    text.push_str("function @abort() external\n");
    text.push_str(&printer("bye", 'b'));
    text.push_str("destructor @bye priority 65535\n");
    let exiting = format!(
        "{text}function @main() -> i32 external {{\nblock0:\n    v0 = iconst.i32 3\n    call \
         @exit(v0)\n    unreachable\n}}\n"
    );
    let (result, output) = run_program(&exiting);
    assert_eq!(result, Ok(Termination::Exited(3)));
    assert_eq!(output, "b");
    let aborting = format!(
        "{text}function @main() -> i32 external {{\nblock0:\n    call @abort()\n    \
         unreachable\n}}\n"
    );
    let (result, output) = run_program(&aborting);
    assert_eq!(result, Ok(Termination::Aborted));
    assert_eq!(output, "");
}

#[test]
fn a_constructor_that_exits_still_runs_destructors_but_not_main() {
    let mut text = String::from(PUTCHAR);
    text.push_str(&printer("bye", 'b'));
    text.push_str(
        "function @early() internal {\nblock0:\n    v0 = iconst.i32 4\n    call @exit(v0)\n    \
         unreachable\n}\nconstructor @early priority 101\ndestructor @bye priority \
         65535\nfunction @main() -> i32 external {\nblock0:\n    v0 = iconst.i32 109\n    v1 = \
         call @putchar(v0)\n    v2 = iconst.i32 0\n    return v2\n}\n",
    );
    let (result, output) = run_program(&text);
    assert_eq!(result, Ok(Termination::Exited(4)));
    assert_eq!(output, "b");
}

#[test]
fn aliases_share_their_targets_address_and_body() {
    let text = "global @value external size 4, align 4 = bytes \"05000000\"\nglobal @other \
                external size 4, align 4 alias @value\nfunction @real(i32) -> i32 external \
                {\nblock0(v0: i32):\n    v1 = iconst.i32 3\n    v2 = imul.i32 v0, v1\n    return \
                v2\n}\nfunction @copy(i32) -> i32 external alias @real\nfunction @main() -> i32 \
                external {\nblock0:\n    v0 = global_addr @value\n    v1 = global_addr @other\n    \
                v2 = icmp.ptr eq v0, v1\n    v3 = iconst.i32 7\n    store.i32 v3, v1, align 4\n    \
                v4 = load.i32 v0, align 4\n    v5 = call @copy(v4)\n    v6 = func_addr @real\n    \
                v7 = func_addr @copy\n    v8 = icmp.ptr eq v6, v7\n    v9 = and.i1 v2, v8\n    \
                v10 = zext.i32 v9\n    v11 = iadd.i32 v5, v10\n    return v11\n}\n";
    let (result, _) = run_program(text);
    assert_eq!(
        result,
        Ok(Termination::Returned(Some(RuntimeValue::Int(22))))
    );
}

#[test]
fn undefined_weak_declarations_are_null_and_host_ones_resolve() {
    let text = "global @missing_object weak size 4, align 4\nfunction @missing(i32) -> i32 \
                weak\nfunction @putchar(i32) -> i32 weak\nfunction @main() -> i32 external \
                {\nblock0:\n    v0 = func_addr @missing\n    v1 = iconst.i64 0\n    v2 = \
                inttoptr.ptr v1\n    v3 = icmp.ptr eq v0, v2\n    v4 = global_addr \
                @missing_object\n    v5 = icmp.ptr eq v4, v2\n    v6 = func_addr @putchar\n    v7 \
                = icmp.ptr ne v6, v2\n    v8 = and.i1 v3, v5\n    v9 = and.i1 v8, v7\n    v10 = \
                zext.i32 v9\n    v11 = iconst.i32 33\n    v12 = call @putchar(v11)\n    return \
                v10\n}\n";
    let (result, output) = run_program(text);
    assert_eq!(
        result,
        Ok(Termination::Returned(Some(RuntimeValue::Int(1))))
    );
    assert_eq!(output, "!");
}

#[test]
fn calling_an_undefined_weak_function_is_undefined() {
    let text = "function @missing() -> i32 weak\nfunction @main() -> i32 external {\nblock0:\n    \
                v0 = call @missing()\n    return v0\n}\n";
    let (result, _) = run_program(text);
    assert!(
        matches!(
            result,
            Err(Trap::UndefinedBehavior(UbKind::CallThroughNonFunction, _))
        ),
        "{result:?}"
    );
}

#[test]
fn weak_definitions_run_their_own_body_in_a_single_module() {
    let text = "function @hook() -> i32 weak {\nblock0:\n    v0 = iconst.i32 9\n    return \
                v0\n}\nfunction @main() -> i32 external {\nblock0:\n    v0 = call @hook()\n    \
                return v0\n}\n";
    let (result, _) = run_program(text);
    assert_eq!(
        result,
        Ok(Termination::Returned(Some(RuntimeValue::Int(9))))
    );
}
