//! C overload symbols must link across compiler boundaries in both directions.
#![expect(unused_crate_dependencies, reason = "Tests run compiler processes.")]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests own files and captured command output."
)]
#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{
            Path,
            PathBuf,
        },
        process::Command,
    };

    const API: &str = r#"
struct Pair { int first, second; };
int choose(int) __attribute__((overloadable));
int choose(double) __attribute__((overloadable));
long long wide(long long,long long) __attribute__((overloadable));
int *point(int *const) __attribute__((overloadable));
int array(int [3]) __attribute__((overloadable));
int pair(struct Pair) __attribute__((overloadable));
int callback(int (*)(int)) __attribute__((overloadable));
int nested(int (*)(int *const)) __attribute__((overloadable));
typedef int Aligned __attribute__((aligned(32)));
int same_alignment(Aligned *,int *) __attribute__((overloadable));
int limited(int n) __attribute__((overloadable,enable_if(n>0,"positive")));
int ordinary(int n) __attribute__((enable_if(n>0,"positive")));
int sized(char *const p __attribute__((pass_object_size(1)))) __attribute__((overloadable));
"#;
    const CALLEE: &str = r#"
#include "api.h"
int choose(int n) __attribute__((overloadable)) { return n+1; }
int choose(double n) __attribute__((overloadable)) { return (int)n+2; }
long long wide(long long a,long long b) __attribute__((overloadable)) { return a+b; }
int *point(int *const p) __attribute__((overloadable)) { return p; }
int array(int a[3]) __attribute__((overloadable)) { return a[1]; }
int pair(struct Pair p) __attribute__((overloadable)) { return p.first+p.second; }
int callback(int (*f)(int)) __attribute__((overloadable)) { return f(6); }
int nested(int (*f)(int *const)) __attribute__((overloadable)) { return f(0); }
int same_alignment(Aligned *a,int *b) __attribute__((overloadable)) { return a==b; }
int limited(int n) __attribute__((overloadable,enable_if(n>0,"positive"))) { return n; }
int ordinary(int n) __attribute__((enable_if(n>0,"positive"))) { return n; }
int sized(char *const p __attribute__((pass_object_size(1)))) __attribute__((overloadable)) { return __builtin_object_size(p,1)==4; }
"#;
    const CALLER: &str = r#"
#include "api.h"
/* Freestanding MSVC floating-point code requires this runtime marker. */
int _fltused = 0;
int plus(int n) { return n+2; }
int pointer_callback(int *const p) { return p==0; }
int entry(void) {
 int n=3,values[3]={2,4,6};char data[4];struct Pair p={2,9};
 int result=choose(5)+choose(2.5)+wide(7,8)+*point(&n)+array(values)+pair(p)+callback(plus)+sized(data)+limited(2)+ordinary(3)+nested(pointer_callback)+same_alignment(&n,&n);
 return result==59 ? 0 : 1;
}
#ifndef _WIN32
int main(void) { return entry(); }
#endif
"#;

    fn checked(command: &mut Command) {
        let output = command
            .env("NO_COLOR", "1")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command:?}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn clang() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/llvm/bin")
            .join(if cfg!(windows) { "clang.exe" } else { "clang" })
    }

    #[test]
    fn overloadable_symbols_interoperate_with_clang_in_both_directions() {
        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            for backend in ["llvm", "native"] {
                for bcc_is_caller in [true, false] {
                    let root = std::env::temp_dir().join(format!(
                        "bcc-overload-abi-{}-{target}-{bcc_is_caller}-{backend}",
                        std::process::id()
                    ));
                    fs::create_dir_all(&root).unwrap();
                    fs::write(root.join("api.h"), API).unwrap();
                    fs::write(root.join("caller.c"), CALLER).unwrap();
                    fs::write(root.join("callee.c"), CALLEE).unwrap();
                    let mut objects = Vec::new();
                    for (unit, is_caller) in [("caller", true), ("callee", false)] {
                        let compiler = if is_caller == bcc_is_caller {
                            PathBuf::from(env!("CARGO_BIN_EXE_bcc-rust"))
                        } else {
                            clang()
                        };
                        let object = root.join(format!("{unit}.o"));
                        let mut compile = Command::new(compiler);
                        _ = compile
                            .args([&format!("--target={target}"), "-std=gnu17", "-c"])
                            .arg(root.join(format!("{unit}.c")))
                            .arg("-o")
                            .arg(&object);
                        if is_caller == bcc_is_caller {
                            _ = compile.arg(format!("--backend={backend}"));
                            if target == "x86_64-w64-windows-gnu" {
                                // The native assembler driver requires a MinGW
                                // root. These header-free objects and the
                                // freestanding link need no target library.
                                _ = compile.env("MINGW_ROOT", &root);
                            }
                        }
                        checked(&mut compile);
                        objects.push(object);
                    }
                    let windows = target.contains("windows");
                    let executable = root.join(if windows { "check.exe" } else { "check" });
                    let native_elf =
                        cfg!(target_os = "linux") && target == "x86_64-unknown-linux-gnu";
                    let mut linker = if native_elf {
                        let mut command = Command::new(clang());
                        _ = command.args([&format!("--target={target}"), "-fuse-ld=lld"]);
                        command
                    } else {
                        let mut command = Command::new(clang().with_file_name(if cfg!(windows) {
                            "lld.exe"
                        } else {
                            "ld.lld"
                        }));
                        if target.ends_with("windows-msvc") {
                            _ = command
                                .args([
                                    "-flavor",
                                    "link",
                                    "/entry:entry",
                                    "/subsystem:console",
                                    "/nodefaultlib",
                                ])
                                .arg(format!("/out:{}", executable.display()));
                        } else {
                            _ = command.args([
                                "-flavor",
                                "gnu",
                                "-m",
                                if windows { "i386pep" } else { "elf_x86_64" },
                                "-e",
                                "entry",
                            ]);
                            if windows {
                                _ = command.args(["--subsystem", "console"]);
                            }
                            _ = command.arg("-o").arg(&executable);
                        }
                        command
                    };
                    _ = linker.args(&objects);
                    if native_elf {
                        _ = linker.arg("-o").arg(&executable);
                    }
                    checked(&mut linker);
                    if (cfg!(windows) && windows) || native_elf {
                        let mut command = if cfg!(windows) {
                            let mut c = Command::new("python");
                            _ = c.arg("scripts/cargo_runner.py").arg(&executable);
                            c
                        } else {
                            Command::new(&executable)
                        };
                        checked(&mut command);
                    }
                    fs::remove_dir_all(root).unwrap();
                }
            }
        }
    }
}
