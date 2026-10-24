//! Static initializers whose designators write addresses out of order, over
//! each other and through GNU ranges must run alike on every arm.
#![cfg(target_os = "linux")]
#![expect(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Integration tests own source files and process output."
)]

#[cfg(test)]
mod tests {
    use std::{
        fs,
        process::Command,
    };

    /// Exits with the number of the first wrong element, or 0.
    const PROGRAM: &str = r"
int a = 1, b = 2, c = 3;
static int f(void) { return 4; }
static int g(void) { return 5; }

/* Addresses written backwards and over each other. */
int *backwards[4] = { [3] = &c, [1] = &b, [0] = &a, [2] = &a, [1] = &c };
/* A scalar replaces an address, and a later address replaces the scalar. */
long mixed[3] = { [0] = (long)&a, [1] = (long)&b, [0] = 7, [2] = 9, [2] = (long)&c };
/* GNU ranges copy the addresses they cover; later designators override. */
int (*ranges[8])(void) = { [0 ... 6] = f, [2] = g, [5 ... 7] = g, [6] = f };
struct pair { int *p; int (*q)(void); };
struct pair pairs[5] = { [4].p = &c, [0 ... 3] = { &a, f }, [1].q = g, [3].p = &b };

int main(void) {
    int n = 0;
    if (++n, *backwards[0] != 1) return n;
    if (++n, *backwards[1] != 3) return n;
    if (++n, *backwards[2] != 1) return n;
    if (++n, *backwards[3] != 3) return n;
    if (++n, mixed[0] != 7) return n;
    if (++n, *(int *)mixed[1] != 2) return n;
    if (++n, *(int *)mixed[2] != 3) return n;
    static const int expected[8] = { 4, 4, 5, 4, 4, 5, 4, 5 };
    for (int i = 0; i < 8; i++)
        if (++n, ranges[i]() != expected[i]) return n;
    for (int i = 0; i < 4; i++)
        if (++n, *pairs[i].p != (i == 3 ? 2 : 1) || pairs[i].q() != (i == 1 ? 5 : 4)) return n;
    if (++n, *pairs[4].p != 3 || pairs[4].q) return n;
    return 0;
}
";

    #[test]
    fn static_relocations_follow_designator_order_on_every_arm() {
        let root =
            std::env::temp_dir().join(format!("bcc-static-relocations-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("relocations.c");
        fs::write(&source, PROGRAM).unwrap();
        let bcc = || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.args(["-std=gnu17", "--target=x86_64-unknown-linux-gnu"]);
            _ = command.arg(&source);
            command
        };
        for flags in [&["--interpret"][..], &["--interpret", "--opt=bcc"]] {
            let output = crate::native::run_native(bcc().args(flags)).unwrap();
            assert_eq!(output.status.code(), Some(0), "{flags:?}: {output:?}");
        }
        let executable = root.join("relocations");
        for flags in [
            &["--llvm-opt=O0"][..],
            &["--opt=bcc", "--llvm-opt=O2"],
            &["--backend=native"],
            &["--backend=native", "--opt=bcc"],
            &["--backend=native", "--native-object"],
            &["--backend=native", "--native-object", "--opt=bcc"],
        ] {
            let output =
                crate::native::run_native(bcc().args(flags).arg("-o").arg(&executable)).unwrap();
            assert!(output.status.success(), "{flags:?}: {output:?}");
            let output = crate::native::run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(output.status.code(), Some(0), "{flags:?}: {output:?}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
