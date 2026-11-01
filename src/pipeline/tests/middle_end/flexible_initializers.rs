//! Static objects whose initializer gives their flexible array member
//! elements (GNU extension, #59): the object grows past its structure while
//! `sizeof` stays the structure's. Checked in every interpreter stage on each
//! target and natively on the host; the expected values are Clang's and
//! GCC's. Empty lists may initialize any flexible array member, as in Clang.
//! A later list or string for the member replaces the earlier ones, and the
//! object ends with the last one's extent (#68).
//! A thread-local object has global storage too, so Clang accepts its
//! flexible array initializer as well.

use super::*;

const SOURCE: &str = r#"
struct F { int n; int v[]; };
static struct F f = { 3, { 1, 2, 3 } };
static const struct { short len; char text[]; } msg = { 5, "hello" };
struct H { char c; long long v[]; };
struct H h = { 1, { 2, 3 } };
static struct F elided = { 1, 2, 3, 4 };
static struct F designated = { .v = { [3] = 9, [1] = 8 }, .n = 2 };
static struct F empty = { 1, {} };
static _Thread_local struct F per_thread = { 2, { 5, 6 } };
static struct F twice = { .v = { 1, 2, 3 }, .n = 1, .v = { 4 } };
static struct F longer = { .v = { 1 }, .n = 1, .v = { 4, 5, 6 } };
static struct F elided_after = { .v = { 1, 2, 3 }, .n = 1, 4 };
static struct S { int n; char t[]; } shorter = { .t = "hello", .t = "x" };
struct G { struct F f; int after; } nested = { { 1, {} }, 2 };
struct F *literal = &(struct F){ 4, {} };
struct Q { int n; struct { short a, b; } v[]; };
static struct Q elided_pairs = { 1, 2, 3 };
static struct Q braced_pairs = { 1, { { 4, 5 }, 6, 7, [3].b = 8 } };
static int automatic(void) { struct F a = { 1, {} }; struct F b = { .v = {}, .n = 2 }; return a.n + b.n; }
int g = 5;
struct P { int *p; int *list[]; } p = { &g, { &g, 0, &f.n } };
static int sum(const int *v, int n) { int s = 0; for (int i = 0; i < n; i++) s += v[i]; return s; }
int main(void) {
    static struct F local = { 7, { 70, 71 } };
    if (sizeof f != sizeof(struct F) || sizeof h != 8 || sizeof msg != 2) return 1;
    if (f.n != 3 || sum(f.v, 3) != 6) return 2;
    if (msg.len != 5 || msg.text[4] != 'o' || msg.text[5] != 0) return 3;
    if (h.c != 1 || h.v[0] != 2 || h.v[1] != 3) return 4;
    if (elided.n != 1 || sum(elided.v, 3) != 9) return 5;
    if (designated.n != 2 || designated.v[0] || designated.v[1] != 8 || designated.v[3] != 9) return 6;
    if (empty.n != 1 || *p.p != 5 || p.list[0] != &g || p.list[1] || p.list[2] != &f.n) return 7;
    if (local.n != 7 || local.v[1] != 71) return 8;
    if (twice.n != 1 || twice.v[0] != 4) return 9;
    if (nested.f.n != 1 || nested.after != 2 || literal->n != 4 || automatic() != 3) return 10;
    if (elided_pairs.v[0].a != 2 || elided_pairs.v[0].b != 3) return 11;
    if (braced_pairs.v[0].b != 5 || braced_pairs.v[1].a != 6 || braced_pairs.v[1].b != 7) return 12;
    if (braced_pairs.v[2].a || braced_pairs.v[3].b != 8) return 13;
    if (per_thread.n != 2 || sum(per_thread.v, 2) != 11) return 14;
    if (longer.v[0] != 4 || longer.v[2] != 6) return 15;
    if (elided_after.v[0] != 4 || elided_after.v[1] != 2 || elided_after.v[2] != 3) return 16;
    if (shorter.t[0] != 'x' || shorter.t[1]) return 17;
    return 0;
}
"#;

#[test]
fn flexible_array_initializers_extend_static_objects_in_all_stages() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            let actual = with_module(SOURCE, target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
        }
    }
}

#[test]
fn the_last_flexible_array_member_initializer_sets_the_storage() {
    // Clang 23: `twice` takes 8 bytes and `shorter` 6; brace elision after a
    // list replaces only the elements it reaches (`elided_after`, which
    // Clang 23 crashes on, keeps the list's three as for a fixed array).
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        with_module(SOURCE, target, false, Profile::PreAbi, |module, _| {
            let size = |name: &str| {
                module
                    .globals()
                    .find(|(_, global)| global.name == name)
                    .map(|(_, global)| global.size)
            };
            assert_eq!(size("twice"), Some(8), "{module}");
            assert_eq!(size("longer"), Some(16), "{module}");
            assert_eq!(size("elided_after"), Some(16), "{module}");
            assert_eq!(size("shorter"), Some(6), "{module}");
        });
    }
}

#[test]
fn flexible_array_initializers_match_native_execution() {
    let Some(target) = host() else { return };
    let _toolchain = Toolchain::bundled(target).expect("the bundled LLVM toolchain");
    let directory = TempDir::new("flexible-initializers");
    for arm in ARMS {
        let executable = directory.join(format!("flexible-{}.exe", arm.name));
        with_module(SOURCE, target, arm.bcc, Profile::PostAbi, |module, _| {
            compile_module(
                module,
                &executable,
                LlvmOptions {
                    opt: Some(arm.opt),
                    disable_llvm_passes: arm.disable_llvm_passes,
                    target,
                    kind: OutputKind::Executable,
                    code: crate::target::CodeOptions::default(),
                    fast_math_runtime: false,
                },
            )
            .unwrap();
        });
        assert_eq!(run_native(&executable).0, 0, "{}", arm.name);
    }
}

/// A flexible array member of a structure rvalue (a call, comma or
/// assignment result) designates an array at its offset in the temporary;
/// it decays without a size to copy (gcc.c-torture/compile/pr16566-*.c).
/// The GNU nested form in `touch` is only lowered, since the temporary has
/// no elements to access.
/// C99: §6.7.2.1p16, p. 103; PDF p. 115; §6.5.2.3p3, p. 73; PDF p. 85.
const RVALUE_MEMBERS: &str = "
struct A { int n; int x[]; };
struct C { int i; struct S *tab[]; };
struct S { struct C c; };
static struct A make(int n) { struct A a = { n }; return a; }
static void touch(struct S *x) {
    if (x) { touch(((void)1, x->c).tab[0]); ((void)1, x->c).tab[0] = 0; }
}
int main(void) {
    struct A a = { 3 }, b = { 5 };
    struct A *p = &b;
    if (((void)0, a).x == 0 || make(7).x == 0 || (*p = a).x == 0) return 1;
    if (((void)0, a).n != 3 || make(7).n != 7 || p->n != 3) return 2;
    touch(0);
    return 0;
}
";

#[test]
fn flexible_array_members_of_rvalues_decay_in_all_stages() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            let actual = with_module(RVALUE_MEMBERS, target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
        }
    }
}
