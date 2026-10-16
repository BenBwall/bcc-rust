//! GNU `transparent_union` arguments (#60): a prototyped call passes an
//! argument of a member's type as the union, with that member holding the
//! value. Checked in every interpreter stage on each target and natively on
//! the host; the expected values are Clang's and GCC's.

use super::*;

const SOURCE: &str = r"
typedef union { int *ip; const long *lp; } __attribute__((transparent_union)) arg_t;
static int take(arg_t a) { return *a.ip; }
typedef union { int *ip; long *lp; } wait_t __attribute__((transparent_union));
static long wait_status(wait_t w) { return w.ip ? *w.lp : -1; }
union number { unsigned u; int i; } __attribute__((__transparent_union__));
static unsigned number(union number n) { return n.u; }
struct pair { int a, b; };
union record { struct pair *p; struct pair s; } __attribute__((transparent_union));
static int record(int which, union record r) { return which ? r.s.b : r.p->a; }
typedef int __attribute__((may_alias)) aliasing_int;
int main(void) {
    int v = 9;
    long l = 42;
    void *vp = &v;
    if (take(&v) != 9 || take(vp) != 9) return 1;
    arg_t whole = { &v };
    if (take(whole) != 9) return 2;
    if (wait_status(&l) != 42 || wait_status(0) != -1) return 3;
    if (number(7) != 7 || number(-1) != 0xffffffffu) return 4;
    struct pair pair = { 5, 6 };
    if (record(0, &pair) != 5 || record(1, pair) != 6) return 5;
    float f = 1.0f;
    aliasing_int *alias = (aliasing_int *)&f;
    if (*alias != 0x3f800000) return 6;
    return 0;
}
";

#[test]
fn transparent_union_arguments_in_all_stages() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            let actual = with_module(SOURCE, target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
        }
    }
}

#[test]
fn transparent_union_arguments_match_native_execution() {
    let Some(target) = host() else { return };
    let _toolchain = Toolchain::bundled(target).expect("the bundled LLVM toolchain");
    let directory = TempDir::new("transparent-unions");
    for arm in ARMS {
        let executable = directory.join(format!("transparent-{}.exe", arm.name));
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
                },
            )
            .unwrap();
        });
        assert_eq!(run_native(&executable).0, 0, "{}", arm.name);
    }
}
