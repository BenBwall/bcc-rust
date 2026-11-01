//! C99 §6.7.8p14-15: strings initialize whole arrays, with optional braces.

use super::*;

fn source(element: &str, prefix: &str) -> String {
    format!(
        r#"
typedef __WCHAR_TYPE__ wchar_t;
typedef {element} character;
character global[] = {{ {prefix}"abc" }};
struct holder {{ character s[4]; }} global_holder = {{ {{ {prefix}"ab" }} }};
character global_matrix[2][3] = {{ {{ {prefix}"ab" }}, {prefix}"c" }};
int main(void) {{
    character a[] = {{ {prefix}"abc" }};
    static character b[5] = {{ {prefix}"ab" }};
    character exact[2] = {{ {prefix}"ab" }};
    struct holder x = {{ {{ {prefix}"ab" }} }};
    character m[2][3] = {{ {{ {prefix}"ab" }}, {prefix}"c" }};
    character plain[] = {prefix}"abc";
    wchar_t encoded[] = {{ L"\u00e4\U0001f642\x1234" }};
    const wchar_t *wide = L"ab";
    const char *narrow = "ab";
    if (encoded[0] != 0xe4 || wide[1] != 'b' || narrow[1] != 'b') return 2;
    if (sizeof(wchar_t) == 2) {{
        if (sizeof encoded != 10 || encoded[1] != 0xd83d || encoded[2] != 0xde42 ||
            encoded[3] != 0x1234 || encoded[4] != 0) return 3;
    }} else {{
        if (sizeof encoded != 16 || encoded[1] != 0x1f642 ||
            encoded[2] != 0x1234 || encoded[3] != 0) return 4;
    }}
    return sizeof a != 4 * sizeof(character) ||
        sizeof global != sizeof a || a[0] != 'a' || a[3] != 0 ||
        global[2] != 'c' || global[3] != 0 || b[1] != 'b' || b[4] != 0 ||
        exact[1] != 'b' || x.s[1] != 'b' || x.s[3] != 0 ||
        global_holder.s[1] != 'b' || global_holder.s[3] != 0 ||
        m[0][1] != 'b' || m[0][2] != 0 || m[1][0] != 'c' || m[1][2] != 0 ||
        global_matrix[0][1] != 'b' || global_matrix[1][0] != 'c' ||
        global_matrix[1][2] != 0 || plain[2] != 'c' || plain[3] != 0;
}}
"#
    )
}

const CHARACTERS: [(&str, &str); 4] = [
    ("char", ""),
    ("signed char", ""),
    ("unsigned char", ""),
    ("wchar_t", "L"),
];

#[test]
fn braced_strings_initialize_whole_arrays_in_all_stages() {
    for (element, prefix) in CHARACTERS {
        let source = source(element, prefix);
        for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
            for (optimize, profile) in STAGES {
                let actual = with_module(&source, target, optimize, profile, interpret);
                assert_eq!(actual.0, 0, "{element} {target:?} {optimize} {profile:?}");
            }
        }
    }
}

#[test]
fn braced_strings_match_clang_and_native_execution() {
    let Some(target) = host() else { return };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("braced-strings");
    for (index, (element, prefix)) in CHARACTERS.into_iter().enumerate() {
        let source = source(element, prefix);
        let input = directory.join(format!("case-{index}.c"));
        std::fs::write(&input, &source).unwrap();
        for opt in [OptLevel::O0, OptLevel::O2] {
            let options = LlvmOptions {
                target,
                opt: Some(opt),
                disable_llvm_passes: false,
                kind: OutputKind::Executable,
                code: crate::target::CodeOptions::default(),
                fast_math_runtime: false,
            };
            let clang = directory.join(format!("clang-{index}-{opt:?}.exe"));
            let output = toolchain.command(&input, &clang, options).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = run_native(&clang);
            assert_eq!(expected.0, 0, "Clang: {element}");
            for optimize in [false, true] {
                let executable = directory.join(format!("bcc-{index}-{opt:?}-{optimize}.exe"));
                with_module(&source, target, optimize, Profile::PostAbi, |module, _| {
                    compile_module(module, &executable, options).unwrap();
                });
                assert_eq!(
                    run_native(&executable),
                    expected,
                    "{element} {opt:?} {optimize}"
                );
            }
        }
    }
}

/// A string that is the sole braced initializer of an array whose elements
/// are not characters initializes the first element, as any other
/// expression would (C99 §6.7.8p17), including SQLite's `azEndings`:
/// static and automatic, incomplete and fixed, and nested arrays, with the
/// remaining elements zero.
const POINTER_ARRAYS: &str = r#"
static const char *file_endings[] = { "dll" };
const char *fixed[3] = { "so" };
const char *nested[][2] = { "a" };
const char *inner[][2] = { { "b" } };
struct entry { const char *name; int value; } entries[] = { "e" };
void *opaque[] = { "v" };
static int endings(void) {
    static const char *azEndings[] = {
       "dll"
    };
    return (int)(sizeof azEndings / sizeof azEndings[0]) * 100 + azEndings[0][1];
}
int main(void) {
    const char *automatic[] = { "auto" };
    const char *parenthesized[2] = { ("p") };
    char text[] = { "xy" };
    if (sizeof file_endings != sizeof(char *) || file_endings[0][2] != 'l') return 1;
    if (fixed[0][1] != 'o' || fixed[1] != 0 || fixed[2] != 0) return 2;
    if (sizeof nested != 2 * sizeof(char *) || nested[0][0][0] != 'a' || nested[0][1] != 0)
        return 3;
    if (sizeof inner != sizeof nested || inner[0][0][0] != 'b' || inner[0][1] != 0) return 4;
    if (sizeof entries != sizeof(struct entry) || entries[0].name[0] != 'e' ||
        entries[0].value != 0) return 5;
    if (((const char *)opaque[0])[0] != 'v') return 6;
    if (endings() != 208) return 7;
    if (sizeof automatic != sizeof(char *) || automatic[0][3] != 'o') return 8;
    if (parenthesized[0][0] != 'p' || parenthesized[1] != 0) return 9;
    if (sizeof text != 3 || text[1] != 'y') return 10;
    return 0;
}
"#;

#[test]
fn braced_strings_initialize_the_first_element_of_pointer_arrays() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            let actual = with_module(POINTER_ARRAYS, target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
        }
    }
    let Some(target) = host() else { return };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("braced-pointer-strings");
    let input = directory.join("pointers.c");
    std::fs::write(&input, POINTER_ARRAYS).unwrap();
    for opt in [OptLevel::O0, OptLevel::O2] {
        let options = LlvmOptions {
            target,
            opt: Some(opt),
            disable_llvm_passes: false,
            kind: OutputKind::Executable,
            code: crate::target::CodeOptions::default(),
            fast_math_runtime: false,
        };
        let clang = directory.join(format!("clang-{opt:?}.exe"));
        let output = toolchain.command(&input, &clang, options).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = run_native(&clang);
        assert_eq!(expected.0, 0, "Clang {opt:?}");
        for optimize in [false, true] {
            let executable = directory.join(format!("bcc-{opt:?}-{optimize}.exe"));
            with_module(
                POINTER_ARRAYS,
                target,
                optimize,
                Profile::PostAbi,
                |module, _| {
                    compile_module(module, &executable, options).unwrap();
                },
            );
            assert_eq!(run_native(&executable), expected, "{opt:?} {optimize}");
        }
    }
}
