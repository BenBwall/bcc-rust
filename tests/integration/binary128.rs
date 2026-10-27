//! Binary128 (`__float128` and `_Float128`) programs on every back end:
//! `tests/fixtures/programs/binary128.c` built by the bundled Clang against
//! libgcc's soft-float routines and glibc is the reference, and the
//! interpreter, LLVM and the native back end must print the same bits.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    use crate::native::run_native;

    /// Arithmetic, comparisons, conversions, calls, variadic arguments,
    /// folded initializers and glibc's `_Float128` functions print what the
    /// Clang-built program prints, interpreted before and after optimization
    /// under both compiler identities, and built through LLVM and the
    /// native back end.
    #[test]
    fn binary128_programs_match_libgcc_on_every_back_end() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/binary128.c");
        let source = source.to_str().unwrap();
        let reference = temporary("reference.exe");
        let built = Command::new("target/llvm/bin/clang")
            .args([source, "-std=c99", "-O0", "-fno-builtin", "-lm", "-o"])
            .arg(&reference)
            .output()
            .unwrap();
        assert!(built.status.success(), "{}", text(&built.stderr));
        let expected = run_native(&mut Command::new(&reference)).unwrap();
        assert_eq!(expected.status.code(), Some(0));
        let expected = text(&expected.stdout);
        std::fs::remove_file(reference).unwrap();
        for flags in [
            &["--interpret", "--opt=none"][..],
            &["--interpret", "--opt=bcc"],
            &["--interpret", "--compiler-identity=gcc"],
        ] {
            let output = bcc(&[&[source][..], flags].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{flags:?}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), expected, "{flags:?}");
        }
        for flags in [
            &["-O0"][..],
            &["-O2"],
            &["--backend=native"],
            &["--backend=native", "--opt=bcc"],
        ] {
            let executable = temporary("binary128.exe");
            let built = bcc(&[
                &[source, "-lm", "-o", executable.to_str().unwrap()][..],
                flags,
            ]
            .concat());
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{flags:?}");
            assert_eq!(text(&run.stdout), expected, "{flags:?}");
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// Under the GCC identity `_Float128` is a keyword with `f128`
    /// constants, glibc's headers declare its functions, and `<float.h>`
    /// gives GCC's `FLT128_*` names when TS 18661-3 is requested; the
    /// limits print as GCC 15 prints them.
    #[test]
    fn gcc_identity_float128_names_and_limits_match_gcc() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = "#define __STDC_WANT_IEC_60559_TYPES_EXT__ 1
#include <float.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
static void show(const char *label, _Float128 value) {
    char text[64];
    strfromf128(text, sizeof text, \"%.36g\", value);
    printf(\"%s %s\\n\", label, text);
}
static const _Float128 third = 1.0f128 / 3;
int main(void) {
    show(\"max\", FLT128_MAX);
    show(\"min\", FLT128_MIN);
    show(\"true_min\", FLT128_TRUE_MIN);
    show(\"epsilon\", FLT128_EPSILON);
    show(\"third\", third);
    show(\"sqrt\", sqrtf128(2.0f128));
    show(\"strto\", strtof128(\"0.1\", NULL));
    printf(\"%d %d %d %d %d\\n\", FLT128_MANT_DIG, FLT128_DIG, FLT128_MIN_EXP,
           FLT128_MAX_10_EXP, FLT128_DECIMAL_DIG);
    printf(\"%d\\n\", (int)sizeof(_Float128) + _Generic(1.0f128, __float128: 100, default: 0));
    return 0;
}
";
        let expected = "max 1.18973149535723176508575932662800702e+4932
min 3.3621031431120935062626778173217526e-4932
true_min 6.47517511943802511092443895822764655e-4966
epsilon 1.92592994438723585305597794258492732e-34
third 0.333333333333333333333333333333333317
sqrt 1.41421356237309504880168872420969798
strto 0.100000000000000000000000000000000005
113 33 -16381 4932 36
116
";
        for flags in [
            &["--interpret"][..],
            &["-O0", "-o"],
            &["--backend=native", "-o"],
        ] {
            let executable = temporary("gcc-identity.exe");
            let mut arguments = vec![
                "--compiler-identity=gcc",
                "-std=gnu17",
                "--input",
                source,
                "-lm",
            ];
            arguments.extend_from_slice(flags);
            let output = if flags.contains(&"-o") {
                arguments.push(executable.to_str().unwrap());
                let built = bcc(&arguments);
                assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
                let run = run_native(&mut Command::new(&executable)).unwrap();
                std::fs::remove_file(&executable).unwrap();
                run
            } else {
                bcc(&arguments)
            };
            assert_eq!(
                output.status.code(),
                Some(0),
                "{flags:?}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), expected, "{flags:?}");
        }
    }

    /// Declarations shared by the two halves of the union ABI program.
    const UNION_SHAPES: &str = r"typedef __float128 Q;
union A { Q q; double d[2]; };
union B { Q q; float f[4]; };
union C { Q q; struct { double a; long b; } s; };
union D { Q q; _Complex double z; };
union E { Q q; long l; };
union G { Q q; double d; };
struct F { union A u; };
union A fa(union A x, double y);
union B fb(union B x);
union C fc(union C x);
union D fd(union D x);
union E fe(union E x);
union G fg(union G x);
struct F ff(struct F x);
union A full(long a, long b, long c, long d, long e, long f, union A x, union A y);
union A vsum(int count, ...);
";

    /// Callees that change and return each union or structure shape.
    const UNION_CALLEE: &str = r"#include <stdarg.h>
union A fa(union A x, double y) { x.q = x.q * 2 + y; return x; }
union B fb(union B x) { x.q += 1; return x; }
union C fc(union C x) { x.q -= 3; return x; }
union D fd(union D x) { x.q *= 5; return x; }
union E fe(union E x) { x.q /= 2; return x; }
union G fg(union G x) { x.q += 0.5Q; return x; }
struct F ff(struct F x) { x.u.q += 7; return x; }
union A full(long a, long b, long c, long d, long e, long f, union A x, union A y) {
    x.q += y.q + a + b + c + d + e + f; return x; }
union A vsum(int count, ...) {
    va_list list; union A total = {0}; va_start(list, count);
    for (int i = 0; i < count; i++) total.q += va_arg(list, union A).q;
    va_end(list); return total; }
";

    /// A caller that checks each result; it exits with the failing check.
    const UNION_CALLER: &str = r"int main(void) {
    union A a = {1.5Q}; union B b = {2.5Q}; union C c = {3.5Q}; union D d = {4.5Q};
    union E e = {5.5Q}; union G g = {6.5Q}; struct F f = {{7.5Q}};
    if (fa(a, 0.25).q != 3.25Q) return 1;
    if (fb(b).q != 3.5Q) return 2;
    if (fc(c).q != 0.5Q) return 3;
    if (fd(d).q != 22.5Q) return 4;
    if (fe(e).q != 2.75Q) return 5;
    if (fg(g).q != 7.0Q) return 6;
    if (ff(f).u.q != 14.5Q) return 7;
    if (full(1, 2, 3, 4, 5, 6, a, a).q != 24.0Q) return 8;
#ifndef NO_VARARGS
    if (vsum(3, a, a, a).q != 4.5Q) return 9;
#endif
    return 0;
}
";

    /// Unions that mix a `__float128` with other members link with the
    /// compiler bcc claims to be, whose classification of them differs:
    /// with the default (Clang) identity a callee or caller built by bcc
    /// links with one built by the bundled Clang, and with the GCC identity
    /// with one built by GCC, on both back ends and in both directions,
    /// including an argument after the general registers run out. Clang's
    /// own `va_arg` of such a union reads the wrong registers, so Clang's
    /// callee is not given variadic ones. The interpreter runs the whole
    /// program.
    #[test]
    fn binary128_unions_link_with_the_claimed_compiler() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let directory = temporary("unions");
        std::fs::create_dir_all(&directory).unwrap();
        let write = |name: &str, body: &str| {
            let path = directory.join(name);
            std::fs::write(&path, format!("{UNION_SHAPES}{body}")).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let definitions = write("callee.c", UNION_CALLEE);
        let checks = write("caller.c", UNION_CALLER);
        let whole = write("whole.c", &format!("{UNION_CALLEE}{UNION_CALLER}"));
        let object = |name: &str| directory.join(name).to_str().unwrap().to_owned();
        let compile = |program: &str, args: &[&str], output: &str| {
            let built = Command::new(program)
                .args(args)
                .args(["-c", "-o", output])
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "{program} {args:?}: {}",
                text(&built.stderr)
            );
        };
        let run = |first: &str, second: &str| {
            let executable = object("program.exe");
            let linked = Command::new("target/llvm/bin/clang")
                .args([first, second, "-lm", "-o", &executable])
                .output()
                .unwrap();
            assert!(linked.status.success(), "{}", text(&linked.stderr));
            run_native(&mut Command::new(&executable))
                .unwrap()
                .status
                .code()
        };
        let bcc_binary = env!("CARGO_BIN_EXE_bcc-rust");
        let clang = "target/llvm/bin/clang";
        compile(clang, &[&definitions], &object("clang-callee.o"));
        compile(clang, &[&checks], &object("clang-caller.o"));
        let gcc = Command::new("gcc")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if gcc {
            compile("gcc", &[&definitions], &object("gcc-callee.o"));
            compile("gcc", &[&checks], &object("gcc-caller.o"));
        }
        for backend in ["--backend=llvm", "--backend=native"] {
            let bcc = |identity: &str, source: &str, extra: &[&str], output: &str| {
                let mut args = vec![backend, identity, source];
                args.extend_from_slice(extra);
                compile(bcc_binary, &args, output);
            };
            let identity = "--compiler-identity=clang";
            bcc(identity, &definitions, &[], &object("bcc-callee.o"));
            bcc(
                identity,
                &checks,
                &["-DNO_VARARGS"],
                &object("bcc-caller.o"),
            );
            assert_eq!(
                run(&object("clang-callee.o"), &object("bcc-caller.o")),
                Some(0),
                "{backend}"
            );
            assert_eq!(
                run(&object("bcc-callee.o"), &object("clang-caller.o")),
                Some(0),
                "{backend}"
            );
            if gcc {
                let identity = "--compiler-identity=gcc";
                bcc(identity, &definitions, &[], &object("bccg-callee.o"));
                bcc(identity, &checks, &[], &object("bccg-caller.o"));
                assert_eq!(
                    run(&object("gcc-callee.o"), &object("bccg-caller.o")),
                    Some(0),
                    "{backend}"
                );
                assert_eq!(
                    run(&object("bccg-callee.o"), &object("gcc-caller.o")),
                    Some(0),
                    "{backend}"
                );
            }
        }
        for identity in ["--compiler-identity=clang", "--compiler-identity=gcc"] {
            let output = bcc(&[identity, "--interpret", &whole]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{identity}: {}",
                text(&output.stderr)
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn bcc(args: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE"),
        )
        .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    /// A fresh path in the system temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "bcc-binary128-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
