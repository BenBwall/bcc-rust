//! Math builtin execution through both executable-producing CLI back ends.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests own subprocess paths, source text and captured output."
)]
mod tests {
    use std::{
        fs,
        process::Command,
    };

    #[test]
    fn math_builtins_link_libm_in_both_executable_backends() {
        let target = if cfg!(all(target_os = "linux", target_env = "gnu")) {
            "x86_64-unknown-linux-gnu"
        } else if cfg!(all(target_os = "linux", target_env = "musl")) {
            "x86_64-unknown-linux-musl"
        } else if cfg!(all(windows, target_env = "gnu")) {
            "x86_64-w64-windows-gnu"
        } else {
            eprintln!("skipping executable math builtins: compatible linker unavailable");
            return;
        };
        let directory = std::env::temp_dir().join(format!("bcc-math-cli-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("math.c");
        fs::write(
            &source,
            r#"
#define INFINITY __builtin_inff()
#define NAN __builtin_nanf("")
#define HUGE_VAL __builtin_huge_val()
static float infinity = INFINITY, nan = NAN;
static double huge = HUGE_VAL;
int main(void) {
    volatile double x = 1.5;
    if (__builtin_ceil(x) != 2.0 || __builtin_sqrt(x + 2.5) != 2.0) return 1;
    if (__builtin_pow(x + 0.5, 3.0) != 8.0 || __builtin_exp(0.0) != 1.0) return 2;
    if (__builtin_fma(x, 2.0, 1.0) != 4.0 || __builtin_fmin(x, 1.0) != 1.0) return 3;
    if (!__builtin_isinf(infinity) || !__builtin_isnan(nan) || !__builtin_isinf(huge)) return 4;
    volatile unsigned count = 32;
    return __builtin_rotateleft32(0x12345678u, count) != 0x12345678u;
}
"#,
        )
        .unwrap();
        for backend in ["llvm", "native"] {
            let executable = directory.join(format!("math-{backend}.exe"));
            let built = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .arg(&source)
                .args([
                    &format!("--target={target}"),
                    &format!("--backend={backend}"),
                    "--opt=none",
                    "-o",
                ])
                .arg(&executable)
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "{backend}: {}",
                String::from_utf8_lossy(&built.stderr)
            );
            let ran = crate::native::run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(ran.status.code(), Some(0), "{backend}: {ran:?}");
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
