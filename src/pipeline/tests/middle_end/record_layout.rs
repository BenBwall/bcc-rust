//! Record layouts observed through execution: the deterministic programs
//! `scripts/layout_fuzz.py --fixture` generated, whose expected lines are the
//! records on which the target's references (MinGW GCC, `cl.exe` or Linux
//! GCC, and Clang) agree, and the Microsoft zero-width bit-field reproducer
//! of #55. Each runs in the interpreter for its target, and natively on the
//! host.

use super::*;

const FIXTURES: [(Target, &str, &str); 3] = [
    (
        Target::WindowsGnu,
        include_str!("../../../../tests/fixtures/layout/layout_gnu.c"),
        include_str!("../../../../tests/fixtures/layout/layout_gnu.expected"),
    ),
    (
        Target::WindowsMsvc,
        include_str!("../../../../tests/fixtures/layout/layout_msvc.c"),
        include_str!("../../../../tests/fixtures/layout/layout_msvc.expected"),
    ),
    (
        Target::LinuxGnu,
        include_str!("../../../../tests/fixtures/layout/layout_linux.c"),
        include_str!("../../../../tests/fixtures/layout/layout_linux.expected"),
    ),
];

/// Every expected line appears in `output`; a record line starts with its
/// name, so a missing one names the record that changed.
fn assert_records(output: &str, expected: &str, what: &str) {
    for line in expected.lines() {
        assert!(
            output.lines().any(|actual| actual == line),
            "{what}: {}\nexpected {line}\nactual   {}",
            line.split(' ').next().unwrap_or_default(),
            output
                .lines()
                .find(|actual| actual.split(' ').next() == line.split(' ').next())
                .unwrap_or("(missing)")
        );
    }
}

#[test]
fn fuzzed_record_layouts_match_the_platform_compilers_in_the_interpreter() {
    for (target, source, expected) in FIXTURES {
        let (status, output) = with_module(source, target, false, Profile::PreAbi, interpret);
        assert_eq!(status, 0, "{target:?}");
        assert_records(&output, expected, &format!("{target:?}"));
    }
}

#[test]
fn fuzzed_record_layouts_match_the_platform_compilers_natively() {
    let Some(target) = host() else { return };
    let Some(&(_, source, expected)) = FIXTURES.iter().find(|(t, ..)| *t == target) else {
        return;
    };
    let _toolchain = Toolchain::bundled(target).expect("the bundled LLVM toolchain");
    let directory = TempDir::new("record-layout");
    let executable = directory.join("layout.exe");
    with_module(source, target, false, Profile::PostAbi, |module, _| {
        compile_module(
            module,
            &executable,
            LlvmOptions {
                opt: Some(OptLevel::O0),
                disable_llvm_passes: false,
                target,
                kind: OutputKind::Executable,
                code: crate::target::CodeOptions::default(),
                fast_math_runtime: false,
            },
        )
        .unwrap();
    });
    let (status, output) = run_native(&executable);
    assert_eq!(status, 0);
    assert_records(&output, expected, "native");
}

/// The #55 reproducer: each record's size and the first byte a store to the
/// member after the zero-width bit-field sets. MinGW GCC and `cl.exe` print
/// these on their targets; Clang's MinGW layout differs on P1 and A1 (it
/// overlaps their fields).
const ZERO_WIDTH: &str = r"
int putchar(int);
struct N1 { int a : 5; short : 0; char b : 6; };
struct N2 { long long a : 15; unsigned char : 0; int b; };
#pragma pack(push, 1)
struct P1 { char a; unsigned b : 18; unsigned : 0; unsigned c : 4; };
struct P2 { long long a : 27; char : 0; };
#pragma pack(4)
struct P3 { long long a : 9; _Bool : 0; short b; };
#pragma pack(pop)
static void number(int n) { if (n >= 10) number(n / 10); putchar('0' + n % 10); }
static void show(const void *p, int size) {
    const unsigned char *bytes = p;
    int at = -1;
    for (int i = 0; i < size; i++) if (bytes[i]) { at = i; break; }
    number(size); putchar(' '); number(at); putchar('\n');
}
#define FIRST(T, f) { struct T s = {0}; s.f = 1; show(&s, (int)sizeof s); }
int main(void) {
    FIRST(N1, b) FIRST(N2, b) FIRST(P1, c) FIRST(P2, a) FIRST(P3, b)
    return 0;
}
";

#[test]
fn zero_width_bit_fields_end_the_microsoft_unit_at_run_time() {
    let expected = "8 4\n16 8\n9 5\n8 0\n12 8\n";
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            let actual = with_module(ZERO_WIDTH, target, optimize, profile, interpret);
            assert_eq!(actual, (0, expected.to_owned()), "{target:?} {profile:?}");
        }
    }
    let Some(target @ (Target::WindowsGnu | Target::WindowsMsvc)) = host() else {
        return;
    };
    let _toolchain = Toolchain::bundled(target).expect("the bundled LLVM toolchain");
    let directory = TempDir::new("zero-width-bit-fields");
    for arm in ARMS {
        let executable = directory.join(format!("zero-width-{}.exe", arm.name));
        with_module(
            ZERO_WIDTH,
            target,
            arm.bcc,
            Profile::PostAbi,
            |module, _| {
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
            },
        );
        assert_eq!(
            run_native(&executable),
            (0, expected.to_owned()),
            "{}",
            arm.name
        );
    }
}
