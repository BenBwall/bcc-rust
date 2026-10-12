//! Compound updates must not straddle evaluation of a called function.

use super::*;

const PROGRAMS: &[(&str, &str)] = &[
    (
        "memory",
        "unsigned x[1] = {2}; unsigned f(void) { x[0] |= 128; return 1; } int main(void) { x[0] \
         |= f(); return x[0] != 131; }",
    ),
    (
        "scalar",
        "int x = 2; int f(void) { x = 130; return 1; } int main(void) { return (x += f()) != 131 \
         || x != 131; }",
    ),
    (
        "bit_field",
        "struct S { unsigned x:8; } s = {2}; unsigned f(void) { s.x |= 128; return 1; } int \
         main(void) { s.x |= f(); return s.x != 131; }",
    ),
    (
        "lvalue_once",
        "int x = 2, calls; int *p(void) { ++calls; return &x; } int f(void) { x = 130; return 1; \
         } int main(void) { *p() += f(); return x != 131 || calls != 1; }",
    ),
];

#[test]
fn compound_updates_follow_rhs_calls_in_all_stages() {
    for &(name, source) in PROGRAMS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                let actual = with_module(source, target, optimize, profile, interpret);
                assert_eq!(actual.0, 0, "{name} {target:?} {optimize} {profile:?}");
            }
        }
    }
}

#[test]
fn compound_updates_follow_rhs_calls_in_native_arms() {
    let Some(target) = host() else { return };
    let directory = TempDir::new("compound-updates");
    for &(name, source) in PROGRAMS {
        for arm in ARMS {
            let exe = directory.join(format!("{name}-{}.exe", arm.name));
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &exe,
                    LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Executable,
                    },
                )
                .expect("compile the compound update regression");
            });
            assert_eq!(run_native(&exe).0, 0, "{name} {}", arm.name);
        }
    }
}
