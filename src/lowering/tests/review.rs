//! Executing regressions for the PR #39 initializer review.
use super::*;
use crate::backend::interpreter::{
    Outcome,
    RuntimeValue,
    Termination,
    run_text,
};

fn check(source: &str) {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let text = lower_for(source, target).unwrap();
        let result = run_text(&text, "main");
        assert!(
            matches!(
                result.result,
                Ok(Outcome {
                    termination: Termination::Returned(Some(RuntimeValue::Int(0))),
                    ..
                })
            ),
            "{target:?}: {result:?}"
        );
    }
}

#[test]
fn repeated_braced_subobject_clears_implicit_members() {
    check(
        "struct S { int a[2]; int b; }; static struct S s = { .a = {1, 2}, .b = 7, .a = {3} }; \
         int main(void) { struct S t = { .a = {1, 2}, .b = 7, .a = {3} }; return s.a[1] != 0 || \
         t.a[1] != 0 || s.a[0] != 3 || t.a[0] != 3 || s.b != 7 || t.b != 7; }",
    );
}

#[test]
fn nested_designators_restart_for_each_braced_list() {
    check(
        "static int s[2][3] = {{[1] = 1, 3}, {[1] = 2, 4}}; struct S { int a, b; }; static struct \
         S t[2] = {{.b = 5}, {.b = 6}}; int main(void) { int a[2][3] = {{[1] = 1, 3}, {[1] = 2, \
         4}}; struct S u[2] = {{.b = 5}, {.b = 6}}; return s[0][1] != 1 || s[1][1] != 2 || \
         s[1][0] != 0 || s[0][2] != 3 || s[1][2] != 4 || a[0][1] != 1 || a[1][1] != 2 || a[1][0] \
         != 0 || a[0][2] != 3 || a[1][2] != 4 || t[1].b != 6 || t[1].a != 0 || u[1].b != 6 || \
         u[1].a != 0; }",
    );
}

#[test]
fn static_unsigned_integers_convert_by_magnitude() {
    check(
        "double x = (unsigned __int128)1 << 127; float f = 0xffffffffffffffffULL; double d = \
         0xffffffffffffffffULL; double neg = -42; int main(void) { return x < 0 || x != 0x1p127 \
         || f != 0x1p64f || d != 0x1p64 || neg != -42; }",
    );
}

#[test]
fn static_unsigned_long_long_converts_exactly_to_x87() {
    let text = lower_for("long double l = 0xffffffffffffffffULL;", Target::LinuxGnu).unwrap();
    assert!(text.contains("ffffffffffffffff3e40"), "{text}");
}

#[test]
fn static_overwrites_replace_pointer_relocations() {
    check(
        "int a, b; int *p[1] = { [0] = &a, [0] = 0 }; int *q[1] = { [0] = &a, [0] = &b }; union U \
         { int *p; unsigned char bytes[8]; }; union U u = { .p = &a, .bytes[1] = 0 }; int \
         main(void) { for (int i = 0; i < 8; i++) if (u.bytes[i]) return 3; return p[0] != 0 || \
         q[0] != &b; }",
    );
}
