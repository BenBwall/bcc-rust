//! Static function addresses must agree with the pinned Clang reference.
#![cfg(target_os = "linux")]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Integration tests own source files and process output."
)]

use std::{
    fs,
    path::PathBuf,
    process::Command,
};

fn clang() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/llvm/bin/clang")
}

fn checked(command: &mut Command) -> String {
    let output = crate::native::run_native(command).unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn static_function_designators_match_clang_relocations() {
    let root = std::env::temp_dir().join(format!("bcc-static-designators-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("addresses.c");
    // GNU fortify wrappers use a private always-inline body for calls, but
    // their address must keep naming the externally supplied function.
    fs::write(
        &source,
        r"
int f(int);
extern inline __attribute__((gnu_inline, always_inline))
int f(int x) { return x + 1; }
int g(int);
int (*fp)(int);
int (*addresses[])(int) = {
 f, &f, *f, **f, &*f, &**f, ***&f,
 1 ? f : 0, 0 ? 0 : f, 1 ? *f : g, 0 ? f : **g,
 *(1 ? f : g), **(0 ? f : g), 1 ? (0 ? g : **f) : g,
 0.5 ? f : g, -0.0 ? f : g, 1 ? f : (int (*)(int))0,
 1 ? f : (int (*)(int))1, 0 ? (int (*)(int))1 : g,
 1 ? f : fp, 0 ? fp : f, 1 ? 0 : f
};
int object;
int *object_address = 1 ? &object : 0;
",
    )
    .unwrap();
    for (compiler, bcc) in [("bcc", true), ("bcc-native", true), ("clang", false)] {
        let object = root.join(format!("{compiler}.o"));
        let mut command = Command::new(if bcc {
            PathBuf::from(env!("CARGO_BIN_EXE_bcc-rust"))
        } else {
            clang()
        });
        _ = command
            .args(["-std=c99", "--target=x86_64-unknown-linux-gnu", "-c"])
            .arg(&source)
            .arg("-o")
            .arg(&object);
        if compiler == "bcc-native" {
            _ = command.args(["--backend=native", "--native-object"]);
        }
        drop(checked(&mut command));
        let relocations = checked(Command::new("readelf").arg("-rW").arg(&object));
        // Every non-null entry is an absolute, zero-addend function relocation.
        let symbols: Vec<_> = relocations
            .lines()
            .filter(|line| line.contains("R_X86_64_64"))
            .map(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                assert_eq!(fields[5..], ["+", "0"], "{relocations}");
                fields[4].to_owned()
            })
            .collect();
        assert_eq!(
            symbols,
            [
                "f", "f", "f", "f", "f", "f", "f", "f", "f", "f", "g", "f", "g", "f", "f", "g",
                "f", "f", "g", "f", "f", "object"
            ],
            "{compiler}: {relocations}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn static_function_designators_reject_object_reads_like_clang() {
    let root = std::env::temp_dir().join(format!(
        "bcc-static-designator-errors-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    for initializer in [
        "*fp",
        "**fp",
        "flag ? f : g",
        "flag ? f : f",
        "1 ? fp : f",
        "0 ? f : fp",
        "*ip",
    ] {
        let source = format!(
            "int f(int); int g(int); int (*fp)(int) = f; int flag; int **ip;\n{}\n",
            if initializer == "*ip" {
                format!("int *p = {initializer};")
            } else {
                format!("int (*p)(int) = {initializer};")
            }
        );
        let input = root.join("rejected.c");
        fs::write(&input, &source).unwrap();
        for bcc in [false, true] {
            let mut command = Command::new(if bcc {
                PathBuf::from(env!("CARGO_BIN_EXE_bcc-rust"))
            } else {
                clang()
            });
            _ = command.arg("-std=c99");
            _ = command.arg(if bcc { "--emit=ir" } else { "-fsyntax-only" });
            _ = command.arg(&input);
            let output = crate::native::run_native(&mut command).unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{source}: {stderr}");
            assert!(stderr.contains("constant"), "{source}: {stderr}");
        }
    }
    fs::remove_dir_all(root).unwrap();
}
