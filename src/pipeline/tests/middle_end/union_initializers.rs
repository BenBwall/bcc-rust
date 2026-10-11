//! Union member replacement, exercised through C execution.

use super::*;

const SOURCE: &str = r"
union U { long long old; struct { int x, y; } next; };
struct S { int before; union U u[2]; int after; };
union U global = { .old = -1, .next.x = 1 };
struct S nested = { .before = 7, .u[0].old = -1, .u[1].old = -1,
    .u[0].next.x = 2, .u[1].next.y = 3, .u[0].next.y = 4, .after = 8 };
union U back = { .next.x = 9, .next.y = 10, .old = -1, .next.y = 11 };
union V { union U a; long long b; };
union V overlap = { .a.next.x = 1, .a.next.y = 2, .b = -1, .a.next.y = 3 };
int main(void) {
    union U local = { .old = -1, .next.x = 1 };
    struct S automatic = { .before = 7, .u[0].old = -1, .u[1].old = -1,
        .u[0].next.x = 2, .u[1].next.y = 3, .u[0].next.y = 4, .after = 8 };
    union U again = { .next.x = 9, .next.y = 10, .old = -1, .next.y = 11 };
    union V overlapping = { .a.next.x = 1, .a.next.y = 2, .b = -1, .a.next.y = 3 };
    union U copied = { .old = -1 };
    struct { union U u; } holder = { .u.next.x = 9, .u = copied, .u.next.x = 2 };
    if (global.next.x != 1 || global.next.y != 0) return 1;
    if (local.next.x != 1 || local.next.y != 0) return 2;
    if (nested.before != 7 || nested.after != 8 || nested.u[0].next.x != 2 ||
        nested.u[0].next.y != 4 || nested.u[1].next.x != 0 ||
        nested.u[1].next.y != 3) return 3;
    if (automatic.before != 7 || automatic.after != 8 ||
        automatic.u[0].next.x != 2 || automatic.u[0].next.y != 4 ||
        automatic.u[1].next.x != 0 || automatic.u[1].next.y != 3) return 4;
    if (back.next.x != 0 || back.next.y != 11 ||
        again.next.x != 0 || again.next.y != 11) return 5;
    if (overlap.a.next.x != 0 || overlap.a.next.y != 3 ||
        overlapping.a.next.x != 0 || overlapping.a.next.y != 3) return 6;
    if (holder.u.next.x != 2 || holder.u.next.y != -1) return 7;
    return 0;
}
";

#[test]
fn changing_union_members_zeroes_the_new_member_in_all_stages() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            let actual = with_module(SOURCE, target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
        }
    }
}

#[test]
fn changing_union_members_matches_native_execution() {
    let Some(target) = host() else {
        return;
    };
    let _toolchain = Toolchain::bundled(target).expect("the bundled LLVM toolchain");
    let directory = TempDir::new("union-member-replacement");
    let executable = directory.join("union-switch.exe");
    with_module(SOURCE, target, false, Profile::PostAbi, |module, _| {
        compile_module(
            module,
            &executable,
            LlvmOptions {
                opt: Some(OptLevel::O0),
                disable_llvm_passes: false,
                target,
                kind: OutputKind::Executable,
            },
        )
        .unwrap();
    });
    assert_eq!(run_native(&executable).0, 0);
}
