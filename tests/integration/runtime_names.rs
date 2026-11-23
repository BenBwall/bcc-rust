//! Runtime functions bcc calls by name on its own, when the unit gives
//! that name to something else. An external function of the name is
//! called as Clang calls it, through its address when its type is not the
//! routine's. A static function or an object of the name is refused in
//! lowering and by the native back end for the names it calls; the
//! `memcpy` LLVM's code generator calls reaches a static function of the
//! name, as in Clang. Each case is checked on a Windows and a Linux target,
//! at `-O0` and `-O2` on both back ends.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        path::PathBuf,
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

    const WINDOWS: [&str; 2] = ["x86_64-pc-windows-msvc", "x86_64-w64-windows-gnu"];
    const LINUX: &str = "x86_64-unknown-linux-gnu";

    /// The power routine of a Windows `__builtin_powi` given to a static
    /// function, which Clang calls with the routine's arguments, or to an
    /// object, is refused with both names. Off Windows the power is the
    /// IR's `powi`, which leaves the static `pow` alone.
    #[test]
    fn a_power_routine_named_by_a_static_function_or_an_object_is_refused() {
        let static_pow = "static int pow(int a) { return a + 1; }\nint use(int a) { return \
                          pow(a); }\ndouble d(double x, int n) { return __builtin_powi(x, n); }\n";
        let static_powf = "static int powf(int a) { return a + 1; }\nint use(int a) { return \
                           powf(a); }\nfloat d(float x, int n) { return __builtin_powif(x, n); }\n";
        let later_pow = "double d(double x, int n) { return __builtin_powi(x, n); }\nstatic int \
                         pow(int a) { return a + 1; }\nint use(int a) { return pow(a); }\n";
        let object_pow = "int pow;\ndouble d(double x, int n) { return __builtin_powi(x, n); }\n";
        let static_powidf2 = "static int __powidf2(int a) { return a + 1; }\nint use(int a) { \
                              return __powidf2(a); }\ndouble d(double x, int n) { return \
                              __builtin_powi(x, n); }\n";
        for target in WINDOWS {
            for (source, identity, message) in [
                (
                    static_pow,
                    "clang",
                    "'__builtin_powi' calls the external function 'pow', but 'pow' names a \
                     function with internal linkage here",
                ),
                (
                    static_powf,
                    "clang",
                    "'__builtin_powif' calls the external function 'powf', but 'powf' names a \
                     function with internal linkage here",
                ),
                (
                    later_pow,
                    "clang",
                    "'__builtin_powi' calls the external function 'pow', but 'pow' names a \
                     function with internal linkage here",
                ),
                (
                    object_pow,
                    "clang",
                    "'__builtin_powi' calls the external function 'pow', but 'pow' names an \
                     object here",
                ),
                (
                    static_powidf2,
                    "gcc",
                    "'__builtin_powi' calls the external function '__powidf2', but '__powidf2' \
                     names a function with internal linkage here",
                ),
            ] {
                for args in compilations(target) {
                    let output = compile(
                        source,
                        &[&args[..], &["--compiler-identity", identity]].concat(),
                    );
                    assert_refused(&output, message, &args);
                }
            }
        }
        for source in [static_pow, later_pow, object_pow] {
            let ir = emit(source, &["--emit=ir", "--target", LINUX]);
            assert!(ir.contains("intrinsic powi.f64 "), "{ir}");
            for args in compilations(LINUX) {
                assert_compiles(&compile(source, &args), &args);
            }
        }
    }

    /// An external function of a runtime routine's name with another type
    /// is the one Clang's call by name reaches: the call goes through its
    /// address with the routine's type, on every back end.
    #[test]
    fn a_runtime_routine_of_another_type_is_called_through_its_address() {
        let cases = [
            (
                "double d(double x, int n) { return __builtin_powi(x, n); }\nint pow(int a) { \
                 return a; }\n",
                "clang",
                "call_indirect v3(v0, v2) : (f64, f64) -> f64",
                &WINDOWS[..],
            ),
            (
                "double d(double x, int n) { return __builtin_powi(x, n); }\nint __powidf2(int a) \
                 { return a; }\n",
                "gcc",
                "call_indirect v2(v0, v1) : (f64, i32) -> f64",
                &WINDOWS[..],
            ),
            (
                "int __cpu_indicator_init(int a) { return a; }\nvoid f(void) { \
                 __builtin_cpu_init(); }\n",
                "clang",
                "call_indirect v0() : ()",
                &[WINDOWS[0], WINDOWS[1], LINUX][..],
            ),
            (
                "int __muldc3(int a) { return a; }\n_Complex double f(_Complex double a, _Complex \
                 double b) { return a * b; }\n",
                "clang",
                "= func_addr @__muldc3",
                &[WINDOWS[0], LINUX][..],
            ),
        ];
        for (source, identity, call, targets) in cases {
            for &target in targets {
                let ir = emit(
                    source,
                    &[
                        "--emit=ir",
                        "--target",
                        target,
                        "--compiler-identity",
                        identity,
                    ],
                );
                assert!(ir.contains(call), "{target}: {ir}");
                for args in compilations(target) {
                    assert_compiles(
                        &compile(
                            source,
                            &[&args[..], &["--compiler-identity", identity]].concat(),
                        ),
                        &args,
                    );
                }
            }
        }
        // bcc's `-fsanitize=undefined` checks only Linux targets.
        let sanitized = "int __ubsan_handle_add_overflow(int a) { return a; }\nint f(int a, int \
                         b) { return a + b; }\n";
        for target in [LINUX] {
            let ir = emit(
                sanitized,
                &["--emit=ir", "-fsanitize=undefined", "--target", target],
            );
            assert!(
                ir.contains("= func_addr @__ubsan_handle_add_overflow"),
                "{ir}"
            );
            assert!(ir.contains(" : (ptr, i64, i64)"), "{ir}");
            for args in compilations(target) {
                assert_compiles(
                    &compile(sanitized, &[&args[..], &["-fsanitize=undefined"]].concat()),
                    &args,
                );
            }
        }
    }

    /// A static function with the name of `__muldc3`, `__cpu_indicator_init`
    /// or the `longjmp` of a jump out of a nested function is not that
    /// routine; lowering refuses the operation that would call it.
    #[test]
    fn other_runtime_routines_named_by_static_functions_are_refused() {
        let complex = "static int __muldc3(int a) { return a; }\nint use(int a) { return \
                       __muldc3(a); }\n_Complex double f(_Complex double a, _Complex double b) { \
                       return a * b; }\n";
        let processor = "static int __cpu_indicator_init(int a) { return a; }\nint use(int a) { \
                         return __cpu_indicator_init(a); }\nvoid f(void) { __builtin_cpu_init(); \
                         }\n";
        for target in [WINDOWS[0], WINDOWS[1], LINUX] {
            for args in compilations(target) {
                assert_refused(
                    &compile(complex, &args),
                    "complex multiplication calls the external function '__muldc3', but \
                     '__muldc3' names a function with internal linkage here",
                    &args,
                );
                assert_refused(
                    &compile(processor, &args),
                    "'__builtin_cpu_init' calls the external function '__cpu_indicator_init', but \
                     '__cpu_indicator_init' names a function with internal linkage here",
                    &args,
                );
            }
        }
        // Only Linux targets lower such a jump; Windows refuses it anyway.
        let nested = "static void longjmp(void *p, int a) { (void)p; (void)a; }\nvoid use(void) { \
                      longjmp(0, 1); }\nint f(int n) {\n  __label__ out;\n  void g(int k) { if \
                      (k) goto out; }\n  g(n);\n  return 0;\nout:\n  return 1;\n}\n";
        for args in compilations(LINUX) {
            assert_refused(
                &compile(
                    nested,
                    &[&args[..], &["--compiler-identity", "gcc"]].concat(),
                ),
                "a jump out of a nested function calls the external function 'longjmp', but \
                 'longjmp' names a function with internal linkage here",
                &args,
            );
        }
    }

    /// libgcc's processor data, `__cpu_model` and `__cpu_features2`, which
    /// the processor queries read, given to a function or to an object with
    /// internal linkage is not that data; lowering refuses the query with
    /// both names, as for the runtime functions. An external object of the
    /// name is read where it stands, whatever its type, as Clang and GCC
    /// read it.
    #[test]
    fn processor_data_named_by_a_function_or_a_static_object_is_refused() {
        let function = "int __cpu_model(int a) { return a; }\nint f(void) { return \
                        __builtin_cpu_supports(\"sse4.2\"); }\n";
        let declared = "int __cpu_model(int);\nint g(void) { return __cpu_model(1); }\nint \
                        f(void) { return __builtin_cpu_is(\"intel\"); }\n";
        let object = "static int __cpu_model = 3;\nint f(void) { return \
                      __builtin_cpu_supports(\"sse4.2\") + __cpu_model; }\n";
        let features = "static int __cpu_features2[3];\nint f(void) { return \
                        __builtin_cpu_supports(\"avx512bf16\") + __cpu_features2[0]; }\n";
        let external = "extern char __cpu_model[64];\nint f(void) { return \
                        __builtin_cpu_supports(\"sse4.2\") + __cpu_model[0]; }\n";
        for target in [WINDOWS[0], WINDOWS[1], LINUX] {
            for args in compilations(target) {
                for (source, message) in [
                    (
                        function,
                        "'__builtin_cpu_supports' reads the external object '__cpu_model', but \
                         '__cpu_model' names a function here",
                    ),
                    (
                        declared,
                        "'__builtin_cpu_is' reads the external object '__cpu_model', but \
                         '__cpu_model' names a function here",
                    ),
                    (
                        object,
                        "'__builtin_cpu_supports' reads the external object '__cpu_model', but \
                         '__cpu_model' names an object with internal linkage here",
                    ),
                    (
                        features,
                        "'__builtin_cpu_supports' reads the external object '__cpu_features2', \
                         but '__cpu_features2' names an object with internal linkage here",
                    ),
                ] {
                    assert_refused(&compile(source, &args), message, &args);
                }
                assert_compiles(&compile(external, &args), &args);
            }
        }
    }

    /// The native back end calls libgcc's `__powidf2` for the IR's `powi`,
    /// so it refuses a module whose `powi` call would reach a static
    /// function or an object of that name, as for its other runtime
    /// routines' names; the LLVM back end calls the static one as Clang
    /// does, the name being reserved. A file that names the routine but
    /// uses no `powi` compiles as before.
    #[test]
    fn the_native_back_end_refuses_a_static_power_routine() {
        let source = "static int __powidf2(int a) { return a + 1; }\nint use(int a) { return \
                      __powidf2(a); }\ndouble d(double x, int n) { return __builtin_powi(x, n); \
                      }\n";
        let object = "int __powisf2;\nfloat d(float x, int n) { return __builtin_powif(x, n); }\n";
        for level in ["-O0", "-O2"] {
            for (source, name) in [(source, "__powidf2"), (object, "__powisf2")] {
                let args = ["--target", LINUX, "--backend=native", level];
                assert_refused(
                    &compile(source, &args),
                    &format!(
                        "not yet supported by the native back end: a module-local definition of \
                         the runtime symbol `{name}`"
                    ),
                    &args,
                );
            }
            let args = ["--target", LINUX, "--backend=llvm", level];
            assert_compiles(&compile(source, &args), &args);
        }
        let unused = [
            "static int __powidf2(int a) { return a + 1; }\nint use(int a) { return __powidf2(a); \
             }\n",
            "extern int __powidf2;\nint *use(void) { return &__powidf2; }\n",
            "static float __powisf2;\nfloat *use(void) { return &__powisf2; }\ndouble d(double x, \
             int n) { return __builtin_powi(x, n); }\n",
        ];
        for source in unused {
            for args in compilations(LINUX) {
                assert_compiles(&compile(source, &args), &args);
            }
        }
    }

    /// The `memcpy`, `memmove` and `memset` LLVM's code generator calls for
    /// copies and fills bind to a module's function of the name, whatever
    /// its type, as in Clang: the LLVM back end keeps a static function's
    /// name. Clang then calls a static `int memcpy(int)` with the library
    /// function's arguments; a file that names one so without including
    /// `<string.h>` is valid, and that call is the known difference.
    #[test]
    fn static_functions_of_the_copy_routines_keep_their_names() {
        let other = "static int memcpy(int a) { return a + 1; }\nstatic int memset(int a) { \
                     return a + 3; }\nint use(void) { return memcpy(0) + memset(0); }\n";
        let library = "typedef __SIZE_TYPE__ size_t;\nstatic void *memcpy(void *d, const void *s, \
                       size_t n) { char *a = d; const char *b = s; while (n--) *a++ = *b++; \
                       return d; }\nvoid f(char *a, char *b) { memcpy(a, b, 3); }\n";
        for target in [WINDOWS[0], LINUX] {
            let llvm = emit(other, &["--emit=llvm", "--target", target]);
            assert!(llvm.contains("define internal i32 @memcpy("), "{llvm}");
            assert!(llvm.contains("define internal i32 @memset("), "{llvm}");
            let llvm = emit(library, &["--emit=llvm", "--target", target]);
            assert!(llvm.contains("define internal ptr @memcpy("), "{llvm}");
        }
    }

    /// A freestanding program without `<string.h>` that defines its own
    /// `memcpy` and a `memset` of a looser type, statically, and is linked
    /// with `-nostdlib`: the copies and fills LLVM's code generator calls by
    /// name reach those functions, as in GCC and Clang, so the program links
    /// and runs. `used` keeps the statics at `-O2`, where LLVM would drop
    /// them, once inlined, before its code generator calls them, as Clang
    /// does. The `memmove` the native back end calls is external, since that
    /// back end refuses a static one. C99: §4p6, p. 8; PDF p. 20; §7.1.3p1,
    /// p. 166; PDF p. 178.
    #[test]
    fn freestanding_copies_reach_the_programs_static_routines() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = "typedef __SIZE_TYPE__ size_t;\n__attribute__((used)) static char *memcpy(d, \
                      s, n) char *d; const char *s; int n; { char *a = d; while (n--) *a++ = \
                      *s++; return d; }\nvoid *memmove(void *d, const void *s, size_t n) { char \
                      *a = d; const char *b = s; if (a < b) while (n--) *a++ = *b++; else while \
                      (n--) a[n] = b[n]; return d; }\n__attribute__((used)) static void \
                      memset(void *d, int c, size_t n) { char *a = d; while (n--) *a++ = (char)c; \
                      }\nstruct S { char b[512]; };\nstatic struct S x, \
                      y;\n__attribute__((noinline)) static int run(void) { x.b[7] = 1; struct S z \
                      = {0}; x = z; memset(&y, 0, 1); y = x; return memmove(&y, &x, 1) != &y || \
                      x.b[7] || memcpy((char *)&y, (char *)&x, 2) != (char *)&y; }\nvoid \
                      _start(void) { int r = run(); __asm__ volatile(\"mov $60, %%eax; syscall\" \
                      :: \"D\"(r)); __builtin_unreachable(); }\n";
        let path = temporary("freestanding.c");
        std::fs::write(&path, source).unwrap();
        let path = path.to_str().unwrap();
        // TCC's command line chooses no back end; bcc's driver gives TCC's
        // identity to both.
        let mut lines = vec![vec!["--tcc-mode", "-O0"], vec!["--tcc-mode", "-O2"]];
        for identity in ["tcc", "gcc", "clang"] {
            for backend in ["--backend=llvm", "--backend=native"] {
                for level in ["-O0", "-O2"] {
                    lines.push(vec!["--compiler-identity", identity, backend, level]);
                }
            }
        }
        for line in lines {
            let executable = temporary("freestanding");
            let executable = executable.to_str().unwrap();
            let args = [
                &line[..],
                &[
                    "-ffreestanding",
                    "-static",
                    "-nostdlib",
                    "-o",
                    executable,
                    path,
                ],
            ]
            .concat();
            let built = bcc(&args);
            assert_compiles(&built, &args);
            let run = run_native(&mut Command::new(executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{args:?}");
        }
    }

    /// The command lines each check compiles an object with: both back ends
    /// at `-O0` and `-O2`.
    fn compilations(target: &str) -> Vec<[&str; 4]> {
        let mut lines = Vec::new();
        for backend in ["--backend=llvm", "--backend=native"] {
            for level in ["-O0", "-O2"] {
                lines.push(["--target", target, backend, level]);
            }
        }
        lines
    }

    /// Compiles `source` to an object with `args`.
    fn compile(source: &str, args: &[&str]) -> Output {
        let path = temporary("unit.c");
        std::fs::write(&path, source).unwrap();
        let object = temporary("unit.o");
        let mut line = vec![path.to_str().unwrap(), "-c", "-o", object.to_str().unwrap()];
        if args.contains(&"-o") {
            line.truncate(1);
        }
        line.extend_from_slice(args);
        bcc(&line)
    }

    /// What `args` print for `source`, which must compile.
    fn emit(source: &str, args: &[&str]) -> String {
        let path = temporary("unit.c");
        std::fs::write(&path, source).unwrap();
        let output = bcc(&[&[path.to_str().unwrap()][..], args].concat());
        assert!(
            output.status.success(),
            "{args:?}: {}",
            text(&output.stderr)
        );
        text(&output.stdout)
    }

    fn assert_compiles(output: &Output, args: &[&str]) {
        assert!(
            output.status.success(),
            "{args:?}: {}",
            text(&output.stderr)
        );
    }

    fn assert_refused(output: &Output, message: &str, args: &[&str]) {
        assert_eq!(
            output.status.code(),
            Some(1),
            "{args:?}: {}",
            text(&output.stderr)
        );
        assert!(
            text(&output.stderr).contains(&format!("error: {message}")),
            "{args:?}: {}",
            text(&output.stderr)
        );
    }

    /// A fresh path in the system's temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "bcc-runtime-names-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(name)
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
}
