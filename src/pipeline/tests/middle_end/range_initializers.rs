//! GNU ranges whose later elements copy the first (issue #82), exercised
//! through C execution: each array checks its own elements, and each range
//! value is evaluated once.

use super::*;

const SOURCE: &str = r#"
int calls;
static int next(void) { return ++calls * 3; }
struct S { int x, y; };
static struct S make(void) { struct S s = { calls++, 7 }; return s; }
struct B { unsigned a:3, b:5, c:24; };
int main(void) {
    int a[40] = { [0 ... 39] = next(), [5] = next() };
    int b[6][20] = { [1 ... 4][2 ... 19] = next() };
    struct S c[20] = { [0 ... 19] = make(), [3].y = 1 };
    struct S d[20] = { [0 ... 19] = { next() }, [7].x = 4 };
    char s[20][4] = { [0 ... 19] = "ab", [4] = "c" };
    struct B f[20] = { [0 ... 19] = { 1, 2, next() }, [9].b = 3 };
    long z[100][3] = { [0 ... 99][0 ... 2] = 0, [50][1] = 1 };
    long g[3][30] = { [0 ... 2] = { [0 ... 29] = next() }, [1][4] = 2 };
    double h[64] = { [0 ... 63] = 1.5, [2] = 0 };
    if (calls != 7) return 1;
    for (int i = 0; i < 40; i++) if (a[i] != (i == 5 ? 6 : 3)) return 2;
    for (int i = 0; i < 6; i++)
        for (int j = 0; j < 20; j++)
            if (b[i][j] != (i >= 1 && i <= 4 && j >= 2 ? 9 : 0)) return 3;
    for (int i = 0; i < 20; i++) if (c[i].x != 3 || c[i].y != (i == 3 ? 1 : 7)) return 4;
    for (int i = 0; i < 20; i++) if (d[i].x != (i == 7 ? 4 : 15) || d[i].y) return 5;
    for (int i = 0; i < 20; i++)
        if (s[i][0] != (i == 4 ? 'c' : 'a') || s[i][1] != (i == 4 ? 0 : 'b') || s[i][2] || s[i][3])
            return 6;
    for (int i = 0; i < 20; i++) if (f[i].a != 1 || f[i].b != (i == 9 ? 3 : 2) || f[i].c != 18) return 7;
    for (int i = 0; i < 100; i++)
        for (int j = 0; j < 3; j++)
            if (z[i][j] != (i == 50 && j == 1)) return 8;
    for (int i = 0; i < 3; i++)
        for (int j = 0; j < 30; j++)
            if (g[i][j] != (i == 1 && j == 4 ? 2 : 21)) return 9;
    for (int i = 0; i < 64; i++) if (h[i] != (i == 2 ? 0 : 1.5)) return 10;
    return 0;
}
"#;

#[test]
fn range_copies_store_every_element_in_all_stages() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            let actual = with_module(SOURCE, target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
        }
    }
}

#[test]
fn range_copies_match_native_execution() {
    let Some(target) = host() else {
        return;
    };
    let _toolchain = Toolchain::bundled(target).expect("the bundled LLVM toolchain");
    let directory = TempDir::new("range-copies");
    for arm in ARMS {
        let executable = directory.join(format!("range-copies-{}.exe", arm.name));
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
