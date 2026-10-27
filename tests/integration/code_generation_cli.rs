#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Test harnesses own process output and file paths outside compilation."
)]
mod tests {
    use std::{
        env,
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

    use crate::{
        native::{
            run_native,
            run_native_with_timeout,
        },
        native_runner::run_until_timeout,
    };

    const SUM: &str = "int main(void) {\n  int s = 0;\n  for (int i = 1; i <= 4; i++)\n    s += \
                       i;\n  return s;\n}\n";

    /// Real glibc fortify wrappers must keep libc addresses in syscall tables.
    #[test]
    fn glibc_fortify_wrapper_addresses_match_clang_and_run() {
        if host() != Some("x86_64-unknown-linux-gnu")
            || !Path::new("/usr/include/unistd.h").is_file()
        {
            eprintln!("skipping glibc fortify test: no Linux GNU host headers");
            return;
        }
        let source = r"
#include <unistd.h>
#include <string.h>
#if __USE_FORTIFY_LEVEL != 2
#error fortify wrappers must be enabled
#endif
char *(*cat_value)(char *, const char *) = strcat;
char *(*cat_indirect)(char *, const char *) = *strcat;
char *(*cat_twice)(char *, const char *) = **strcat;
char *(*cat_conditional)(char *, const char *) = 1 ? strcat : strcpy;
char *(*cwd_value)(char *, size_t) = getcwd;
char *(*cwd_address)(char *, size_t) = &getcwd;
ssize_t (*read_value)(int, void *, size_t) = read;
ssize_t (*read_address)(int, void *, size_t) = &read;
void (*syscalls[])(void) = { (void (*)(void))getcwd, (void (*)(void))&read };
int main(void) {
    static char *(*cwd_static)(char *, size_t) = getcwd;
    char *(*cwd_local)(char *, size_t) = &getcwd;
    ssize_t (*read_local)(int, void *, size_t) = read;
    char buf[4096];
    if (cat_value != strcat || cat_indirect != strcat
        || cat_twice != strcat || cat_conditional != strcat) return 3;
    if (cwd_value != cwd_address || cwd_local != cwd_static || cwd_local != getcwd
        || read_value != read_address || read_local != read
        || syscalls[0] != (void (*)(void))getcwd || syscalls[1] != (void (*)(void))read)
        return 1;
    if (!cwd_local(buf, sizeof buf) || !getcwd(buf, sizeof buf)) return 2;
    return read_local(-1, buf, 0) != -1 || read(-1, buf, 0) != -1;
}
";
        let flags = [
            "--target=x86_64-unknown-linux-gnu",
            "-D_FORTIFY_SOURCE=2",
            "-D__OPTIMIZE__",
            "--idirafter",
            "/usr/include/x86_64-linux-gnu",
            "--idirafter",
            "/usr/include",
        ];
        let input = temporary("fortify.c");
        std::fs::write(&input, source).unwrap();
        let reference = Command::new("target/llvm/bin/clang")
            .arg(&input)
            .args([
                "-D_FORTIFY_SOURCE=2",
                "-D__OPTIMIZE__",
                "-S",
                "-emit-llvm",
                "-o",
                "-",
            ])
            .output()
            .unwrap();
        assert!(reference.status.success(), "{}", text(&reference.stderr));
        let output = bcc(source, &[&flags[..], &["--emit=llvm"]].concat());
        assert!(output.status.success(), "{}", text(&output.stderr));
        for llvm in [text(&reference.stdout), text(&output.stdout)] {
            for (global, symbol) in [
                ("cat_value", "strcat"),
                ("cat_indirect", "strcat"),
                ("cat_twice", "strcat"),
                ("cat_conditional", "strcat"),
                ("cwd_value", "getcwd"),
                ("cwd_address", "getcwd"),
                ("read_value", "read"),
                ("read_address", "read"),
            ] {
                let line = llvm
                    .lines()
                    .find(|line| line.starts_with(&format!("@{global} =")))
                    .unwrap();
                assert!(line.contains(&format!("ptr @{symbol}")), "{line}");
            }
            let table = llvm
                .lines()
                .find(|line| line.starts_with("@syscalls ="))
                .unwrap();
            assert!(
                table.contains("ptr @getcwd") && table.contains("ptr @read"),
                "{table}"
            );
        }
        let executable = temporary("fortify.exe");
        for options in [
            &["--opt=none", "--llvm-opt=O0"][..],
            &["--opt=bcc", "--llvm-opt=O2"][..],
        ] {
            let built = bcc(
                source,
                &[&flags[..], options, &["-o", executable.to_str().unwrap()]].concat(),
            );
            assert!(
                built.status.success(),
                "{options:?}: {}",
                text(&built.stderr)
            );
            let output = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{options:?}: {}",
                text(&output.stderr)
            );
        }
        std::fs::remove_file(input).unwrap();
        std::fs::remove_file(executable).unwrap();
    }

    /// `-funsigned-char` makes plain `char` unsigned in every phase, with
    /// the resource `<limits.h>` following it; `-fsigned-char` restores the
    /// target's signed `char`, and the last flag wins.
    #[test]
    fn unsigned_char_flag_reaches_limits_folding_and_lowering() {
        let source = "#include <limits.h>\nstatic int f(char x) { return x; }\nint main(void) { \
                      char d = -1; return (d > 0) + (f((char)0x80) == 128) * 2 + (CHAR_MIN == 0) \
                      * 4 + (CHAR_MAX == 255) * 8 + ('\\xff' == 255) * 16; }\n";
        for (flags, expected) in [
            (&["-funsigned-char"][..], 31),
            (&["-fno-signed-char"], 31),
            (&["-funsigned-char", "-fsigned-char"], 0),
            (&[], 0),
        ] {
            let result = bcc(
                source,
                &[&["-ffreestanding", "--interpret"][..], flags].concat(),
            );
            assert_eq!(
                result.status.code(),
                Some(expected),
                "{flags:?}: {}",
                text(&result.stderr)
            );
        }
    }

    /// `-ffloatn-types` makes `_Float32`, `_Float64` and `_Float32x`
    /// distinct types that compute in binary32 and binary64, with their
    /// constant suffixes and GCC's infinity builtins (C23 Annex H).
    #[test]
    fn floatn_types_lower_with_their_standard_representations() {
        let source = "int main(void) {\n  _Float32 a = 1.5f32; _Float64 b = 2.25f64; _Float32x c \
                      = 0.5f32x;\n  float f = a * 2;\n  _Float64 sum = a + b + c;\n  if (sum != \
                      4.25 || f != 3.0f || _Generic(a + f, _Float32: 0, default: 1)) return 1;\n  \
                      if (!(__builtin_inff32() > 1e38f32)) return 2;\n  _Complex _Float32 z = a; \
                      return (int)(__real__ z * 4) + (int)sizeof c;\n}\n";
        let result = bcc(source, &["-ffloatn-types", "--interpret"]);
        assert_eq!(result.status.code(), Some(14), "{}", text(&result.stderr));
        let rejected = bcc(source, &["--interpret"]);
        assert_eq!(
            rejected.status.code(),
            Some(1),
            "{}",
            text(&rejected.stderr)
        );
    }

    /// An unknown escape sequence stands for its character after a
    /// warning in every mode, as GCC and Clang do: `-pedantic-errors` makes
    /// it an error, and its `unknown-escape-sequence` group controls it.
    #[test]
    fn unknown_escape_sequences_warn_and_use_the_character() {
        let source = "int main(void) { return '\\j' == 'j' && sizeof \"\\%\" == 2 ? 7 : 1; }\n";
        for flags in [
            &["--interpret"][..],
            &["--interpret", "-std=c11"],
            &["--interpret", "-std=gnu17", "-pedantic"],
        ] {
            let result = bcc(source, flags);
            assert_eq!(
                result.status.code(),
                Some(7),
                "{flags:?}: {}",
                text(&result.stderr)
            );
            let stderr = text(&result.stderr);
            assert!(
                stderr.contains("warning: unknown escape sequence `\\j`")
                    && stderr.contains("[-Wunknown-escape-sequence]"),
                "{flags:?}: {stderr}"
            );
        }
        for flags in [
            &["--interpret", "-pedantic-errors"][..],
            &["--interpret", "-Werror=unknown-escape-sequence"],
        ] {
            let result = bcc(source, flags);
            assert_eq!(result.status.code(), Some(1), "{flags:?}");
            assert!(
                text(&result.stderr).contains("error: unknown escape sequence `\\j`"),
                "{flags:?}: {}",
                text(&result.stderr)
            );
        }
        let quiet = bcc(source, &["--interpret", "-Wno-unknown-escape-sequence"]);
        assert_eq!(quiet.status.code(), Some(7));
        assert!(
            !text(&quiet.stderr).contains("warning"),
            "{}",
            text(&quiet.stderr)
        );
    }

    /// Presenting as GCC (GCC's version macros, no Clang ones), glibc
    /// chooses its GCC `_FORTIFY_SOURCE` wrappers, which forward their
    /// anonymous arguments with `__builtin_va_arg_pack ()` and count them
    /// with `__builtin_va_arg_pack_len ()`. Every call then reaches the
    /// checked function, with and without the bcc optimizer, on both back
    /// ends and in the interpreter, whose host has the checked formatting
    /// functions but not `__printf_chk`, files or `%f`.
    #[test]
    fn glibc_gcc_fortify_wrappers_forward_argument_packs() {
        if host() != Some("x86_64-unknown-linux-gnu")
            || !Path::new("/usr/include/x86_64-linux-gnu/bits/stdio2.h").is_file()
        {
            eprintln!("skipping glibc GCC fortify test: no Linux GNU host headers");
            return;
        }
        let gcc = [
            "--target=x86_64-unknown-linux-gnu",
            "-U__clang__",
            "-U__clang_major__",
            "-U__clang_minor__",
            "-U__clang_patchlevel__",
            "-U__GNUC__",
            "-D__GNUC__=14",
            "-U__GNUC_MINOR__",
            "-D__GNUC_MINOR__=2",
            "-D_FORTIFY_SOURCE=2",
            "-O2",
            "--idirafter",
            "/usr/include/x86_64-linux-gnu",
            "--idirafter",
            "/usr/include",
        ];
        let source = r#"
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>
#if !defined __va_arg_pack || !defined __va_arg_pack_len || __USE_FORTIFY_LEVEL != 2
#error glibc did not choose its GCC fortify wrappers
#endif
int main(int argc, char **argv) {
    char buf[16], back[16] = {0};
    int n = snprintf(buf, sizeof buf, "%s:%d", "ab", 12);
    n += sprintf(buf + n, "/%c%.1f", 'z', 2.5);
    printf("%s %d %ld\n", buf, n, 7L);
    int fd = open(argv[1], O_CREAT | O_WRONLY | O_TRUNC, 0600);
    if (fd < 0 || write(fd, buf, n) != n || close(fd)) return 1;
    fd = open(argv[1], O_RDONLY);
    if (fd < 0 || read(fd, back, sizeof back) != n || close(fd)) return 2;
    return printf("%s\n", back) < 0;
}
"#;
        let ir = bcc(source, &[&gcc[..], &["--opt=none", "--emit=ir"]].concat());
        assert!(ir.status.success(), "{}", text(&ir.stderr));
        let ir = text(&ir.stdout);
        for call in [
            "call @__snprintf_chk(",
            "call @__sprintf_chk(",
            "call @__printf_chk(",
            "call @__open_2(",
            "call @open(",
        ] {
            assert!(ir.contains(call), "{call}:\n{ir}");
        }
        assert!(!ir.contains("call @__open_too_many_args"), "{ir}");
        assert!(!ir.contains("call @__open_missing_mode"), "{ir}");
        let file = temporary("fortify-open.txt");
        let executable = temporary("fortify-pack.exe");
        for options in [
            &["--opt=none"][..],
            &["--opt=bcc"][..],
            &["--opt=none", "--backend=native"][..],
            &["--opt=bcc", "--backend=native"][..],
        ] {
            let built = bcc(
                source,
                &[&gcc[..], options, &["-o", executable.to_str().unwrap()]].concat(),
            );
            assert!(
                built.status.success(),
                "{options:?}: {}",
                text(&built.stderr)
            );
            let output = run_native(Command::new(&executable).arg(&file)).unwrap();
            assert_eq!(
                (output.status.code(), text(&output.stdout).as_str()),
                (Some(0), "ab:12/z2.5 10 7\nab:12/z2.5\n"),
                "{options:?}: {}",
                text(&output.stderr)
            );
        }
        let interpreted = r#"
#include <stdio.h>
#ifndef __va_arg_pack
#error glibc did not choose its GCC fortify wrappers
#endif
int main(void) {
    char buf[16];
    int n = snprintf(buf, sizeof buf, "%s:%d", "ab", 12);
    n += sprintf(buf + n, "/%c%d", 'z', 25);
    puts(buf);
    return n;
}
"#;
        for optimizer in ["--opt=none", "--opt=bcc"] {
            let output = bcc(
                interpreted,
                &[&gcc[..], &[optimizer, "--interpret"]].concat(),
            );
            assert_eq!(
                (output.status.code(), text(&output.stdout).as_str()),
                (Some(9), "ab:12/z25\n"),
                "{optimizer}: {}",
                text(&output.stderr)
            );
        }
        std::fs::remove_file(file).unwrap();
        std::fs::remove_file(executable).unwrap();
    }

    /// glibc's fortify wrappers ask `__builtin_object_size` of their
    /// parameter, which only expanding the `always_inline` call answers:
    /// bcc answers it after inlining, so overflowing `memcpy`, `strcpy`,
    /// `read` and `snprintf` calls abort with glibc's "buffer overflow
    /// detected" under both GCC and Clang identity and with `--gcc-mode`,
    /// on both back ends and at both optimization levels, while calls that
    /// fit run.
    #[test]
    fn glibc_fortify_wrappers_check_object_sizes_after_inlining() {
        if host() != Some("x86_64-unknown-linux-gnu")
            || !Path::new("/usr/include/x86_64-linux-gnu/bits/string_fortified.h").is_file()
        {
            eprintln!("skipping glibc object-size test: no Linux GNU host headers");
            return;
        }
        let headers = [
            "--target=x86_64-unknown-linux-gnu",
            "-D_FORTIFY_SOURCE=2",
            "-O2",
            "--idirafter",
            "/usr/include/x86_64-linux-gnu",
            "--idirafter",
            "/usr/include",
        ];
        let gcc = [
            "-U__clang__",
            "-U__clang_major__",
            "-U__clang_minor__",
            "-U__clang_patchlevel__",
            "-U__GNUC__",
            "-D__GNUC__=14",
            "-U__GNUC_MINOR__",
            "-D__GNUC_MINOR__=2",
        ];
        let source = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#if __USE_FORTIFY_LEVEL != 2
#error glibc did not choose its fortify wrappers
#endif
struct record { long id; char name[4]; char rest[12]; };
int main(int argc, char **argv) {
    char buf[4];
    struct record r = {0};
    const char *s = argc > 2 ? argv[2] : "abcdefgh";
    switch (argv[1][0]) {
    case 'm': memcpy(buf, s, strlen(s)); break;
    case 's': strcpy(buf, s); break;
    case 'r': return read(0, buf, 16) < 0;
    case 'n': return snprintf(buf, 16, "%s", "x") < 0;
    case 'p': strcpy(r.name, s); break;
    default:
        memcpy(buf, "abc", 4);
        strcpy(r.name, "xyz");
        if (snprintf(buf, sizeof buf, "%s", "de") != 2 || read(0, buf, sizeof buf) != 0)
            return 1;
        return printf("%s %s\n", buf, r.name) < 0;
    }
    return puts(buf) < 0;
}
"#;
        let executable = temporary("fortify-sizes.exe");
        let check = |label: &str| {
            let fits = run_native(Command::new(&executable).arg("k")).unwrap();
            assert_eq!(
                (fits.status.code(), text(&fits.stdout).as_str()),
                (Some(0), "de xyz\n"),
                "{label}: {}",
                text(&fits.stderr)
            );
            for case in ["m", "s", "r", "n", "p"] {
                let output = run_native(Command::new(&executable).arg(case)).unwrap();
                assert!(
                    !output.status.success()
                        && text(&output.stderr).contains("buffer overflow detected"),
                    "{label} {case}: {:?} {}",
                    output.status,
                    text(&output.stderr)
                );
            }
        };
        let options = [
            ["--opt=none", "--backend=llvm"],
            ["--opt=bcc", "--backend=llvm"],
            ["--opt=none", "--backend=native"],
            ["--opt=bcc", "--backend=native"],
        ];
        for identity in [&[][..], &gcc[..]] {
            for options in options {
                let built = bcc(
                    source,
                    &[
                        &headers[..],
                        identity,
                        &options[..],
                        &["-o", executable.to_str().unwrap()],
                    ]
                    .concat(),
                );
                assert!(
                    built.status.success(),
                    "{identity:?} {options:?}: {}",
                    text(&built.stderr)
                );
                check(&format!("{identity:?} {options:?}"));
            }
        }
        // The GCC personality, whose glibc wrappers forward argument packs.
        let file = temporary("fortify-sizes.c");
        std::fs::write(&file, source).unwrap();
        for [optimizer, backend] in options {
            let built = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args([
                    "--gcc-mode",
                    "-D_FORTIFY_SOURCE=2",
                    "-O2",
                    "-Xbcc",
                    optimizer,
                ])
                .args(["-Xbcc", backend, "-o"])
                .arg(&executable)
                .arg(&file)
                .env("NO_COLOR", "1")
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "--gcc-mode {optimizer} {backend}: {}",
                text(&built.stderr)
            );
            check(&format!("--gcc-mode {optimizer} {backend}"));
        }
        std::fs::remove_file(file).unwrap();
        std::fs::remove_file(executable).unwrap();
    }

    /// Object-size queries in argument pack copies are answered after the
    /// copies are expanded (#150, review round 5), as GCC answers them: a
    /// query whose offset depends on another query, a wrapper with a dead
    /// recursive call, and nested pack wrappers. `B(p)` is mode 1.
    #[test]
    fn argument_pack_copies_answer_object_sizes_after_inlining() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let prologue = "#define B(x) __builtin_object_size((x), 1)\nextern int \
                        __snprintf_chk(char *, unsigned long, int, unsigned long, const char *, \
                        ...);\n";
        let nested = "\
static inline __attribute__((always_inline)) unsigned long f(char *p, ...) {
    return B(p + ((B(p) + 1) & 1)) + __builtin_va_arg_pack_len();
}
int main(void) { char a[4]; return f(a); }
";
        let nested_check = "\
static inline __attribute__((always_inline)) int f(char *p, ...) {
    return __snprintf_chk(p + ((B(p) + 1) & 1), 4, 1, B(p + ((B(p) + 1) & 1)), \"%s\",
                          __builtin_va_arg_pack());
}
int main(void) { char a[4]; return f(a, \"abc\") < 0; }
";
        let recursive = "\
static inline __attribute__((always_inline)) unsigned long f(char *p, ...) {
    if (0) return f(p);
    return __builtin_object_size(p, 1) + __builtin_va_arg_pack_len();
}
int main(void) { char a[4]; return USE; }
";
        let wrappers = "\
static inline __attribute__((always_inline)) int g(char *p, ...) {
    return __snprintf_chk(p, 16, 1, __builtin_object_size(p, 1), \"%s\", __builtin_va_arg_pack());
}
static inline __attribute__((always_inline)) int f(char *p, ...) {
    return g(p, __builtin_va_arg_pack());
}
int main(int argc, char **argv) { char a[4]; return (argc > 1 ? g(a, \"x\") : f(a, \"x\")) < 0; }
";
        let executable = temporary("pack-sizes.exe");
        let programs = [
            (nested.to_owned(), Some(3)),
            (nested_check.to_owned(), None),
            (recursive.replace("USE", "f(a)"), Some(4)),
            (recursive.replace("USE", "4"), Some(4)),
            (wrappers.to_owned(), None),
        ];
        for (program, expected) in &programs {
            let source = format!("{prologue}{program}");
            for options in [
                &["--opt=none"][..],
                &["--opt=bcc"][..],
                &["--opt=none", "--backend=native"][..],
                &["--opt=bcc", "--backend=native"][..],
            ] {
                let built = bcc(
                    &source,
                    &[options, &["-o", executable.to_str().unwrap()]].concat(),
                );
                assert!(
                    built.status.success(),
                    "{options:?}: {}\n{source}",
                    text(&built.stderr)
                );
                for args in [&[][..], &["g"][..]] {
                    let output = run_native(Command::new(&executable).args(args)).unwrap();
                    match expected {
                        | Some(code) => assert_eq!(
                            output.status.code(),
                            Some(*code),
                            "{options:?}: {}\n{source}",
                            text(&output.stderr)
                        ),
                        | None => assert!(
                            !output.status.success()
                                && text(&output.stderr).contains("buffer overflow detected"),
                            "{options:?} {args:?}: {:?}\n{source}",
                            output.status
                        ),
                    }
                }
            }
            if let Some(code) = expected {
                for optimizer in ["--opt=none", "--opt=bcc"] {
                    let output = bcc(&source, &[optimizer, "--interpret"]);
                    assert_eq!(
                        output.status.code(),
                        Some(*code),
                        "{optimizer}: {}\n{source}",
                        text(&output.stderr)
                    );
                }
            }
        }
        std::fs::remove_file(executable).unwrap();
    }

    /// A pointer an inlined function forms by a member access names a
    /// narrower subobject than its caller's argument (#151 review rounds 1
    /// and 2, also when written through the parameter's address):
    /// modes 1 and 3 of it, queried there or in a further wrapper, get the
    /// unknown answers (GCC finds 3), never the 15 bytes left in the
    /// structure the caller named. A parameter offset by pointer arithmetic
    /// alone keeps its caller's subobject and matches GCC. Without the C
    /// library (`-ffreestanding`), `malloc` names no known object, as in GCC
    /// and Clang, since the program may define it as anything.
    #[test]
    fn object_size_hints_never_cross_member_accesses() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let prologue = "struct S { char a[4]; char b[12]; };\n#define INLINE static inline \
                        __attribute__((always_inline))\n";
        let member = "\
INLINE unsigned long f(struct S *p) { char *q = &p->a[1]; return QUERY(q, MODE); }
int main(void) { struct S s; return f(&s) & 255; }
";
        let forwarded = "\
INLINE unsigned long g(char *q) { return __builtin_object_size(q, 1) + (__builtin_object_size(q, \
                         3) << 4); }
INLINE unsigned long f(struct S *p) { char *q = &p->a[1]; return g(q); }
int main(void) { struct S s; return f(&s) & 0xff; }
";
        let written = "\
INLINE unsigned long f(char *p) { p = &((struct S *)p)->b[1]; return __builtin_object_size(p, 1); }
int main(void) { char buf[16]; return f(buf) & 255; }
";
        // Written through its address (#151 review round 2).
        let through_address = "\
INLINE unsigned long f(char *p) { char **q = &p; *q = &((struct S *)p)->a[1]; return \
                               __builtin_object_size(p, 3); }
int main(void) { struct S s; return f((char *)&s) & 255; }
";
        // A dynamic query of one of two objects (round 2): GCC selects 4 at
        // run time; bcc answers unknown, never the larger 16. The static
        // query takes the larger, as GCC's does.
        let dynamic_pick = "\
volatile int pick = 0;
INLINE unsigned long f(char *p) { return __builtin_dynamic_object_size(p, 0); }
int main(void) { char small[4], large[16]; return f(pick ? large : small) & 0xff; }
";
        let arithmetic = "\
INLINE unsigned long g(char *q) { return __builtin_object_size(q + 1, 1); }
INLINE unsigned long f(char *p) { return g(p + 2); }
int main(void) { struct S s; return f(s.b) * 10 + g(&s.a[1]); }
";
        let programs = [
            (
                member
                    .replace("QUERY", "__builtin_object_size")
                    .replace("MODE", "3"),
                0,
            ),
            (
                member
                    .replace("QUERY", "__builtin_object_size")
                    .replace("MODE", "1"),
                255,
            ),
            (
                member
                    .replace("QUERY", "__builtin_dynamic_object_size")
                    .replace("MODE", "1"),
                255,
            ),
            (forwarded.to_owned(), 255),
            (written.to_owned(), 255),
            (through_address.to_owned(), 0),
            (through_address.replace(", 3)", ", 1)"), 255),
            (
                through_address.replace(
                    "__builtin_object_size(p, 3)",
                    "__builtin_dynamic_object_size(p, 3)",
                ),
                0,
            ),
            (
                through_address.replace(
                    "__builtin_object_size(p, 3)",
                    "__builtin_dynamic_object_size(p, 1)",
                ),
                255,
            ),
            (through_address.replace("char **q = &p; *q =", "*(&p) ="), 0),
            (dynamic_pick.to_owned(), 255),
            (
                dynamic_pick.replace("__builtin_dynamic_object_size", "__builtin_object_size"),
                16,
            ),
            (arithmetic.to_owned(), 92),
        ];
        let executable = temporary("subobject-hints.exe");
        for (program, expected) in &programs {
            let source = format!("{prologue}{program}");
            for options in [
                &["--opt=none"][..],
                &["--opt=bcc"][..],
                &["--opt=none", "--backend=native"][..],
                &["--opt=bcc", "--backend=native"][..],
            ] {
                let built = bcc(
                    &source,
                    &[options, &["-o", executable.to_str().unwrap()]].concat(),
                );
                assert!(
                    built.status.success(),
                    "{options:?}: {}",
                    text(&built.stderr)
                );
                let output = run_native(&mut Command::new(&executable)).unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(*expected),
                    "{options:?}\n{source}"
                );
            }
            for optimizer in ["--opt=none", "--opt=bcc"] {
                let output = bcc(&source, &[optimizer, "--interpret"]);
                assert_eq!(
                    output.status.code(),
                    Some(*expected),
                    "{optimizer}\n{source}"
                );
            }
        }
        let source = "\
extern void *malloc(unsigned long);
int main(void) { char *p = malloc(16); return __builtin_object_size(p, 0) & 0xff; }
";
        for backend in ["--backend=llvm", "--backend=native"] {
            for (hosted, expected) in [("-fhosted", 16), ("-ffreestanding", 255)] {
                let built = bcc(
                    source,
                    &[backend, hosted, "-o", executable.to_str().unwrap()],
                );
                assert!(
                    built.status.success(),
                    "{backend} {hosted}: {}",
                    text(&built.stderr)
                );
                let output = run_native(&mut Command::new(&executable)).unwrap();
                assert_eq!(output.status.code(), Some(expected), "{backend} {hosted}");
            }
        }
        std::fs::remove_file(executable).unwrap();
    }

    /// GCC never evaluates an object-size operand, so a query whose operand
    /// would read through a pointer, read an atomic or volatile object, or
    /// call a function, or evaluate a variably modified type's bound, is
    /// answered as unknown without evaluating it (#151 review rounds 3 and
    /// 4): a null pointer is never read, by either builtin, on any back end,
    /// at either level.
    #[test]
    fn object_size_operands_are_never_evaluated() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let operands = [
            ("_Atomic(char *) *p = 0;", "*p"),
            ("char * volatile *p = 0;", "*p"),
            ("char **p = 0;", "*p"),
            ("struct { char *s; } *p = 0;", "p->s"),
            ("char **p = 0;", "p[1]"),
            ("extern char *get(void);", "get()"),
            // A variably modified cast evaluates its bound (round 4).
            ("int *n = 0;", "(char (*)[*n])0"),
            ("_Atomic(int) *n = 0;", "(char (*)[*n])0"),
            ("int *n = 0;", "(__typeof__(char [*n]) *)0"),
        ];
        let executable = temporary("unevaluated-size.exe");
        for (declaration, operand) in operands {
            for builtin in ["__builtin_object_size", "__builtin_dynamic_object_size"] {
                for mode in [0, 1] {
                    let source = format!(
                        "char *get(void) {{ return *(char *volatile *)0; }}\nint main(void) {{ \
                         {declaration} return {builtin}({operand}, {mode}) & 255; }}\n"
                    );
                    for options in [
                        &["--opt=none"][..],
                        &["--opt=bcc"][..],
                        &["--opt=none", "--backend=native"][..],
                        &["--opt=bcc", "--backend=native"][..],
                    ] {
                        let built = bcc(
                            &source,
                            &[options, &["-o", executable.to_str().unwrap()]].concat(),
                        );
                        assert!(
                            built.status.success(),
                            "{options:?}: {}",
                            text(&built.stderr)
                        );
                        let output = run_native(&mut Command::new(&executable)).unwrap();
                        assert_eq!(output.status.code(), Some(255), "{options:?}\n{source}");
                    }
                    for optimizer in ["--opt=none", "--opt=bcc"] {
                        let output = bcc(&source, &[optimizer, "--interpret"]);
                        assert_eq!(output.status.code(), Some(255), "{optimizer}\n{source}");
                    }
                }
            }
        }
        std::fs::remove_file(executable).unwrap();
    }

    #[test]
    fn interpreter_step_limit_applies_to_both_main_signatures() {
        for source in [
            SUM.to_owned(),
            SUM.replace("main(void)", "main(int argc, char **argv)"),
        ] {
            let limited = bcc(&source, &["--interpret", "--interpret-step-limit=1"]);
            assert_eq!(limited.status.code(), Some(70), "{}", text(&limited.stderr));
            assert!(text(&limited.stderr).contains("exceeded its step limit"));
            for limit in ["10000", "0"] {
                let result = bcc(
                    &source,
                    &["--interpret", &format!("--interpret-step-limit={limit}")],
                );
                assert_eq!(result.status.code(), Some(10), "{}", text(&result.stderr));
            }
        }
        for flags in [
            vec!["--interpret-step-limit=1"],
            vec!["--interpret", "--interpret-step-limit=-1"],
            vec!["--interpret", "--interpret-step-limit=none"],
        ] {
            assert_eq!(bcc(SUM, &flags).status.code(), Some(2));
        }
    }

    #[test]
    fn c99_host_math_families_match_native_output_in_each_precision() {
        let Some(target) = host() else {
            return;
        };
        let clang = if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        };
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/host_math.c");
        for long_double in [false, true] {
            let executable = temporary("host-math.exe");
            let mut compiler = Command::new(clang);
            _ = compiler
                .arg(&source)
                .args([
                    &format!("--target={target}"),
                    "-std=c99",
                    "-O0",
                    "-fno-builtin",
                    "-o",
                ])
                .arg(&executable);
            if !long_double {
                _ = compiler.arg("-DNO_LONG_DOUBLE");
            }
            if cfg!(unix) {
                _ = compiler.arg("-lm");
            }
            let built = compiler.output().unwrap();
            assert!(built.status.success(), "{}", text(&built.stderr));
            let reference = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(
                reference.status.code(),
                Some(0),
                "{}",
                text(&reference.stdout)
            );
            let expected = text(&reference.stdout);
            assert!(
                expected.lines().all(|line| line.ends_with(" 1")),
                "{expected}"
            );
            let targets = if long_double {
                vec!["x86_64-pc-windows-msvc"]
            } else {
                vec!["x86_64-w64-windows-gnu", "x86_64-unknown-linux-gnu"]
            };
            for target in targets {
                for optimize in ["--opt=none", "--opt=bcc"] {
                    let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
                    _ = command.arg(&source).args([
                        "--interpret",
                        &format!("--target={target}"),
                        optimize,
                    ]);
                    if !long_double {
                        _ = command.arg("-DNO_LONG_DOUBLE");
                    }
                    let result = run_native(&mut command).unwrap();
                    assert_eq!(
                        result.status.code(),
                        Some(0),
                        "{target} {optimize}: {}\n{}",
                        text(&result.stderr),
                        text(&result.stdout)
                    );
                    assert_eq!(text(&result.stdout), expected, "{target} {optimize}");
                }
            }
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// `tests/fixtures/programs/long_double_encodings.c` builds every x87
    /// encoding class from raw bytes, including unnormals, pseudo-denormals,
    /// pseudo-infinities and pseudo-NaNs, and prints the bits the `long
    /// double` libm functions give: the interpreter must print glibc's.
    #[test]
    fn long_double_encodings_match_glibc_bit_for_bit() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs/long_double_encodings.c");
        let source = source.to_str().unwrap();
        let reference = temporary("long-double-encodings.exe");
        let built = Command::new("target/llvm/bin/clang")
            .args([source, "-std=c99", "-O0", "-fno-builtin", "-lm", "-o"])
            .arg(&reference)
            .output()
            .unwrap();
        assert!(built.status.success(), "{}", text(&built.stderr));
        let expected = run_native(&mut Command::new(&reference)).unwrap();
        assert_eq!(expected.status.code(), Some(0));
        std::fs::remove_file(reference).unwrap();
        for optimize in ["--opt=none", "--opt=bcc"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.args([source, "--interpret", optimize]);
            let output = run_native(&mut command).unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{optimize}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), text(&expected.stdout), "{optimize}");
        }
    }

    /// `tests/fixtures/programs/long_double_math.c` passes values of every
    /// x87 encoding class through the transcendental `long double` libm
    /// functions and prints the result bits: the interpreter, which runs
    /// glibc's algorithms with the processor computing `f2xm1`, `fyl2x`,
    /// `fyl2xp1` and `fpatan`, must print what the program built against
    /// glibc prints on the same processor.
    #[test]
    fn long_double_transcendentals_match_glibc_bit_for_bit() {
        interpreter_matches_glibc("long_double_math.c", &[]);
    }

    /// `%Lf` reads the target's `long double` from a `va_list` after ABI
    /// lowering: an x87 value in the System V overflow area, a reference to
    /// one in a MinGW slot, and an inline binary64 in an MSVC slot.
    #[test]
    fn long_double_printf_reads_each_targets_variadic_representation() {
        let source = "typedef __builtin_va_list va_list;
int printf(const char *, ...);
int __vsprintf_chk(char *, int, unsigned long long, const char *, va_list);
int vprintf(const char *, va_list);
static void show(const char *format, ...) {
  va_list list; char text[64];
  __builtin_va_start(list, format);
  __vsprintf_chk(text, 1, sizeof text, format, list);
  __builtin_va_end(list);
  printf(\"%s\", text);
  __builtin_va_start(list, format);
  vprintf(format, list);
  __builtin_va_end(list);
}
int main(void) { volatile long double x = 1.5L; printf(\"%Lf %d \", x, 7);
  show(\"%Lf %d %Lg %Le\\n\", x, 8, x * 2, x / 4); return 0; }
";
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-pc-windows-msvc",
        ] {
            for optimize in ["--opt=none", "--opt=bcc"] {
                let output = bcc(source, &["--interpret", target, optimize]);
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{target} {optimize}: {}",
                    text(&output.stderr)
                );
                assert_eq!(
                    text(&output.stdout),
                    "1.500000 7 1.500000 8 3 3.750000e-01\n1.500000 8 3 3.750000e-01\n",
                    "{target} {optimize}"
                );
            }
        }
    }

    /// `tests/fixtures/programs/host_library.c` exercises the host's streams
    /// and files, `va_list` formatting, `scanf`, `qsort` and `bsearch` with
    /// interpreted comparisons (tracing the elements glibc compares), and its
    /// string, wide-string, character and conversion functions: with host
    /// files allowed, the interpreter must write what the program built
    /// against glibc writes, to stdout and to stderr.
    #[test]
    fn host_library_functions_match_glibc() {
        interpreter_matches_glibc("host_library.c", &["--interpret-allow-fs"]);
    }

    /// `tests/fixtures/programs/stdio_writes.c` checks when writes reach a
    /// file (held in the stream's buffer until it fills, `fflush`,
    /// `fflush(NULL)` in glibc's stream order, `fclose`, `freopen`, a seek or
    /// a read), how a failing write surfaces, `ftell` with bytes held, and
    /// `fgets` without room leaving the stream unoriented: the interpreter
    /// must write what the program built against glibc 2.39 to 2.43 writes.
    #[test]
    fn stdio_writes_match_glibc() {
        interpreter_matches_glibc("stdio_writes.c", &["--interpret-allow-fs"]);
    }

    /// At a normal end, after `main` returns or the program calls `exit`,
    /// glibc writes out what every stream holds, the most recently opened
    /// first; `abort`, even from a destructor, writes out nothing. The
    /// expected file contents are what glibc programs leave.
    #[test]
    fn interpreter_writes_out_streams_at_exit_in_glibc_order() {
        let path = temporary("exit-order.txt");
        let path_text = path.to_str().unwrap().replace('\\', "/");
        for (ending, expected) in [
            ("return 0;", "Abcdef"),
            ("exit(0);", "Abcdef"),
            ("abort();", "abcdef"),
            (
                "return 0; } __attribute__((destructor)) static void end(void) { abort();",
                "abcdef",
            ),
        ] {
            std::fs::write(&path, "abcdef").unwrap();
            let source = format!(
                "typedef struct _IO_FILE FILE; FILE *fopen(const char *, const char *);
int fputc(int, FILE *); void exit(int); void abort(void);
int main(void) {{ FILE *first = fopen(\"{path_text}\", \"r+\");
  FILE *second = fopen(\"{path_text}\", \"r+\"); fputc('A', first); fputc('B', second);
  {ending} }}
"
            );
            let output = bcc(&source, &["--interpret", "--interpret-allow-fs"]);
            assert!(
                matches!(output.status.code(), Some(0 | 134)),
                "{ending}: {}",
                text(&output.stderr)
            );
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                expected,
                "{ending}"
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    /// `tests/fixtures/programs/scanf_cases.c` runs the `sscanf` edge cases
    /// that glibc 2.39 to 2.43 agree on: matching and input failures,
    /// suppression, widths, scansets, white space and every form of floating
    /// input.
    #[test]
    fn scanf_edge_cases_match_glibc() {
        interpreter_matches_glibc("scanf_cases.c", &[]);
    }

    /// `tests/fixtures/programs/scanf_cases_modern_glibc.c` runs the `sscanf`
    /// cases that glibc 2.41 to 2.43 changed (NaN payloads, prefixes without
    /// digits, field widths): the interpreter follows 2.43, so it must write
    /// the checked-in `.expected` output on any glibc, and a host with glibc
    /// 2.43 or later must still write it too.
    #[test]
    fn scanf_edge_cases_match_modern_glibc() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let expected =
            std::fs::read_to_string(fixture_program("scanf_cases_modern_glibc.expected")).unwrap();
        if host_glibc_version() >= (2, 43) {
            let glibc = glibc_output("scanf_cases_modern_glibc.c");
            assert_eq!(text(&glibc.stdout), expected, "glibc");
        }
        interpreter_writes("scanf_cases_modern_glibc.c", &[], &expected, "");
    }

    /// `tests/fixtures/programs/long_double_modern_glibc.c` raises
    /// non-canonical x87 bases to the powers -3 to 3, which glibc 2.43's
    /// rewritten `powl` computes differently from 2.39: the interpreter
    /// follows 2.43, so it must write the checked-in `.expected` output on
    /// any glibc, and a host with glibc 2.43 or later must still write it
    /// too.
    #[test]
    fn long_double_powers_match_modern_glibc() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let expected =
            std::fs::read_to_string(fixture_program("long_double_modern_glibc.expected")).unwrap();
        if host_glibc_version() >= (2, 43) {
            let glibc = glibc_output("long_double_modern_glibc.c");
            assert_eq!(text(&glibc.stdout), expected, "glibc");
        }
        interpreter_writes("long_double_modern_glibc.c", &[], &expected, "");
    }

    /// `tests/fixtures/programs/stdio_modern_glibc.c` runs the stdio cases
    /// glibc changed after 2.39: flushing streams that read (2.42:
    /// `fflush(NULL)` gives back what they read ahead, `fflush` drops a
    /// pushback area) and a seek on an oriented stream without a buffer
    /// (2.41: it counts and drops the pushback area). The interpreter
    /// follows 2.43, so it must write the checked-in `.expected` output (and
    /// the redirected `stdout` case's report on stderr) on any glibc, and a
    /// host with glibc 2.43 or later must still write it too.
    #[test]
    fn stdio_matches_modern_glibc() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let expected =
            std::fs::read_to_string(fixture_program("stdio_modern_glibc.expected")).unwrap();
        let errors = "stdout 0 0 a\n";
        if host_glibc_version() >= (2, 43) {
            let glibc = glibc_output("stdio_modern_glibc.c");
            assert_eq!(text(&glibc.stdout), expected, "glibc");
            assert_eq!(text(&glibc.stderr), errors, "glibc");
        }
        interpreter_writes(
            "stdio_modern_glibc.c",
            &["--interpret-allow-fs"],
            &expected,
            errors,
        );
    }

    /// `tests/fixtures/programs/errno_cases.c` prints every `strerror`
    /// message, both `strerror_r` forms and `perror`, and the `errno` of
    /// failing conversions (C23 binary prefixes included), allocations,
    /// stream and file operations and math functions in each precision:
    /// the interpreter must print what glibc prints.
    #[test]
    fn errno_and_its_messages_match_glibc() {
        interpreter_matches_glibc("errno_cases.c", &["--interpret-allow-fs"]);
    }

    /// `tests/fixtures/programs/wide_stdio.c` uses wide streams, `fwide`,
    /// `swprintf`, the `wscanf` family and `%lc`/`%ls` in the narrow
    /// functions, in the "C" locale and in "C.UTF-8": the interpreter must
    /// print what glibc prints.
    #[test]
    fn wide_streams_match_glibc() {
        interpreter_matches_glibc("wide_stdio.c", &["--interpret-allow-fs"]);
    }

    /// `tests/fixtures/programs/gamma_functions.c` calls the `lgamma`
    /// family in each precision, with the sign each leaves in `signgam` or
    /// stores through its pointer, glibc's `gamma` names, and `tgamma`, with
    /// `errno`: the interpreter must print what glibc prints, the x87
    /// `lgammal` and `tgammal` bit for bit.
    #[test]
    fn gamma_functions_and_signgam_match_glibc() {
        interpreter_matches_glibc("gamma_functions.c", &[]);
    }

    /// `tests/fixtures/programs/exit_handlers.c` ends in each way glibc
    /// offers: the `atexit` and `on_exit` handlers in reverse order around
    /// the destructors, then the streams written out, for a return from
    /// `main` and for `exit`, also when a handler exits again; the
    /// `at_quick_exit` handlers and nothing written out for `quick_exit`;
    /// nothing at all for `_Exit` and `_exit`.
    #[test]
    fn exit_functions_end_programs_as_glibc_does() {
        for mode in 0..=7 {
            merged_mode_matches_glibc("exit_handlers.c", mode);
        }
    }

    /// `tests/fixtures/programs/stdio_buffering.c` sets the buffering of the
    /// standard streams and a file in each way `setvbuf`, `setbuf`,
    /// `setbuffer` and `setlinebuf` can: the output reaching one file must
    /// interleave as glibc's does, and the file's contents and `ftell` must
    /// agree at each step.
    #[test]
    fn stream_buffering_modes_match_glibc() {
        for mode in 0..=12 {
            merged_mode_matches_glibc("stdio_buffering.c", mode);
        }
    }

    /// `tests/fixtures/programs/posix_descriptors.c` mixes descriptors with
    /// the streams: `write(1)` and `write(2)` against buffered `stdout` and
    /// `stderr`, `dup2` and `close` under `stdout`, `freopen`, `fdopen`,
    /// `fileno`, files and pipes through descriptors, and `errno` on each
    /// failure. With stdout and stderr in one file, the interpreter must
    /// write what glibc writes and leave the same file.
    #[test]
    fn descriptors_and_streams_match_glibc() {
        for mode in 0..=8 {
            merged_mode_matches_glibc("posix_descriptors.c", mode);
        }
    }

    /// `tests/fixtures/programs/posix_console.c`: on pipes, as the tests run
    /// programs, `fstat`, `lseek` and `isatty` of descriptors 1 and 2 report
    /// what glibc's do, and `write(1)` overtakes buffered `stdout`.
    #[test]
    fn console_descriptors_match_glibc_on_pipes() {
        interpreter_matches_glibc("posix_console.c", &[]);
    }

    /// The program is a pipe's only process, so reading an empty pipe whose
    /// write end is open, or filling one, would wait forever, and writing to
    /// one nobody reads raises `SIGPIPE`: the interpreter stops the program,
    /// through a descriptor or a stream. Without `--interpret-allow-fs`,
    /// `open`, `stat` and `unlink` fail with `EACCES` and are noted, while
    /// the console's descriptors and pipes still work.
    #[test]
    fn descriptors_stop_on_dead_pipes_and_need_file_access() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let prelude = "typedef struct _IO_FILE FILE; int printf(const char *, ...);
int pipe(int *); long read(int, void *, unsigned long); long write(int, const void *, unsigned \
                       long);
int close(int); FILE *fdopen(int, const char *); int fputs(const char *, FILE *);
int fflush(FILE *); int fgetc(FILE *);
";
        for (body, message) in [
            ("char c; read(p[0], &c, 1);", "wait forever"),
            (
                "static char big[100000]; write(p[1], big, sizeof big);",
                "wait forever",
            ),
            ("close(p[0]); write(p[1], \"x\", 1);", "SIGPIPE"),
            ("fgetc(fdopen(p[0], \"r\"));", "wait forever"),
            (
                "close(p[0]); FILE *f = fdopen(p[1], \"w\"); fputs(\"x\", f); fflush(f);",
                "SIGPIPE",
            ),
        ] {
            let source =
                format!("{prelude}int main(void) {{ int p[2]; pipe(p); {body} return 0; }}\n");
            let output = bcc(&source, &["--interpret"]);
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(70), "{body}: {stderr}");
            assert!(stderr.contains(message), "{body}: {stderr}");
        }
        let path = temporary("descriptor-access.txt");
        std::fs::write(&path, "kept\n").unwrap();
        let path_text = path.to_str().unwrap();
        let source = format!(
            "int printf(const char *, ...); int open(const char *, int, ...); int \
             *__errno_location(void);
struct stat {{ char bytes[144]; }}; int stat(const char *, struct stat *); int unlink(const char \
             *);
long write(int, const void *, unsigned long); int pipe(int *);
int main(void) {{ const char *name = \"{path_text}\"; struct stat st; int p[2];
  int fd = open(name, 01 | 01000); int e1 = *__errno_location();
  int s = stat(name, &st); int u = unlink(name); int e2 = *__errno_location();
  printf(\"%d %d %d %d %d %ld %d\\n\", fd, e1, s, u, e2, write(1, \"\", 0), pipe(p)); return 0; }}
"
        );
        let output = bcc(&source, &["--interpret"]);
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "-1 13 -1 -1 13 0 0\n");
        let stderr = text(&output.stderr);
        assert!(stderr.contains("refused 3 host file accesses"), "{stderr}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept\n");
        let output = bcc(&source, &["--interpret", "--interpret-allow-fs"]);
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "3 0 0 0 0 0 0\n");
        assert!(!path.exists());
    }

    /// A byte function on a wide-oriented stream, or a wide one on a
    /// byte-oriented stream, is undefined and traps.
    #[test]
    fn mixing_byte_and_wide_functions_on_a_stream_traps() {
        let prelude = "typedef struct _IO_FILE FILE; extern FILE *stdout; typedef int wchar_t;
int printf(const char *, ...); int wprintf(const wchar_t *, ...);
unsigned fputwc(wchar_t, FILE *); int fputc(int, FILE *);
";
        for body in [
            "wprintf(L\"wide\"); printf(\"narrow\");",
            "printf(\"narrow\"); fputwc(L'w', stdout);",
            "fputwc(L'w', stdout); fputc('n', stdout);",
        ] {
            let source = format!("{prelude}int main(void) {{ {body} return 0; }}\n");
            let output = bcc(&source, &["--interpret"]);
            assert_eq!(output.status.code(), Some(70), "{body}");
            assert!(
                text(&output.stderr).contains("wide-oriented stream"),
                "{body}: {}",
                text(&output.stderr)
            );
        }
    }

    /// Builds a fixture with clang against glibc, with `MODE` defined to
    /// `mode` and `PATH` to a temporary file, and checks that the
    /// interpreter, under both optimization settings, writes the same text
    /// to stdout and stderr sharing one file, exits with the same status,
    /// and leaves the same contents at `PATH`. GNU/Linux hosts only.
    fn merged_mode_matches_glibc(fixture: &str, mode: u32) {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let source = fixture_program(fixture);
        let path = temporary("mode-file.txt");
        let defines = [
            format!("-DMODE={mode}"),
            format!("-DPATH=\"{}\"", path.display()),
        ];
        let reference = temporary("mode-reference.exe");
        let built = Command::new("target/llvm/bin/clang")
            .arg(&source)
            .args(["-std=c99", "-O0", "-fno-builtin", "-w", "-lm", "-o"])
            .arg(&reference)
            .args(&defines)
            .output()
            .unwrap();
        assert!(built.status.success(), "{}", text(&built.stderr));
        // Runs `program` with stdout and stderr in one file: its status,
        // what the file holds, and what it left at `PATH`.
        let run = |program: &mut Command| {
            let merged = temporary("mode-output.txt");
            let mut shell = Command::new("sh");
            _ = shell
                .args(["-c", "exec \"$@\" > \"$0\" 2>&1"])
                .arg(&merged)
                .arg(program.get_program())
                .args(program.get_args());
            let status = run_native(&mut shell).unwrap().status.code();
            let output = std::fs::read_to_string(&merged).unwrap();
            std::fs::remove_file(&merged).unwrap();
            let file = std::fs::read_to_string(&path).unwrap_or_default();
            _ = std::fs::remove_file(&path).ok();
            (status, output, file)
        };
        let expected = run(&mut Command::new(&reference));
        std::fs::remove_file(reference).unwrap();
        for optimize in ["--opt=none", "--opt=bcc"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command
                .arg(&source)
                .args(["--interpret", optimize, "--interpret-allow-fs"])
                .args(&defines);
            assert_eq!(
                run(&mut command),
                expected,
                "{fixture} MODE={mode} {optimize}"
            );
        }
    }

    /// Builds a fixture with clang against glibc and checks that the
    /// interpreter, with `flags`, writes the same stdout and stderr under
    /// both optimization settings. GNU/Linux hosts only.
    fn interpreter_matches_glibc(fixture: &str, flags: &[&str]) {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let expected = glibc_output(fixture);
        interpreter_writes(
            fixture,
            flags,
            &text(&expected.stdout),
            &text(&expected.stderr),
        );
    }

    /// What a fixture built with clang against the host's glibc writes.
    fn glibc_output(fixture: &str) -> Output {
        let source = fixture_program(fixture);
        let reference = temporary("glibc-reference.exe");
        let built = Command::new("target/llvm/bin/clang")
            .arg(&source)
            .args(["-std=c99", "-O0", "-fno-builtin", "-w", "-lm", "-o"])
            .arg(&reference)
            .output()
            .unwrap();
        assert!(built.status.success(), "{}", text(&built.stderr));
        let output = run_native(&mut Command::new(&reference)).unwrap();
        assert_eq!(output.status.code(), Some(0));
        std::fs::remove_file(reference).unwrap();
        output
    }

    /// Checks that the interpreter, with `flags`, runs a fixture to exit
    /// status 0 writing `stdout` and `stderr` under both optimization
    /// settings.
    fn interpreter_writes(fixture: &str, flags: &[&str], stdout: &str, stderr: &str) {
        let source = fixture_program(fixture);
        for optimize in ["--opt=none", "--opt=bcc"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command
                .arg(&source)
                .args(["--interpret", optimize])
                .args(flags);
            let output = run_native(&mut command).unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{fixture} {optimize}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), stdout, "{fixture} {optimize}");
            assert_eq!(text(&output.stderr), stderr, "{fixture} {optimize}");
        }
    }

    /// The path of `tests/fixtures/programs/{fixture}`.
    fn fixture_program(fixture: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs")
            .join(fixture)
    }

    /// The host glibc's (major, minor) version, from
    /// `getconf GNU_LIBC_VERSION` ("glibc 2.39").
    fn host_glibc_version() -> (u32, u32) {
        let output = Command::new("getconf")
            .arg("GNU_LIBC_VERSION")
            .output()
            .unwrap();
        let version = text(&output.stdout);
        let (major, minor) = version
            .trim()
            .strip_prefix("glibc ")
            .and_then(|version| version.split_once('.'))
            .unwrap_or_else(|| panic!("glibc version {version:?}"));
        let minor = minor.split('.').next().unwrap();
        (major.parse().unwrap(), minor.parse().unwrap())
    }

    /// Host files are opt-in. Without `--interpret-allow-fs`, `fopen` in any
    /// mode and `remove` fail as for a permission error, the file system is
    /// untouched, and the interpreter notes the refusal; with it, they act.
    #[test]
    fn interpreter_file_access_is_opt_in() {
        let path = temporary("file-access.txt");
        std::fs::write(&path, "kept\n").unwrap();
        let path_text = path.to_str().unwrap().replace('\\', "/");
        let source = format!(
            "typedef struct _IO_FILE FILE; int printf(const char *, ...);
FILE *fopen(const char *, const char *); int fclose(FILE *); int remove(const char *);
int fputs(const char *, FILE *);
int main(void) {{ const char *name = \"{path_text}\";
  FILE *read = fopen(name, \"r\"); FILE *write = fopen(name, \"w\");
  printf(\"%d %d \", read != 0, write != 0);
  if (write) {{ fputs(\"replaced\\n\", write); fclose(write); }}
  if (read) fclose(read);
  printf(\"%d\\n\", remove(name)); return 0; }}
"
        );
        let output = bcc(&source, &["--interpret"]);
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "0 0 -1\n");
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains("refused 3 host file accesses")
                && stderr.contains("--interpret-allow-fs"),
            "{stderr}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept\n");
        let output = bcc(&source, &["--interpret", "--interpret-allow-fs"]);
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "1 1 0\n");
        assert!(!text(&output.stderr).contains("refused"));
        assert!(!path.exists());
    }

    /// stdout and stderr keep glibc's order when both go to one file. stderr
    /// is unbuffered. stdout to a pipe or file appears at `fflush`, when it
    /// is closed or reopened (even by a failed `freopen`), and at exit; a
    /// terminal's stdout, which glibc line-buffers, also at each newline.
    /// The expected text is what glibc programs print, through a pipe and
    /// through a pseudo-terminal.
    #[test]
    fn interpreter_interleaves_stdout_and_stderr_as_glibc_buffers() {
        let prelude = "typedef struct _IO_FILE FILE; extern FILE *stdout, *stderr;
int fputs(const char *, FILE *); int fflush(FILE *); int puts(const char *);
int fclose(FILE *); FILE *freopen(const char *, const char *, FILE *);
";
        let lines = "fputs(\"A\\n\", stdout); fputs(\"B\", stderr); fputs(\"C\", stdout);
  fputs(\"D\", stderr); fputs(\"E\\n\", stdout);";
        for (body, flags, expected) in [
            (
                "fputs(\"A\", stdout); fflush(stdout); fputs(\"B\", stderr); puts(\"C\");
  fputs(\"D\", stderr); fflush(0); fputs(\"E\", stderr); fputs(\"F\", stdout);",
                &[][..],
                "ABDC\nEF",
            ),
            (lines, &[][..], "BDA\nCE\n"),
            (lines, &["--interpret-terminal-stdout"][..], "A\nBDCE\n"),
            (
                "fputs(\"A\", stdout); fclose(stdout); fputs(\"B\", stderr);",
                &[][..],
                "AB",
            ),
            (
                "fputs(\"A\", stdout); FILE *f = freopen(\"/nonexistent-bcc/x\", \"w\", stdout);
  fputs(f ? \"F\" : \"B\", stderr);",
                &["--interpret-allow-fs"][..],
                "AB",
            ),
        ] {
            let source = format!("{prelude}int main(void) {{ {body} return 0; }}\n");
            assert_eq!(
                interpret_merged(&source, flags, b""),
                expected,
                "{body} {flags:?}"
            );
        }
    }

    /// glibc flushes a terminal's line-buffered stdout before input only
    /// when a line-buffered (terminal) stdin has to be refilled: not when
    /// stdin is a pipe, not while its buffer still holds the rest of a line,
    /// and not once it has reached end-of-file, which stays set. The
    /// expected text is what glibc programs print with stdout and stderr on
    /// a pseudo-terminal or one pipe, and stdin a pipe or a pseudo-terminal
    /// that receives the whole input as one line and then end-of-file.
    #[test]
    fn interpreter_flushes_terminal_stdout_when_terminal_stdin_refills() {
        let prelude = "typedef struct _IO_FILE FILE; extern FILE *stdout, *stderr;
int fputs(const char *, FILE *); int getchar(void);
";
        let stdout = "--interpret-terminal-stdout";
        let stdin = "--interpret-terminal-stdin";
        let once = "fputs(\"A\", stdout); getchar(); fputs(\"B\", stderr);";
        for (body, flags, expected) in [
            (once, &[stdout][..], "BA"),
            (once, &[stdin][..], "BA"),
            (once, &[stdout, stdin][..], "AB"),
            (
                "fputs(\"A\", stdout); getchar(); fputs(\"B\", stdout); getchar();
  fputs(\"C\", stderr);",
                &[stdout, stdin][..],
                "ACB",
            ),
            (
                "fputs(\"A\", stdout); getchar(); getchar(); getchar(); fputs(\"B\", stdout);
  getchar(); fputs(\"C\", stderr); fputs(\"D\", stdout); getchar(); fputs(\"E\", stderr);",
                &[stdout, stdin][..],
                "ABCED",
            ),
        ] {
            let source = format!("{prelude}int main(void) {{ {body} return 0; }}\n");
            assert_eq!(
                interpret_merged(&source, flags, b"xy\n"),
                expected,
                "{body} {flags:?}"
            );
        }
        // An `fread` of at least the terminal's 1024-byte buffer reads the
        // line straight into the array, without a refill to flush before;
        // a smaller one refills.
        let mut line = vec![b'x'; 1023];
        line.push(b'\n');
        for (size, expected) in [(1024, "BA"), (1023, "AB")] {
            let source = format!(
                "{prelude}typedef unsigned long size_t; size_t fread(void *, size_t, size_t, FILE \
                 *);
extern FILE *stdin;
int main(void) {{ static char buffer[1024]; fputs(\"A\", stdout);
  fread(buffer, 1, {size}, stdin); fputs(\"B\", stderr); return 0; }}\n"
            );
            assert_eq!(
                interpret_merged(&source, &[stdout, stdin], &line),
                expected,
                "fread {size}"
            );
        }
    }

    /// Interprets `source` with `flags` and `input` piped to its stdin, with
    /// stdout and stderr both going to one file; what the file holds.
    fn interpret_merged(source: &str, flags: &[&str], input: &[u8]) -> String {
        use std::{
            io::Write as _,
            process::Stdio,
        };
        let merged = temporary("merged-output.txt");
        let file = std::fs::File::create(&merged).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["--interpret", "--target=x86_64-unknown-linux-gnu"])
            .args(flags)
            .args(["--input", source])
            .stdin(Stdio::piped())
            .stdout(file.try_clone().unwrap())
            .stderr(file)
            .spawn()
            .unwrap();
        // One write, so that one `read` sees it whole as a terminal's would.
        // A program that failed to compile has closed the pipe; its status
        // tells.
        _ = child.stdin.take().unwrap().write_all(input).ok();
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(0), "{source}");
        let text = std::fs::read_to_string(&merged).unwrap();
        std::fs::remove_file(merged).unwrap();
        text
    }

    /// Using a closed stream is undefined: after `fclose(stdin)` or
    /// `fclose(stdout)`, the functions that use them implicitly trap, and a
    /// failed `freopen` closes its stream, whose stale pointer stays invalid
    /// when a later `fopen` reuses its table slot.
    #[test]
    fn closed_streams_trap_even_when_used_implicitly() {
        let path = temporary("reused-slot.txt");
        let path_text = path.to_str().unwrap().replace('\\', "/");
        let prelude = "typedef struct _IO_FILE FILE; extern FILE *stdin, *stdout;
int fclose(FILE *); int getchar(void); int puts(const char *); int printf(const char *, ...);
int scanf(const char *, ...) __asm__(\"__isoc99_scanf\"); int fputs(const char *, FILE *);
FILE *fopen(const char *, const char *); FILE *freopen(const char *, const char *, FILE *);
";
        for body in [
            "fclose(stdin); return getchar();".to_string(),
            "int x; fclose(stdin); return scanf(\"%d\", &x);".to_string(),
            "fclose(stdout); return puts(\"x\");".to_string(),
            "fclose(stdout); return printf(\"x\");".to_string(),
            format!(
                "FILE *a = fopen(\"{path_text}\", \"w\"); FILE *b = \
                 freopen(\"/nonexistent-bcc/x\", \"r\", a);
  FILE *c = fopen(\"{path_text}\", \"r\"); if (b || !c) return 1; return fputs(\"stale\", a);"
            ),
        ] {
            let source = format!("{prelude}int main(void) {{ {body} }}\n");
            let output = bcc(&source, &["--interpret", "--interpret-allow-fs"]);
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(70), "{body}: {stderr}");
            assert!(stderr.contains("not an open FILE"), "{body}: {stderr}");
        }
        std::fs::remove_file(path).unwrap();
    }

    /// `%lf` stores a `double` on every target, Win64's 32-bit `long`
    /// notwithstanding; `long double` input (`%Lf`, and `%llf` and `%qf`,
    /// which glibc reads alike) is refused rather than run as a `double`
    /// conversion (the wide conversions run, as `wide_streams_match_glibc`
    /// checks); and a wide-string length whose byte size overflows is out
    /// of bounds rather than a short copy.
    #[test]
    fn scanf_lengths_and_wide_sizes_follow_the_conversion() {
        let source = "int sscanf(const char *, const char *, ...) __asm__(\"__isoc99_sscanf\");
int printf(const char *, ...);
int main(void) { double d = 0; long l = 0; float f = 0;
  int n = sscanf(\"2.5 -7 0.25\", \"%lf %ld %f\", &d, &l, &f);
  printf(\"%d %g %ld %g\\n\", n, d, l, (double)f); return 0; }
";
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-pc-windows-msvc",
        ] {
            let output = bcc(source, &["--interpret", target]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{target}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), "3 2.5 -7 0.25\n", "{target}");
        }
        for conversion in ["%Lf", "%llf", "%qf"] {
            let source = format!(
                "int sscanf(const char *, const char *, ...) __asm__(\"__isoc99_sscanf\");
int main(void) {{ long double buffer[4]; return sscanf(\"a\", \"{conversion}\", buffer); }}
"
            );
            let output = bcc(&source, &["--interpret"]);
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(70), "{conversion}: {stderr}");
            assert!(
                stderr.contains("scanf conversion the host does not implement"),
                "{stderr}"
            );
        }
        // Guest-controlled exponents, payloads and offsets saturate or fail;
        // they never overflow the interpreter's own arithmetic.
        let source = "typedef struct _IO_FILE FILE; extern FILE *stdin;
int sscanf(const char *, const char *, ...) __asm__(\"__isoc99_sscanf\");
int printf(const char *, ...); int ungetc(int, FILE *); int fseek(FILE *, long, int);
void *memcpy(void *, const void *, unsigned long);
int main(void) { double d = 1, e = 1; unsigned long long bits;
  int n = sscanf(\"0x0.0p-9223372036854775808 nan(18446744073709551616)\", \"%lf %lf\", &d, &e);
  memcpy(&bits, &e, 8); ungetc('x', stdin);
  printf(\"%d %a %llx %d\\n\", n, d, bits, fseek(stdin, -9223372036854775807L - 1, 1));
  return 0; }
";
        // stdin is a pipe, which cannot seek; glibc's seek on /dev/null
        // would succeed.
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["--interpret", "--target=x86_64-unknown-linux-gnu"])
            .args(["--input", source])
            .stdin(std::process::Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "2 0x0p+0 7fffffffffffffff -1\n");
        let source = "typedef int wchar_t; wchar_t *wmemcpy(wchar_t *, const wchar_t *, unsigned \
                      long);
int main(void) { wchar_t a[2] = {1, 2}, b[2]; wmemcpy(b, a, 1ULL << 62); return b[0]; }
";
        let output = bcc(
            source,
            &["--interpret", "--target=x86_64-unknown-linux-gnu"],
        );
        let stderr = text(&output.stderr);
        assert_eq!(output.status.code(), Some(70), "{stderr}");
        assert!(stderr.contains("out-of-bounds access"), "{stderr}");
    }

    /// The interpreter computes the x87 `long double` functions, including
    /// the transcendental ones (and `tgammal`, built on them) on an x86-64
    /// host, and refuses those it does not implement, such as `cabsl`.
    #[test]
    fn host_math_runs_x87_functions_and_refuses_the_rest() {
        for target in [
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-unknown-linux-gnu",
        ] {
            let mut sources = vec![
                "long double fabsl(long double); int main(void) { return fabsl(-1.5L)!=1.5L; }",
                "float nexttowardf(float,long double); int main(void) { return \
                 nexttowardf(1.0f,2.0L)<=1.0f; }",
                "long double sinl(long double); int main(void) { return sinl(1.0L) <= 0.84L; }",
            ];
            if cfg!(target_arch = "x86_64") {
                sources.push(
                    "long double expl(long double); int main(void) { return expl(1.0L) <= 2.718L; \
                     }",
                );
                sources.push(
                    "long double tgammal(long double); int main(void) { return tgammal(3.0L) != \
                     2.0L; }",
                );
            }
            for source in sources {
                let output = bcc(source, &["--interpret", target]);
                assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
            }
            let output = bcc(
                "long double cabsl(long double _Complex); int main(void) { long double _Complex z \
                 = 3.0L; return cabsl(z) > 0; }",
                &["--interpret", target],
            );
            assert_eq!(output.status.code(), Some(70), "{}", text(&output.stderr));
            assert!(
                text(&output.stderr).contains("f80 or f128 library function"),
                "{}",
                text(&output.stderr)
            );
        }
    }

    /// `tests/fixtures/programs/long_double.c` prints x87 arithmetic,
    /// conversions, comparisons, the exact `<math.h>` functions, `va_arg` and
    /// every `printf` conversion of `long double`: the interpreter, bcc
    /// through LLVM and bcc's native back end must print what the program
    /// built by clang prints, digit for digit. glibc's formatting is the
    /// reference, so this runs on GNU/Linux hosts.
    #[test]
    fn long_double_programs_match_native_output_on_every_arm() {
        if host() != Some("x86_64-unknown-linux-gnu") {
            return;
        }
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/long_double.c");
        let source = source.to_str().unwrap();
        let reference = temporary("long-double-reference.exe");
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
        for optimize in ["--opt=none", "--opt=bcc"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.args([source, "--interpret", optimize]);
            let output = run_native(&mut command).unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{optimize}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), expected, "interpreter {optimize}");
        }
        let executable = temporary("long-double.exe");
        for arm in [
            &["--llvm-opt=O0"][..],
            &["--opt=bcc", "--llvm-opt=O2"],
            &["--backend=native"],
            &["--opt=bcc", "--backend=native"],
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.arg(source).args(arm).arg("-o").arg(&executable);
            let built = command.output().unwrap();
            assert!(built.status.success(), "{arm:?}: {}", text(&built.stderr));
            let output = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(output.status.code(), Some(0), "{arm:?}");
            assert_eq!(text(&output.stdout), expected, "{arm:?}");
        }
        std::fs::remove_file(executable).unwrap();
    }

    #[test]
    fn floating_output_formats_values_beyond_the_integer_digit_buffer() {
        let output = bcc(
            "int printf(const char *,...); int main(void) {
printf(\"%f %f %F\\n\",1e100,-0.0,1.0/0.0); return 0; }",
            &["--interpret"],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        let output = text(&output.stdout);
        assert!(
            output.len() > 100 && output.ends_with(".000000 -0.000000 INF\n"),
            "{output}"
        );
    }

    #[test]
    fn aggregate_fetch_abi_refusal_has_owner_and_source_location() {
        let source = "struct Huge { char bytes[4294967296ULL]; };\nint fetch(int last, ...) \
                      {\n__builtin_va_list ap; \
                      __builtin_va_start(ap,last);\n__builtin_va_arg(ap,struct Huge); \
                      __builtin_va_end(ap); return 0;\n}\n";
        for (source, owner, line) in [
            (source.to_owned(), "fetch", 2),
            (
                source.replace(
                    "int fetch(int last, ...) {",
                    "int fetch(int last, ...) __asm__(\"alias\");\nint fetch(int last, ...) {",
                ),
                "alias",
                3,
            ),
        ] {
            for target in [
                "x86_64-unknown-linux-gnu",
                "x86_64-w64-windows-gnu",
                "x86_64-pc-windows-msvc",
            ] {
                for optimize in ["--opt=none", "--opt=bcc"] {
                    let output = bcc(
                        &source,
                        &[&format!("--target={target}"), optimize, "--emit=ir-abi"],
                    );
                    let stderr = text(&output.stderr);
                    assert_eq!(output.status.code(), Some(1), "{stderr}");
                    assert!(
                        stderr.contains(&format!("@{owner}: aggregate size 4294967296")),
                        "{stderr}"
                    );
                    assert!(stderr.contains(&format!("<input>:{line}:5")), "{stderr}");
                    assert_eq!(output.stdout.as_slice(), b"");
                }
            }
        }
    }

    /// glibc's Clang path implements `fpclassify`, `isfinite`, `isnormal`,
    /// `isnan` and `isinf` with these builtins; MinGW's passes the frame
    /// address to `_setjmp`.
    #[test]
    fn classification_and_frame_address_builtins_run_in_both_back_ends() {
        let Some(target) = host() else {
            return;
        };
        let target = format!("--target={target}");
        let check = |ty: &str, subnormal: &str| {
            format!(
                "  {{ volatile {ty} zero = 0, one = 1, tiny = {subnormal};\n    {ty} nan = zero / \
                 zero, inf = one / zero, values[] = {{ nan, -inf, inf, one, -tiny, zero, -zero \
                 }};\n    int expected[] = {{ 0, 1, 1, 2, 3, 4, 4 }};\n    for (int i = 0; i < 7; \
                 i++) {{\n      {ty} v = values[i];\n      int c = __builtin_fpclassify(0, 1, 2, \
                 3, 4, v);\n      if (c != expected[i] || __builtin_isnan(v) != (c == 0)\n        \
                 || __builtin_isfinite(v) != (c > 1) || __builtin_isnormal(v) != (c == 2)\n        \
                 || __builtin_isinf_sign(v) != (c == 1 ? (v < 0 ? -1 : 1) : 0))\n        return \
                 failures + i + 1;\n    }}\n    failures += 10; }}\n"
            )
        };
        let program = |long_double: bool| {
            let mut source = String::from(
                "int main(void) {\n  int failures = -20;\n  void *frame = \
                 __builtin_frame_address(0);\n  void *ret = \
                 __builtin_extract_return_addr(__builtin_return_address(0));\n  if (!frame || \
                 !ret) return 100;\n",
            );
            source.push_str(&check("float", "1e-40f"));
            source.push_str(&check("double", "1e-310"));
            if long_double {
                source.push_str(&check("long double", "1e-4940L"));
                source.push_str("  failures -= 10;\n");
            }
            source.push_str("  return failures;\n}\n");
            source
        };
        for opt in ["--opt=none", "--opt=bcc"] {
            let output = bcc(
                &program(!target.ends_with("msvc")),
                &[&target, opt, "--interpret"],
            );
            assert!(output.status.success(), "{opt}: {}", text(&output.stderr));
        }
        let executable = temporary("classify.exe");
        for opt in [&["--llvm-opt=O0"][..], &["--opt=bcc", "--llvm-opt=O2"][..]] {
            // MSVC's long double is binary64, where the x87 subnormal is 0.
            let built = bcc(
                &program(!target.ends_with("msvc")),
                &[&[&target[..], "-o", executable.to_str().unwrap()][..], opt].concat(),
            );
            assert!(built.status.success(), "{opt:?}: {}", text(&built.stderr));
            let output = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(output.status.code(), Some(0), "{opt:?}");
        }
        std::fs::remove_file(executable).unwrap();
    }

    /// C11 atomics under contention
    /// (`tests/fixtures/programs/atomic_threads.c`): counters that only atomic
    /// read-modify-writes keep exact, a compare-exchange loop, an `atomic_flag`
    /// spin lock and a libatomic `__int128`, with one, four and eight threads,
    /// through LLVM and the native back end. The driver links `-latomic` for
    /// the `__int128`. The command line has no `-pthread` yet, so the test
    /// needs a C library that holds the threads functions itself (glibc 2.34
    /// and later, musl), which a probe link through the LLVM back end checks.
    /// C11: §7.17.7, pp. 282-285; PDF pp. 300-303.
    #[test]
    fn atomic_counters_stay_exact_across_threads() {
        if host() != Some("x86_64-unknown-linux-gnu")
            || !Path::new("/usr/include/pthread.h").is_file()
        {
            eprintln!("skipping: no Linux host C library with pthreads");
            return;
        }
        let host_flags = [
            "--target=x86_64-unknown-linux-gnu",
            "--idirafter",
            "/usr/include/x86_64-linux-gnu",
            "--idirafter",
            "/usr/include",
        ];
        let probe = temporary("pthread-probe.exe");
        let probed = bcc(
            "#include <pthread.h>\nstatic void *f(void *p) { return p; }\nint main(void) { \
             pthread_t t; if (pthread_create(&t, 0, f, 0)) return 1; return pthread_join(t, 0); }",
            &[&host_flags[..], &["-o", probe.to_str().unwrap()]].concat(),
        );
        if !probed.status.success() && text(&probed.stderr).contains("undefined symbol: pthread_") {
            eprintln!(
                "skipping: the C library does not link pthreads without -pthread:\n{}",
                text(&probed.stderr)
            );
            return;
        }
        assert!(probed.status.success(), "{}", text(&probed.stderr));
        std::fs::remove_file(&probe).unwrap();
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/atomic_threads.c");
        let executable = temporary("atomic-threads.exe");
        let path = executable.to_str().unwrap();
        for backend in [
            &["--backend=llvm", "--llvm-opt=O0"][..],
            &["--backend=llvm", "--opt=bcc", "--llvm-opt=O2"][..],
            &["--backend=native", "--opt=none"][..],
            &["--backend=native", "--opt=bcc"][..],
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command
                .arg(&source)
                .args(host_flags)
                .args(["-o", path])
                .args(backend)
                .env("NO_COLOR", "1");
            let built = run_native(&mut command).unwrap();
            assert!(
                built.status.success(),
                "{backend:?}: {}",
                text(&built.stderr)
            );
            for threads in ["1", "4", "8"] {
                let output = run_native(Command::new(&executable).arg(threads)).unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{backend:?} with {threads} threads: {}",
                    text(&output.stdout)
                );
            }
        }
        std::fs::remove_file(&executable).unwrap();
    }

    /// `setjmp` and `longjmp` (#50) from `tests/fixtures/programs/setjmp.c`:
    /// MinGW-w64's declarations, whose `setjmp` passes `mingw_getsp()`, in
    /// the interpreter for the Windows target; the host's `<setjmp.h>` in the
    /// interpreter and in native executables built through LLVM and through
    /// the native back end.
    #[test]
    fn setjmp_and_longjmp_run_in_the_interpreter_and_natively() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/setjmp.c");
        let compile = |flags: &[&str]| {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.arg(&source).args(flags).env("NO_COLOR", "1");
            run_native(&mut command).unwrap()
        };
        for opt in ["--opt=none", "--opt=bcc"] {
            let output = compile(&[
                "--interpret",
                "--target=x86_64-w64-windows-gnu",
                "-DMINGW_DECLARATIONS",
                opt,
            ]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{opt}: {}",
                text(&output.stderr)
            );
        }
        // bcc has no default C library directories; take the host's.
        let (target, include) = match host() {
            | Some(target @ "x86_64-unknown-linux-gnu")
                if Path::new("/usr/include/setjmp.h").is_file() =>
                (
                    target,
                    vec![
                        "/usr/include/x86_64-linux-gnu".to_owned(),
                        "/usr/include".to_owned(),
                    ],
                ),
            | Some(target @ "x86_64-w64-windows-gnu") => {
                let Some((_, include)) = mingw() else {
                    eprintln!("skipping native runs: no MinGW-w64 gcc on PATH");
                    return;
                };
                (target, vec![include.to_str().unwrap().to_owned()])
            },
            | _ => {
                eprintln!("skipping native runs: no supported host C library");
                return;
            },
        };
        let mut flags = vec![format!("--target={target}")];
        for directory in include {
            flags.extend(["--idirafter".to_owned(), directory]);
        }
        let flags: Vec<&str> = flags.iter().map(String::as_str).collect();
        let target = &flags[..];
        for opt in ["--opt=none", "--opt=bcc"] {
            let output = compile(&[&["--interpret", opt][..], target].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{opt}: {}",
                text(&output.stderr)
            );
        }
        let executable = temporary("setjmp.exe");
        let path = executable.to_str().unwrap();
        for opt in [
            &["--llvm-opt=O0"][..],
            &["--opt=bcc", "--llvm-opt=O2"][..],
            &["--backend=native", "--opt=none"][..],
            &["--backend=native", "--opt=bcc"][..],
        ] {
            let built = compile(&[target, &["-o", path][..], opt].concat());
            assert!(built.status.success(), "{opt:?}: {}", text(&built.stderr));
            let output = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(output.status.code(), Some(0), "{opt:?}");
        }
        std::fs::remove_file(&executable).unwrap();
    }

    /// GCC's `__builtin_setjmp` and `__builtin_longjmp` from
    /// `tests/fixtures/programs/builtin_setjmp.c` print what GCC 15 and
    /// Clang print at -O0 and -O2, in the interpreter and in executables
    /// built through LLVM and through the native back end.
    #[test]
    fn builtin_setjmp_and_longjmp_match_gcc_on_every_back_end() {
        const TRANSCRIPT: &str = "many: 268935754\ncaller: 808336096\nrepeated: 1000\nloop body: \
                                  10045\narrays: 1649\ncopied: 42\nnested: 40\nvalue: \
                                  7\nfailures: 0\n";
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/builtin_setjmp.c");
        let compile = |flags: &[&str]| {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.arg(&source).args(flags).env("NO_COLOR", "1");
            run_native(&mut command).unwrap()
        };
        for opt in ["--opt=none", "--opt=bcc"] {
            let output = compile(&["--interpret", opt]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{opt}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), TRANSCRIPT, "{opt}");
        }
        let Some(target) = host() else {
            return;
        };
        let target = format!("--target={target}");
        let executable = temporary("builtin-setjmp.exe");
        let path = executable.to_str().unwrap();
        for opt in [
            &["--llvm-opt=O0"][..],
            &["--opt=bcc", "--llvm-opt=O2"][..],
            &["--backend=native", "--opt=none"][..],
            &["--backend=native", "--opt=bcc"][..],
        ] {
            let built = compile(&[&[target.as_str(), "-o", path][..], opt].concat());
            assert!(built.status.success(), "{opt:?}: {}", text(&built.stderr));
            let output = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(output.status.code(), Some(0), "{opt:?}");
            assert_eq!(text(&output.stdout), TRANSCRIPT, "{opt:?}");
        }
        std::fs::remove_file(&executable).unwrap();
    }

    #[test]
    fn inline_modes_emit_bodies_and_run_in_both_back_ends() {
        let Some(target) = host() else {
            return;
        };
        let target = format!("--target={target}");
        for (flags, definition) in [
            (
                vec!["-std=gnu17", "-fgnu89-inline"],
                "inline int f(int x) { return x + 1; }",
            ),
            (vec!["-std=gnu89"], "inline int f(int x) { return x + 1; }"),
            (
                vec!["-std=c89"],
                "__inline__ int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c90"],
                "__inline__ int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=gnu17", "-fgnu89-inline", "-fno-gnu89-inline"],
                "extern inline int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=gnu89", "-fno-gnu89-inline"],
                "extern inline int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c99"],
                "extern inline int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c99"],
                "inline int f(int x) { return x + 1; } extern int f(int);",
            ),
            (
                vec!["-std=c99"],
                "inline __attribute__((gnu_inline)) int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c99"],
                "extern inline __attribute__((gnu_inline)) int f(int x) { return 99; } int f(int \
                 x) { return x + 1; }",
            ),
            (
                vec!["-std=c99"],
                "static inline int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c99", "-fgnu89-inline"],
                "static inline int f(int x) { return x + 1; }",
            ),
        ] {
            // Taking the address also verifies that a real callable body
            // exists.
            let source =
                format!("{definition} int main(void) {{ int (*p)(int) = f; return p(6) != 7; }}");
            let executable = temporary("inline.exe");
            let path = executable.to_str().unwrap();
            for opt in ["--opt=none", "--opt=bcc"] {
                let mut common = flags.clone();
                common.extend([&target, opt]);
                let mut interpret = common.clone();
                interpret.push("--interpret");
                let output = bcc(&source, &interpret);
                assert!(
                    output.status.success(),
                    "{flags:?}: {}",
                    text(&output.stderr)
                );
                common.extend(["--llvm-opt=O0", "-o", path]);
                let output = bcc(&source, &common);
                assert!(
                    output.status.success(),
                    "{flags:?}: {}",
                    text(&output.stderr)
                );
                let output = run_native(&mut Command::new(&executable)).unwrap();
                assert!(output.status.success(), "{flags:?}: {output:?}");
            }
            std::fs::remove_file(executable).unwrap();
        }
    }

    #[test]
    fn inline_only_calls_stay_external_and_w_suppresses_only_warnings() {
        for (flags, definition) in [
            (vec!["-std=c99"], "inline int f(void) { return 99; }"),
            (
                vec!["-std=c99", "-fgnu89-inline"],
                "extern inline int f(void) { return 99; }",
            ),
            (
                vec!["-std=c99"],
                "extern inline __attribute__((gnu_inline)) int f(void) { return 99; }",
            ),
        ] {
            let source = format!("{definition} int main(void) {{ return f(); }}");
            for opt in ["--opt=none", "--opt=bcc"] {
                let mut args = flags.clone();
                args.extend([opt, "--emit=llvm"]);
                let output = bcc(&source, &args);
                assert!(output.status.success(), "{}", text(&output.stderr));
                assert!(
                    text(&output.stdout).contains("declare i32 @f("),
                    "{}",
                    text(&output.stdout)
                );
                _ = args.pop();
                args.push("--interpret");
                let output = bcc(&source, &args);
                assert_eq!(output.status.code(), Some(70), "{}", text(&output.stderr));
            }
        }
        let source = "int main(void) { exit(0); }";
        assert!(text(&bcc(source, &["--interpret"]).stderr).contains("warning:"));
        let output = bcc(source, &["--interpret", "-w"]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
        let output = bcc("int main(void) { return missing; }", &["--interpret", "-w"]);
        assert!(!output.status.success());
        assert!(text(&output.stderr).contains("error:"));
    }

    #[test]
    fn fold_finishes_many_noundef_parameters_and_volatile_bit_field_writes() {
        use std::fmt::Write as _;

        let mut source = String::from("struct B { unsigned b:1; }; void f(");
        for index in 0..1023 {
            write!(source, "int p{index}, ").unwrap();
        }
        source.push_str("volatile struct B *q) {\n");
        for _ in 0..1024 {
            source.push_str("q->b = p0;\n");
        }
        source.push_str("}\n");
        // The old nested parameter/attribute scans performed ~537M comparisons.
        // Allow ample process and front-end time while bounding that
        // regression.
        let output = run_native_with_timeout(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--emit=ir", "--passes=fold", "--input", &source])
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE"),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
        let ir = text(&output.stdout);
        assert_eq!(ir.matches(" noundef").count(), 1024);
        assert_eq!(ir.matches("freeze.").count(), 1024);
    }

    #[test]
    fn noundef_scalar_argument_reports_ub_with_status_70() {
        let source =
            "static int ignore(int x) { return 0; } int main(void) { int x; return ignore(x); }";
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
        ] {
            let output = bcc(source, &["--interpret", "--opt=none", target]);
            assert_eq!(output.status.code(), Some(70), "{}", text(&output.stderr));
            assert!(text(&output.stderr).contains("poison reached a noundef"));
        }
    }

    #[test]
    fn nested_overlapping_unions_finish_and_match_the_simple_union_abi() {
        let source = include_str!("../fixtures/lowering/nested_overlapping_unions.c");
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-pc-windows-msvc",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                for mode in ["--emit=ir", "--emit=ir-abi", "--emit=llvm", "--interpret"] {
                    let output = run_native_with_timeout(
                        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                            .args([mode, target, opt, "--input", source])
                            .env("NO_COLOR", "1")
                            .env_remove("CLICOLOR_FORCE"),
                        std::time::Duration::from_secs(10),
                    )
                    .unwrap();
                    assert!(
                        output.status.success(),
                        "{mode} {target} {opt}: {}",
                        text(&output.stderr)
                    );
                    assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
                    let ir = text(&output.stdout);
                    match mode {
                        | "--emit=ir" =>
                            for name in ["f", "simple"] {
                                assert!(
                                    ir.contains(&format!(
                                        "function @{name}(ptr aggregate(size 4, align 4, [0: \
                                         i32]) noundef) -> i32"
                                    )),
                                    "{ir}"
                                );
                            },
                        | "--emit=ir-abi" =>
                            for name in ["f", "simple"] {
                                assert!(
                                    ir.contains(&format!("function @{name}(i32) -> i32")),
                                    "{ir}"
                                );
                            },
                        | "--emit=llvm" =>
                            for name in ["f", "simple"] {
                                assert!(ir.contains(&format!("define i32 @{name}(i32 %")), "{ir}");
                            },
                        | "--interpret" => assert!(ir.is_empty(), "{ir}"),
                        | _ => unreachable!("all tested modes are covered"),
                    }
                }
            }
        }
    }

    #[test]
    fn unresolved_external_objects_trap_on_access() {
        for source in [
            "extern int shared; int main(void){ return shared != 7; }",
            "extern int shared; int main(void){ shared = 7; return 0; }",
            "extern int shared; int main(void){ int *p = &shared; return *p; }",
            "extern int shared; int *p = &shared; int main(void){ return *p; }",
            "extern int shared[2]; int *p = shared; int main(void){ return p[1]; }",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let output = bcc(&format!("{source}\n"), &["--interpret", opt]);
                assert_eq!(
                    output.status.code(),
                    Some(70),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert!(
                    text(&output.stderr).contains("unresolved external object @shared"),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert_eq!(output.stdout.as_slice(), b"");
            }
        }
    }

    #[test]
    fn tentative_object_definitions_remain_zero_initialized() {
        for source in [
            "int shared; int main(void){ return shared; }",
            "extern int shared; int shared; int main(void){ return shared; }",
            "int shared; int *p = &shared; int main(void){ return *p; }",
            "extern int shared; int shared = 7; int main(void){ return shared != 7; }",
            "extern int shared; int main(void){ return 0; }",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let output = bcc(&format!("{source}\n"), &["--interpret", opt]);
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert_eq!(output.stderr.as_slice(), b"");
            }
        }
    }

    #[test]
    fn incompatible_host_signatures_trap_before_execution() {
        for source in [
            "double malloc(unsigned long); int main(void){return (int)malloc(1);}",
            "double putchar(int); int main(void){return (int)putchar(65);}",
            "double abs(int); int main(void){return (int)abs(-1);}",
            "double printf(const char *, ...); int main(void){return (int)printf(\"x\");}",
            "int fabs(double); int main(void){return fabs(-1.);}",
            "int free(void *); int main(void){return free(0);}",
            "int exit(int); int main(void){return exit(0);}",
            "int abort(void); int main(void){return abort();}",
            "void putchar(int); int main(void){putchar(65); return 0;}",
            "int putchar(double); int main(void){return putchar(65.);}",
            "void *memcpy(void *, const void *, double); int main(void){char a, b; memcpy(&a, &b, \
             1.); return 0;}",
            "int puts(void); int main(void){return puts();}",
            "int abs(int, int); int main(void){return abs(1, 2);}",
            "double putchar(int); int main(void){double (*p)(int)=putchar; return (int)p(65);}",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let output = bcc(&format!("{source}\n"), &["--interpret", opt]);
                assert_eq!(
                    output.status.code(),
                    Some(70),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert!(
                    text(&output.stderr).contains("wrong signature"),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert!(
                    output.stdout.is_empty(),
                    "{source}: {}",
                    text(&output.stdout)
                );
            }
        }
    }

    #[test]
    fn poison_return_from_main_traps_like_exit() {
        for ending in ["return x;", "exit(x);"] {
            let source = format!("void exit(int); int main(void) {{ int x; {ending} }}\n");
            for flags in [["--interpret", "--opt=none"], ["--interpret", "--opt=bcc"]] {
                let output = bcc(&source, &flags);
                assert_eq!(
                    output.status.code(),
                    Some(70),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert!(text(&output.stderr).contains("undefined behaviour"));
                assert!(text(&output.stderr).contains("poison"));
            }
        }
    }

    const HELLO: &str = "int printf(const char *, ...);\nint main(void) {\n  printf(\"%s %d\\n\", \
                         \"answer\", 42);\n  return 3;\n}\n";

    /// Passes a structure by value both ways, which ABI lowering turns into
    /// two registers on System V and an `sret` pointer and a copy on Win64.
    const PAIR: &str = "struct pair { long long a, b; };\nstruct pair swap(struct pair p) {\n  \
                        struct pair r = { p.b, p.a };\n  return r;\n}\nint main(void) {\n  struct \
                        pair p = { 3, 40 };\n  return swap(p).a + 2;\n}\n";

    /// Runs the compiler on the inline `source` with `flags`.
    fn bcc(source: &str, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--input", source])
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .output()
            .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    #[test]
    fn closed_stdout_exits_quietly() {
        use std::{
            fmt::Write as _,
            process::Stdio,
        };
        let mut source = String::from(
            "int putchar(int); int main(void) { for (int i = 0; i < 20000; ++i) putchar(65); \
             return 3; }\n",
        );
        for i in 0..300 {
            writeln!(source, "int f{i}(int x) {{ return x + {i}; }}").unwrap();
        }
        for flag in [
            "--emit=ir",
            "--emit=ir-abi",
            "--emit=llvm",
            "--print-default-passes",
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command
                .arg(flag)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if flag != "--print-default-passes" {
                _ = command.args(["--input", &source]);
            }
            let mut child = command.spawn().unwrap();
            // Close the read end before compilation finishes; enough output is
            // produced to reach the OS pipe even with stdout buffering.
            drop(child.stdout.take());
            let output = child.wait_with_output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{flag}: {}",
                text(&output.stderr)
            );
            assert!(output.stderr.is_empty(), "{flag}: {}", text(&output.stderr));
        }
    }

    #[test]
    fn closed_stdout_preserves_interpreter_status_and_traps() {
        use std::process::Stdio;
        for (ending, status, diagnostic) in [
            ("return 42;", 42, ""),
            ("exit(23);", 23, ""),
            ("abort();", 134, ""),
            (
                "return 1 / zero;",
                70,
                "error: the interpreter stopped the program:",
            ),
        ] {
            let source = format!(
                "int putchar(int); void exit(int); void abort(void); int main(void) {{ int zero = \
                 0; for (int i = 0; i < 20000; ++i) putchar(65); {ending} }}\n"
            );
            let mut child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--interpret", "--input", &source])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            drop(child.stdout.take());
            let output = child.wait_with_output().unwrap();
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(status), "{ending}: {stderr}");
            if diagnostic.is_empty() {
                assert!(stderr.is_empty(), "{ending}: {stderr}");
            } else {
                assert!(stderr.contains(diagnostic), "{ending}: {stderr}");
            }
        }
    }

    /// Opens a stdio handle whose writes fail. Rust's standard library
    /// treats EBADF as closed stdio and drops the output silently, so on
    /// Unix a read-only descriptor does not fail; `/dev/full` reports ENOSPC.
    fn failing_stdio(path: &Path) -> std::fs::File {
        if cfg!(unix) {
            std::fs::File::options()
                .write(true)
                .open("/dev/full")
                .unwrap()
        } else {
            std::fs::File::open(path).unwrap()
        }
    }

    #[test]
    fn read_only_stdout_reports_an_error_without_panicking() {
        let path = temporary("read-only-stdout.txt");
        std::fs::write(&path, b"unchanged").unwrap();
        for flag in [
            "--emit=ir",
            "--emit=ir-abi",
            "--emit=llvm",
            "--interpret",
            "--print-default-passes",
        ] {
            let file = failing_stdio(&path);
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.arg(flag).stdout(file);
            if flag != "--print-default-passes" {
                _ = command.args(["--input", HELLO]);
            }
            let output = command.output().unwrap();
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{flag}: {stderr}");
            assert!(
                stderr.starts_with("error: failed writing to stdout: "),
                "{flag}: {stderr}"
            );
            assert!(!stderr.contains("panicked"), "{flag}: {stderr}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"unchanged");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_only_stderr_preserves_exit_status() {
        let path = temporary("read-only-stderr.txt");
        std::fs::write(&path, b"unchanged").unwrap();
        let missing_output = temporary("missing-parent").join("output.o");
        let missing_input = temporary("missing-input.c");
        for (source, flags, status, fail_stdout) in [
            (
                "int main(void){int z=0;return 1/z;}",
                vec!["--interpret"],
                70,
                false,
            ),
            (
                "int main(void){return unknown;}",
                vec!["--interpret"],
                1,
                false,
            ),
            (
                "int main(void){__asm__(\"\" :: \"t\"(1.0L));return 0;}",
                vec!["--emit=ir"],
                1,
                false,
            ),
            (HELLO, vec!["--interpret"], 1, true),
            (SUM, vec!["--emit=llvm"], 1, true),
            (
                SUM,
                vec!["-c", "-o", missing_output.to_str().unwrap()],
                1,
                false,
            ),
            (
                "int main(void){return 42;}",
                vec!["--interpret", "--print-after-all"],
                42,
                false,
            ),
            ("int main(void){return 0;}", vec!["--interpret"], 0, false),
            (
                "void abort(void);int main(void){abort();}",
                vec!["--interpret"],
                134,
                false,
            ),
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command
                .args(&flags)
                .args(["--input", source])
                .stderr(failing_stdio(&path));
            if fail_stdout {
                _ = command.stdout(failing_stdio(&path));
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(status), "{flags:?}: {source}");
        }
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .arg("--interpret")
            .arg(&missing_input)
            .stderr(failing_stdio(&path))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "input file error");
        assert_eq!(std::fs::read(&path).unwrap(), b"unchanged");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn interpreter_rejects_excessive_printf_widths_and_precisions() {
        for format in [
            "%18446744073709551616d",
            "%999999999999999999999999999999d",
            "%1048577d",
            "%.18446744073709551616d",
            "%.1048577d",
        ] {
            let source = format!(
                "int printf(const char *, ...); int main(void) {{ printf(\"{format}\", 1); return \
                 0; }}\n"
            );
            let output = bcc(&source, &["--interpret"]);
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(70), "{format}: {stderr}");
            assert!(
                stderr.starts_with("error: the interpreter stopped the program:"),
                "{format}: {stderr}"
            );
            assert!(
                output.stdout.is_empty(),
                "{format}: output was padded before rejecting the format"
            );
            assert!(!stderr.contains("panicked"), "{format}: {stderr}");
        }
    }

    #[test]
    fn print_default_passes_lists_a_pipeline_that_passes_accepts() {
        let run = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .arg("--print-default-passes")
            .output()
            .unwrap();
        assert!(run.status.success(), "{}", text(&run.stderr));
        let list = text(&run.stdout);
        let list = list.trim_end();
        assert!(!list.is_empty() && !list.contains('\n'), "{list:?}");
        // The list is in `--passes` syntax, so the optimizer accepts it.
        let passes = format!("--passes={list}");
        let optimized = bcc(SUM, &["--emit=ir", &passes]);
        assert!(optimized.status.success(), "{}", text(&optimized.stderr));
    }

    #[test]
    fn emit_ir_prints_the_module_and_the_optimizer_rewrites_it() {
        let lowered = bcc(SUM, &["--emit=ir"]);
        assert!(lowered.status.success(), "{}", text(&lowered.stderr));
        let lowered = text(&lowered.stdout);
        assert!(
            lowered.starts_with("target triple = \"x86_64-unknown-linux-gnu\"\n"),
            "{lowered}"
        );
        assert!(
            lowered.contains("function @main() -> i32 external {"),
            "{lowered}"
        );
        // The loop's increment has its own block until simplify-cfg.
        assert!(lowered.contains("block4:"), "{lowered}");

        for flags in [
            &["--emit=ir", "--opt=bcc"][..],
            &["--emit=ir", "--passes=simplify-cfg"][..],
        ] {
            let optimized = bcc(SUM, flags);
            assert!(optimized.status.success(), "{}", text(&optimized.stderr));
            let optimized = text(&optimized.stdout);
            assert!(!optimized.contains("block4:"), "{flags:?}: {optimized}");
        }
    }

    #[test]
    fn print_after_all_and_the_bisect_limit_control_the_optimizer() {
        let printed = bcc(
            SUM,
            &["--emit=ir", "--passes=fold,dce", "--print-after-all"],
        );
        assert!(printed.status.success());
        let stderr = text(&printed.stderr);
        assert!(
            stderr.contains("*** IR after fold of @main ***"),
            "{stderr}"
        );
        assert!(stderr.contains("*** IR after dce of @main ***"), "{stderr}");

        // No rewrite is allowed, so the module stays as lowered.
        let bisected = bcc(SUM, &["--emit=ir", "--opt=bcc", "--opt-bisect-limit=0"]);
        assert!(bisected.status.success());
        assert_eq!(
            text(&bisected.stdout),
            text(&bcc(SUM, &["--emit=ir"]).stdout)
        );

        // The per-pass report goes to stderr and leaves the module alone.
        let timed = bcc(SUM, &["--emit=ir", "--time-passes"]);
        assert!(timed.status.success());
        let report = text(&timed.stderr);
        assert!(
            report.contains(" rewrites") && report.contains("  sroa "),
            "{report}"
        );
        assert_eq!(
            text(&timed.stdout),
            text(&bcc(SUM, &["--emit=ir", "--opt=bcc"]).stdout)
        );

        let unknown = bcc(SUM, &["--emit=ir", "--passes=fold,unknown"]);
        assert_eq!(unknown.status.code(), Some(2));
        assert!(text(&unknown.stderr).contains("unknown pass `unknown`"));
    }

    #[test]
    fn emit_llvm_prints_llvm_ir_for_the_target() {
        let output = bcc(HELLO, &["--emit=llvm", "--target=x86_64-w64-windows-gnu"]);
        assert!(output.status.success(), "{}", text(&output.stderr));
        let stdout = text(&output.stdout);
        assert!(
            stdout.contains("target triple = \"x86_64-w64-windows-gnu\""),
            "{stdout}"
        );
        assert!(stdout.contains("define i32 @main()"), "{stdout}");
        assert!(
            stdout.contains("declare i32 @printf(ptr noundef, ...)"),
            "{stdout}"
        );
    }

    #[test]
    fn back_ends_receive_the_module_after_abi_lowering() {
        // `--emit=ir` prints the module before ABI lowering ...
        let pre = bcc(PAIR, &["--emit=ir", "--opt=bcc"]);
        assert!(pre.status.success(), "{}", text(&pre.stderr));
        let pre = text(&pre.stdout);
        assert!(!pre.contains("abi lowered"), "{pre}");
        assert!(
            pre.contains(
                "function @swap(ptr result(size 16, align 8, [0: i64, 8: i64]), ptr \
                 aggregate(size 16, align 8, [0: i64, 8: i64]) noundef) external"
            ),
            "{pre}"
        );
        // ... and `--emit=ir-abi` after it, for the target's convention.
        let post = bcc(PAIR, &["--emit=ir-abi", "--opt=bcc"]);
        assert!(post.status.success(), "{}", text(&post.stderr));
        let post = text(&post.stdout);
        assert!(post.contains("abi lowered\n"), "{post}");
        assert!(
            post.contains("function @swap(ptr registers(i64, i64, align 8), i64, i64) external"),
            "{post}"
        );
        let win64 = bcc(PAIR, &["--emit=ir-abi", "--target=x86_64-w64-windows-gnu"]);
        let win64 = text(&win64.stdout);
        assert!(
            win64.contains("function @swap(ptr sret(size 16, align 8), ptr noundef) external"),
            "{win64}"
        );

        // The LLVM back end prints the target's convention ...
        let llvm = bcc(PAIR, &["--emit=llvm"]);
        assert!(llvm.status.success(), "{}", text(&llvm.stderr));
        let llvm = text(&llvm.stdout);
        assert!(llvm.contains("define { i64, i64 } @swap(i64 %"), "{llvm}");
        let llvm = text(&bcc(PAIR, &["--emit=llvm", "--target=x86_64-w64-windows-gnu"]).stdout);
        assert!(llvm.contains("sret([16 x i8]) align 8"), "{llvm}");

        // ... and the interpreter runs the post-ABI module for each target.
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let run = bcc(PAIR, &["--interpret", target, opt]);
                assert_eq!(
                    run.status.code(),
                    Some(42),
                    "{target} {opt}: {}",
                    text(&run.stderr)
                );
            }
        }
    }

    #[test]
    fn interpret_exits_with_the_status_of_main_and_forwards_output() {
        let output = bcc(HELLO, &["--interpret"]);
        assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "answer 42\n");

        let output = bcc(SUM, &["--interpret", "--opt=bcc"]);
        assert_eq!(output.status.code(), Some(10));

        let exited = bcc(
            "void exit(int);\nint main(void) {\n  exit(5);\n  return 1;\n}\n",
            &["--interpret"],
        );
        assert_eq!(exited.status.code(), Some(5));
    }

    #[test]
    fn interpret_supplies_modifiable_main_arguments() {
        let source = "int main(int argc, char **argv) {\nif (argc != 1 || argv == 0 || argv[0] == \
                      0 || argv[1] != 0) return 1;\nargv[0][0] = 'x'; argv[0] = 0; return 0;\n}\n";
        for target in [
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-unknown-linux-gnu",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let output = bcc(source, &["--interpret", target, opt]);
                assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
            }
        }
    }

    #[test]
    fn interpreter_traps_report_an_error_and_a_distinct_status() {
        let output = bcc(
            "int main(void) {\n  int a[2];\n  return a[5];\n}\n",
            &["--interpret"],
        );
        assert_eq!(output.status.code(), Some(70));
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains("error: the interpreter stopped the program: undefined behaviour"),
            "{stderr}"
        );
    }

    #[test]
    fn unsupported_constructs_and_errors_fail_code_generation_only() {
        let source = "int main(void) {\n  __asm__(\"\" :: \"t\"(1.0L));\n  return 0;\n}\n";
        for flags in [
            &["--interpret"][..],
            &["--emit=ir"][..],
            &["--emit=llvm"][..],
        ] {
            let output = bcc(source, flags);
            assert_eq!(output.status.code(), Some(1), "{flags:?}");
            assert!(output.stdout.is_empty(), "{flags:?}");
            let stderr = text(&output.stderr);
            assert!(
                stderr.contains(
                    "error: not yet supported by lowering: inline assembly operands of this \
                     register class or type"
                ),
                "{stderr}"
            );
            assert!(stderr.contains("1 error generated."), "{stderr}");
        }
        // Without a back end, the program is valid C and nothing is lowered.
        let output = bcc(source, &[]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));

        let output = bcc("int main(void) {\n  return x;\n}\n", &["--interpret"]);
        assert_eq!(output.status.code(), Some(1));
        let stderr = text(&output.stderr);
        assert!(stderr.contains("no visible declaration"), "{stderr}");
        assert!(!stderr.contains("lowering"), "{stderr}");
    }

    #[test]
    fn back_end_options_need_a_back_end() {
        for flags in [
            &["--emit=ir", "--interpret"][..],
            &["--emit=ir", "--syntax-tree"][..],
            &["-c", "--emit=ir"][..],
        ] {
            let output = bcc(SUM, flags);
            assert_eq!(output.status.code(), Some(2), "{flags:?}");
        }
        // `-c` names its object after the input file, which `--input` lacks.
        let output = bcc(SUM, &["-c"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            text(&output.stderr).contains("give -o for --input"),
            "{}",
            text(&output.stderr)
        );
    }

    /// The target the tests run on, if the LLVM back end supports it.
    fn host() -> Option<&'static str> {
        if cfg!(all(windows, target_env = "gnu")) {
            Some("x86_64-w64-windows-gnu")
        } else if cfg!(all(windows, target_env = "msvc")) {
            Some("x86_64-pc-windows-msvc")
        } else if cfg!(all(target_os = "linux", target_env = "gnu")) {
            Some("x86_64-unknown-linux-gnu")
        } else if cfg!(all(target_os = "linux", target_env = "musl")) {
            Some("x86_64-unknown-linux-musl")
        } else {
            None
        }
    }

    /// A fresh path in the system temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        env::temp_dir().join(format!(
            "bcc-cli-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn object_output_cannot_overwrite_input_source() {
        assert_output_preserves_input(true, false);
    }

    #[test]
    fn executable_output_cannot_overwrite_input_source() {
        assert_output_preserves_input(false, false);
    }

    #[test]
    fn object_output_cannot_overwrite_input_through_a_hard_link() {
        assert_output_preserves_input(true, true);
    }

    #[test]
    fn executable_output_cannot_overwrite_input_through_a_hard_link() {
        assert_output_preserves_input(false, true);
    }

    /// The same-file check must not open the output to read its identity:
    /// opening a FIFO for reading blocks until a writer appears.
    #[cfg(unix)]
    #[test]
    fn object_output_to_a_fifo_does_not_block() {
        let source = temporary("fifo.c");
        let fifo = temporary("fifo.o");
        std::fs::write(&source, SUM).unwrap();
        let made = Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(made.success(), "mkfifo failed");
        let mut reader = Command::new("cat")
            .arg(&fifo)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let result = run_native_with_timeout(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .arg(&source)
                .args(["-c", "-o"])
                .arg(&fifo),
            std::time::Duration::from_secs(60),
        );
        // A failed compile may never open the FIFO, leaving the reader
        // blocked in its own open; only a successful write ends it.
        if !result.as_ref().is_ok_and(|output| output.status.success()) {
            drop(reader.kill());
        }
        let reader_status = reader.wait().unwrap();
        std::fs::remove_file(&source).unwrap();
        std::fs::remove_file(&fifo).unwrap();
        let output = result.unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert!(reader_status.success());
    }

    fn assert_output_preserves_input(object: bool, hard_link: bool) {
        let source = temporary("preserve.c");
        let alias = temporary("alias.c");
        std::fs::write(&source, SUM).unwrap();
        let output = if hard_link {
            std::fs::hard_link(&source, &alias).unwrap();
            &alias
        } else {
            &source
        };
        let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
        _ = command.arg(&source).arg("-o").arg(output);
        if object {
            _ = command.arg("-c");
        }
        let result = command.output().unwrap();
        let contents = std::fs::read(&source).unwrap();
        std::fs::remove_file(&source).unwrap();
        if hard_link {
            std::fs::remove_file(&alias).unwrap();
        }
        assert!(contents == SUM.as_bytes(), "the C source was overwritten");
        assert_eq!(result.status.code(), Some(1), "{}", text(&result.stderr));
        assert_eq!(
            text(&result.stderr).trim(),
            format!(
                "error: output file '{}' is the same as input file '{}'",
                output.display(),
                source.display()
            )
        );
        assert_eq!(result.stdout.as_slice(), b"");
    }

    #[test]
    fn output_builds_executables_and_objects_through_clang() {
        let Some(target) = host() else {
            eprintln!("skipping: the host is not a supported target");
            return;
        };
        let clang = Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        });
        if !clang.is_file() {
            eprintln!("skipping: no bundled clang at {}", clang.display());
            return;
        }
        let target = format!("--target={target}");
        let executable = temporary("hello.exe");
        let object = temporary("hello.o");
        let path = |path: &Path| path.to_str().unwrap().to_owned();
        let built = bcc(
            HELLO,
            &[
                &target,
                "--opt=bcc",
                "--llvm-opt=O2",
                "--llvm-codegen-only",
                "-o",
                &path(&executable),
            ],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let run = run_native(&mut Command::new(&executable)).unwrap();
        assert_eq!(run.status.code(), Some(3));
        assert_eq!(text(&run.stdout), "answer 42\n");

        let compiled = bcc(HELLO, &[&target, "-c", "-o", &path(&object)]);
        assert!(compiled.status.success(), "{}", text(&compiled.stderr));
        assert!(object.is_file());

        // A structure passed and returned by value goes through ABI lowering.
        let pair = temporary("pair.exe");
        for flags in [&["--llvm-opt=O0"][..], &["--opt=bcc", "--llvm-opt=O2"][..]] {
            let built = bcc(
                PAIR,
                &[&[&target[..], "-o", &path(&pair)][..], flags].concat(),
            );
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&pair)).unwrap();
            assert_eq!(run.status.code(), Some(42), "{flags:?}");
        }
        drop(std::fs::remove_file(executable));
        drop(std::fs::remove_file(object));
        drop(std::fs::remove_file(pair));
    }

    #[test]
    fn native_back_end_builds_runs_and_refuses_clearly() {
        let Some(target) = host() else {
            eprintln!("skipping: the host is not a supported target");
            return;
        };
        let target = format!("--target={target}");
        let path = |path: &Path| path.to_str().unwrap().to_owned();
        let executable = temporary("native-hello.exe");
        // The default is direct objects on Windows and the assembler on
        // Linux; each flag selects one path on every host.
        for flags in [
            &[][..],
            &["--opt=bcc"][..],
            &["--native-object"][..],
            &["--native-object", "--opt=bcc"][..],
            &["--no-native-object"][..],
            &["--no-native-object", "--opt=bcc"][..],
        ] {
            let built = bcc(
                HELLO,
                &[
                    &[&target[..], "--backend=native", "-o", &path(&executable)][..],
                    flags,
                ]
                .concat(),
            );
            if built.status.code() == Some(1) && text(&built.stderr).contains("cannot find") {
                eprintln!("skipping: {}", text(&built.stderr));
                return;
            }
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(3), "{flags:?}");
            assert_eq!(text(&run.stdout), "answer 42\n", "{flags:?}");
        }
        let pair = temporary("native-pair.exe");
        let built = bcc(PAIR, &[&target, "--backend=native", "-o", &path(&pair)]);
        assert!(built.status.success(), "{}", text(&built.stderr));
        let run = run_native(&mut Command::new(&pair)).unwrap();
        assert_eq!(run.status.code(), Some(42));
        // On Windows the default assembles a listing the encoder refuses, here
        // for its section name, and links that object as it links its own.
        let refused = temporary("native-refused.exe");
        let built = bcc(
            "__attribute__((section(\"my data\"))) int x = 42;\nint main(void) { return x; }\n",
            &[&target, "--backend=native", "-o", &path(&refused)],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let run = run_native(&mut Command::new(&refused)).unwrap();
        assert_eq!(run.status.code(), Some(42));
        for flags in [
            &[][..],
            &["--native-object"][..],
            &["--no-native-object"][..],
        ] {
            let object = temporary("native-hello.o");
            let linked = temporary("native-object-hello.exe");
            assert!(!object.exists());
            let compiled = bcc(
                HELLO,
                &[
                    &[&target[..], "--backend=native", "-c", "-o", &path(&object)][..],
                    flags,
                ]
                .concat(),
            );
            assert!(compiled.status.success(), "{}", text(&compiled.stderr));
            let bytes = std::fs::read(&object).unwrap();
            if target.contains("linux") {
                assert!(bytes.starts_with(b"\x7fELF\x02\x01\x01"));
                assert!(bytes.windows(6).any(|s| s == b".text\0"));
                assert!(bytes.windows(5).any(|s| s == b"main\0"));
            } else {
                assert!(
                    bytes.starts_with(&[0x64, 0x86]),
                    "{flags:?}: not x86-64 COFF"
                );
                assert!(bytes.windows(5).any(|s| s == b".text"), "{flags:?}");
                assert!(bytes.windows(4).any(|s| s == b"main"), "{flags:?}");
            }
            let clang = Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
                "target/llvm/bin/clang.exe"
            } else {
                "target/llvm/bin/clang"
            });
            let result = Command::new(clang)
                .arg(&target)
                .arg(&object)
                .arg("-o")
                .arg(&linked)
                .output()
                .unwrap();
            assert!(result.status.success(), "{}", text(&result.stderr));
            let run = run_native(&mut Command::new(&linked)).unwrap();
            assert_eq!(run.status.code(), Some(3));
            assert_eq!(text(&run.stdout), "answer 42\n");
            std::fs::remove_file(object).unwrap();
            std::fs::remove_file(linked).unwrap();
        }
        for file in [executable, pair, refused] {
            drop(std::fs::remove_file(file));
        }

        // `long double` goes through the x87 unit.
        let float = temporary("native-float.exe");
        let built = bcc(
            "long double half(long double x) { return x / 2; }\nint main(void) { return half(4) \
             != 2; }\n",
            &[&target, "--backend=native", "-o", &path(&float)],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let run = run_native(&mut Command::new(&float)).unwrap();
        assert_eq!(run.status.code(), Some(0));
        drop(std::fs::remove_file(&float));

        // Unsupported code is refused before anything is written.
        let refused = bcc(
            "int pick(__int128 x) { switch (x) { case 1: return 2; default: return 3; } }\nint \
             main(void) { return pick(1) != 2; }\n",
            &[&target, "--backend=native", "-o", &path(&float)],
        );
        assert_eq!(refused.status.code(), Some(1));
        assert_eq!(
            text(&refused.stderr).trim(),
            "error: not yet supported by the native back end: a switch on an i128 value (in @pick)"
        );
        assert!(!float.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn native_named_array_and_bss_sections_run() {
        let source = r#"int printf(const char *, ...);
static void hello(void) { printf("ctor\n"); }
static void goodbye(void) { printf("fini\n"); }
__attribute__((section(".init_array"), used)) static void (*p)(void) = hello;
__attribute__((section(".fini_array.101"), used)) static void (*q)(void) = goodbye;
__attribute__((section(".bss.foo"))) int zero;
int main(void) { printf("main\n"); return zero; }
"#;
        for flags in [
            &[][..],
            &["--opt=bcc"][..],
            &["--native-object"][..],
            &["--native-object", "--opt=bcc"][..],
        ] {
            let executable = temporary("native-named-sections.exe");
            let built = bcc(
                source,
                &[
                    &[
                        "--target=x86_64-unknown-linux-gnu",
                        "--backend=native",
                        "-o",
                        executable.to_str().unwrap(),
                    ][..],
                    flags,
                ]
                .concat(),
            );
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0));
            assert_eq!(text(&run.stdout), "ctor\nmain\nfini\n");
            std::fs::remove_file(executable).unwrap();
        }
    }

    #[test]
    fn native_assembly_and_option_conflicts() {
        let asm = bcc(
            SUM,
            &[
                "--target=x86_64-unknown-linux-gnu",
                "--backend=native",
                "--emit=asm",
            ],
        );
        assert!(asm.status.success(), "{}", text(&asm.stderr));
        let listing = text(&asm.stdout);
        assert!(listing.contains("\t.globl\tmain\n"), "{listing}");
        assert!(listing.contains(".cfi_startproc"), "{listing}");
        let asm = bcc(
            SUM,
            &[
                "--target=x86_64-w64-windows-gnu",
                "--backend=native",
                "--emit=asm",
            ],
        );
        assert!(text(&asm.stdout).contains(".seh_proc\tmain"));
        // Without --backend=native, --emit=asm prints LLVM's assembly.
        let asm = bcc(SUM, &["--target=x86_64-unknown-linux-gnu", "--emit=asm"]);
        assert!(asm.status.success(), "{}", text(&asm.stderr));
        assert!(text(&asm.stdout).contains("main:"), "{}", text(&asm.stdout));
        for (flags, message) in [
            (
                &["--native-object", "--emit=ir"][..],
                "error: --native-object requires --backend=native",
            ),
            (
                &["--no-native-object", "--emit=ir"][..],
                "error: --no-native-object requires --backend=native",
            ),
            (
                &["--no-native-object", "--native-object", "--emit=ir"][..],
                "error: --native-object requires --backend=native",
            ),
            (
                &["--backend=native", "--emit=llvm"][..],
                "error: --emit=llvm prints LLVM IR, which --backend=native does not use",
            ),
            (
                &["--backend=native", "--interpret"][..],
                "error: --backend=native needs -o, -c, -S or --emit=asm; --interpret runs the IR",
            ),
            (
                &["--backend=native", "--emit=asm", "--llvm-opt=O2"][..],
                "error: --llvm-opt and --llvm-codegen-only configure the LLVM back end, not \
                 --backend=native",
            ),
        ] {
            let output = bcc(SUM, flags);
            assert_eq!(output.status.code(), Some(1), "{flags:?}");
            assert_eq!(text(&output.stderr).trim(), message, "{flags:?}");
            assert!(output.stdout.is_empty(), "{flags:?}");
        }
    }

    #[test]
    fn llvm_diagnostic_errors_fail_cli_without_an_output_file() {
        // This static body is retained by a call, rather than `used`, so it
        // reaches LLVM's MC diagnostics instead of the early semantic check.
        let source = "static __attribute__((noinline,section(\".bss.z\"))) int f(int x) { return \
                      x+1; }\nstatic __attribute__((noinline,section(\".bss.second\"))) int h(int \
                      x) { return x+2; }\nint g(int x) { return f(x) + h(x); }\n";
        let object = temporary("llvm-diagnostic-refused.o");
        std::fs::write(&object, b"stale object").unwrap();
        let output = bcc(
            source,
            &[
                "--target=x86_64-unknown-linux-gnu",
                "--opt=none",
                "--llvm-opt=O0",
                "-c",
                "-o",
                object.to_str().unwrap(),
            ],
        );
        assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
        assert!(
            text(&output.stderr).contains("BSS section '.bss.z' cannot have non-zero bytes"),
            "{}",
            text(&output.stderr)
        );
        assert!(
            text(&output.stderr).contains("BSS section '.bss.second' cannot have non-zero bytes"),
            "{}",
            text(&output.stderr)
        );
        assert!(!object.exists(), "an LLVM error wrote an object");
        // Installing the diagnostic callback must preserve LLVM's filtering
        // of optimization remarks on ordinary successful compilations.
        let valid = bcc(
            "int value(void) { return 3; }",
            &[
                "--target=x86_64-unknown-linux-gnu",
                "--opt=none",
                "--llvm-opt=O0",
                "-c",
                "-o",
                object.to_str().unwrap(),
            ],
        );
        assert!(valid.status.success(), "{}", text(&valid.stderr));
        assert!(valid.stderr.is_empty(), "{}", text(&valid.stderr));
        assert!(object.exists());
        std::fs::remove_file(object).unwrap();
    }

    #[test]
    fn bss_initializer_errors_fail_all_backends_without_an_output_file() {
        let source = "int a __attribute__((section(\".bss.z\"))) = 4;\nint b \
                      __attribute__((section(\".gnu.linkonce.tb.z\"))) = 5;\n";
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            let object = temporary("bss-refused.o");
            std::fs::write(&object, b"stale object").unwrap();
            let output_path = object.to_str().unwrap();
            let mut flags = vec!["--target=x86_64-unknown-linux-gnu", "-c", "-o", output_path];
            flags.extend_from_slice(backend);
            let output = bcc(source, &flags);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{backend:?}: {}",
                text(&output.stderr)
            );
            assert!(
                !object.exists(),
                "a failed compilation wrote an object: {backend:?}"
            );
            let stderr = text(&output.stderr);
            assert!(
                stderr.contains("BSS section '.bss.z' cannot have non-zero bytes"),
                "{stderr}"
            );
            assert!(
                stderr.contains("BSS section '.gnu.linkonce.tb.z' cannot have non-zero bytes"),
                "{stderr}"
            );
            assert!(stderr.contains(":1:") && stderr.contains(":2:"), "{stderr}");
        }
    }

    #[test]
    fn bss_static_objects_are_checked_only_when_emitted() {
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            for optimize in [false, true] {
                for (source, emitted) in [
                    (
                        "static int b[4] __attribute__((section(\".bss.t\"))) = {1}; int \
                         f(void){return 0;}",
                        false,
                    ),
                    (
                        "static int b[4] __attribute__((section(\".bss.t\"))) = {1}; int \
                         *f(void){return b;}",
                        true,
                    ),
                    (
                        "static int b[4] __attribute__((used,section(\".bss.t\"))) = {1}; int \
                         f(void){return 0;}",
                        true,
                    ),
                    (
                        "struct P {int a,b;}; static struct P p[1000] \
                         __attribute__((section(\".bss.t\"))) = {[0 ... 999]={0,1}}; int \
                         f(void){return 0;}",
                        false,
                    ),
                    (
                        "int f(void){static int b[4] \
                         __attribute__((section(\".bss.t\")))={1};return b[0];}",
                        true,
                    ),
                    (
                        "static int b[4] __attribute__((section(\".bss.t\")))={1}; static int \
                         f(void){return b[0];} int g(void){return 0;}",
                        false,
                    ),
                    (
                        "static int b[4] __attribute__((section(\".bss.t\")))={1}; int *p=b;",
                        true,
                    ),
                ] {
                    let object = temporary("bss-static.o");
                    let mut flags = vec!["-c", "-o", object.to_str().unwrap()];
                    flags.extend_from_slice(backend);
                    if optimize {
                        flags.push(if backend.is_empty() {
                            "--llvm-opt=O2"
                        } else {
                            "--opt=bcc"
                        });
                    }
                    let output = bcc(source, &flags);
                    assert_eq!(
                        output.status.success(),
                        !emitted,
                        "{backend:?} optimize={optimize}: {}",
                        text(&output.stderr)
                    );
                    assert_eq!(object.exists(), !emitted);
                    if emitted {
                        assert!(
                            text(&output.stderr).contains("-->"),
                            "{}",
                            text(&output.stderr)
                        );
                        assert!(
                            text(&output.stderr)
                                .contains("BSS section '.bss.t' cannot have non-zero bytes"),
                            "{}",
                            text(&output.stderr)
                        );
                    } else {
                        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
                        std::fs::remove_file(object).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn failed_compilation_removes_stale_object_files() {
        let cases = [
            ("preprocessing", "#error deliberate failure\n"),
            ("parsing", "int value = ;\n"),
            ("semantics", "int value = not_declared;\n"),
            (
                "lowering",
                "int value(void) { __asm__(\"\" :: \"t\"(1.0L)); return 0; }\n",
            ),
            (
                "backend",
                "static __attribute__((noinline,section(\".bss.fn\"))) int f(int x) { return x+1; \
                 } int g(int x) { return f(x); }\n",
            ),
        ];
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            for (kind, source) in cases {
                let object = temporary("stale-object.o");
                std::fs::write(&object, b"stale object").unwrap();
                let mut flags = vec![
                    "--target=x86_64-unknown-linux-gnu",
                    "-c",
                    "-o",
                    object.to_str().unwrap(),
                ];
                flags.extend_from_slice(backend);
                let output = bcc(source, &flags);
                assert_eq!(
                    output.status.code(),
                    Some(1),
                    "{kind} {backend:?}: {}",
                    text(&output.stderr)
                );
                assert!(!object.exists(), "{kind} {backend:?} left stale output");
            }
        }
    }

    #[test]
    fn driver_conflicts_preserve_stale_object_files() {
        for conflict in [
            &["--native-object"][..],
            &["--no-native-object"][..],
            &["--backend=native", "--llvm-opt=O2"][..],
            &["--backend=native", "--llvm-codegen-only"][..],
        ] {
            let object = temporary("driver-conflict.o");
            std::fs::write(&object, b"existing object").unwrap();
            let mut flags = vec!["-c", "-o", object.to_str().unwrap()];
            flags.extend_from_slice(conflict);
            // A usage error must precede even invalid source diagnostics.
            let output = bcc("int value = not_declared;", &flags);
            assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
            assert!(text(&output.stderr).contains("--backend=native"));
            assert!(!text(&output.stderr).contains("not_declared"));
            assert_eq!(std::fs::read(&object).unwrap(), b"existing object");
            std::fs::remove_file(object).unwrap();
        }
    }

    #[test]
    #[cfg(unix)]
    fn failed_compilation_preserves_read_only_object_files() {
        use std::os::unix::fs::PermissionsExt as _;

        // Root's access(W_OK) succeeds even for mode 0444, as it does in LLVM.
        // SAFETY: getuid has no preconditions.
        if unsafe { libc::getuid() } == 0 {
            return;
        }
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            for source in ["int value;", "int value = not_declared;"] {
                let object = temporary("read-only-object.o");
                std::fs::write(&object, b"existing object").unwrap();
                std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o444)).unwrap();
                let mut flags = vec!["-c", "-o", object.to_str().unwrap()];
                flags.extend_from_slice(backend);
                let output = bcc(source, &flags);
                assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
                assert_eq!(std::fs::read(&object).unwrap(), b"existing object");
                std::fs::remove_file(object).unwrap();
            }
        }
    }

    #[test]
    fn front_end_failure_preserves_stale_executable_like_clang() {
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            let executable = temporary("stale-executable");
            std::fs::write(&executable, b"existing executable").unwrap();
            let mut flags = vec![
                "--target=x86_64-unknown-linux-gnu",
                "-o",
                executable.to_str().unwrap(),
            ];
            flags.extend_from_slice(backend);
            let output = bcc("int value = not_declared;", &flags);
            assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
            assert_eq!(std::fs::read(&executable).unwrap(), b"existing executable");
            std::fs::remove_file(executable).unwrap();
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn folded_pointer_cases_retain_switch_dispatch_on_all_backends() {
        let source = "struct S {int a,b;};
            int f1(int x){switch(x){case (long)((char *)0 + 1):return 11;default:return 0;}}
            int f2(int x){switch(x){case (long)&((struct S *)0)->b:return 12;default:return 0;}}
            int f3(int x){switch(x){case (int)(long)(char *)4:return 13;default:return 0;}}
            int f4(int x){switch(x){case (unsigned char)((char *)8 - (char *)0):return \
                      14;default:return 0;}}
            int f5(int x){switch(x){case (unsigned char)(long)(char *)260:return 15;default:return \
                      0;}}
            int f6(int x){switch(x){case (long)(_Bool)(char *)4:return 16;default:return 0;}}
            int main(void){return !(f5(4)==15 && f5(5)==0 && f6(1)==16 && f6(0)==0 && f1(1)==11 && \
                      f1(2)==0 && f2(4)==12 && f2(5)==0
                && f3(4)==13 && f3(5)==0 && f4(8)==14 && f4(9)==0);}";
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            for mode in ["-std=gnu17", "-std=c99"] {
                let executable = temporary("folded-pointer-cases");
                let mut flags = vec![mode, "-o", executable.to_str().unwrap()];
                flags.extend_from_slice(backend);
                let output = bcc(source, &flags);
                assert!(
                    output.status.success(),
                    "{backend:?}: {}",
                    text(&output.stderr)
                );
                assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
                assert_eq!(
                    run_native(&mut Command::new(&executable))
                        .unwrap()
                        .status
                        .code(),
                    Some(0)
                );
                std::fs::remove_file(executable).unwrap();
                let object = temporary("folded-pointer-duplicate.o");
                let mut flags = vec![mode, "-c", "-o", object.to_str().unwrap()];
                flags.extend_from_slice(backend);
                let output = bcc(
                    "int f(int x){switch(x){case 4:return 0;case (unsigned char)(long)(char \
                     *)260:return 1;}return 2;}",
                    &flags,
                );
                assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
                assert!(
                    text(&output.stderr).contains("case value repeated"),
                    "{}",
                    text(&output.stderr)
                );
                assert!(!text(&output.stderr).contains("internal error"));
                assert!(!object.exists());
            }
        }
        let object = temporary("folded-pointer-sizeof.o");
        let output = bcc(
            "int f2[sizeof(char[(unsigned char)(long)(char *)260])];",
            &["-std=c99", "-c", "-o", object.to_str().unwrap()],
        );
        assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
        assert!(!object.exists());
    }

    #[test]
    fn null_pointer_arithmetic_emits_zero_data_without_fixups() {
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            let object = temporary("null-pointer-arithmetic.o");
            let mut flags = vec![
                "--target=x86_64-unknown-linux-gnu",
                "-c",
                "-o",
                object.to_str().unwrap(),
            ];
            flags.extend_from_slice(backend);
            let output = bcc(
                "char *a __attribute__((section(\".bss.a\"))) = (char *)0 + 0;\nint *b \
                 __attribute__((section(\".bss.b\"))) = 0 + (int *)0;\nchar *c = (char *)0 - 0;\n",
                &flags,
            );
            assert!(
                output.status.success(),
                "{backend:?}: {}",
                text(&output.stderr)
            );
            assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
            assert!(object.exists());
            if cfg!(target_os = "linux") {
                let relocations = Command::new("readelf")
                    .arg("-r")
                    .arg(&object)
                    .output()
                    .unwrap();
                assert!(relocations.status.success());
                assert!(
                    text(&relocations.stdout).contains("There are no relocations"),
                    "{}",
                    text(&relocations.stdout)
                );
            }
            std::fs::remove_file(object).unwrap();
        }
    }

    /// Neither LLVM's objects nor the native back end's direct COFF objects,
    /// its Windows default, involve the MinGW installation.
    #[test]
    fn windows_gnu_object_output_needs_no_mingw() {
        for flags in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--opt=bcc"][..],
        ] {
            let object = temporary("no-mingw.o");
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--target=x86_64-pc-windows-gnu", "-c", "--input", SUM, "-o"])
                .arg(&object)
                .args(flags)
                .env_remove("MINGW_ROOT")
                .env("PATH", "")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{flags:?}: {}",
                text(&output.stderr)
            );
            let bytes = std::fs::read(&object).unwrap();
            assert!(bytes.len() > 20, "{flags:?}: object has no COFF header");
            assert_eq!(
                &bytes[..2],
                &[0x64, 0x86],
                "{flags:?}: object is x86-64 COFF"
            );
            std::fs::remove_file(object).unwrap();
        }
    }

    /// GitHub issue #87: the native back end encodes COFF objects in process
    /// by default on both Windows targets, and Linux keeps the assembler.
    /// `--native-object` selects the encoder and `--no-native-object` the
    /// bundled clang's assembly of the listing, the last of the two winning.
    #[test]
    fn native_objects_are_direct_by_default_on_windows_only() {
        let clang = bundled_clang();
        if !clang.is_file() {
            eprintln!("skipping: no bundled clang at {}", clang.display());
            return;
        }
        for target in [
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu",
        ] {
            for optimize in [&[][..], &["--opt=bcc"][..]] {
                let target_flag = format!("--target={target}");
                let common = [&[&target_flag[..], "--backend=native"][..], optimize].concat();
                let object = |flags: &[&str]| {
                    let path = temporary("native-default.o");
                    let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
                    _ = command
                        .args(&common)
                        .args(flags)
                        .args(["-c", "--input", HELLO, "-o"])
                        .arg(&path)
                        .env("NO_COLOR", "1");
                    if target.ends_with("windows-gnu") {
                        // bcc's toolchain lookup asks for a MinGW root before
                        // clang assembles even an object, on every host; this
                        // header-free source needs none.
                        _ = command.env("MINGW_ROOT", env::temp_dir());
                    }
                    let output = command.output().unwrap();
                    assert!(
                        output.status.success(),
                        "{target} {optimize:?} {flags:?}: {}",
                        text(&output.stderr)
                    );
                    let bytes = std::fs::read(&path).unwrap();
                    std::fs::remove_file(path).unwrap();
                    bytes
                };
                // The assembler path is exactly clang's object for the listing.
                let listing = bcc(HELLO, &[&common[..], &["--emit=asm"]].concat());
                assert!(listing.status.success(), "{}", text(&listing.stderr));
                let assembly = temporary("native-default.s");
                let reference = temporary("native-default-reference.o");
                std::fs::write(&assembly, &listing.stdout).unwrap();
                let assembled = Command::new(&clang)
                    .arg(&target_flag)
                    .arg("-c")
                    .arg(&assembly)
                    .arg("-o")
                    .arg(&reference)
                    .output()
                    .unwrap();
                assert!(assembled.status.success(), "{}", text(&assembled.stderr));
                let clang_object = std::fs::read(&reference).unwrap();
                std::fs::remove_file(assembly).unwrap();
                std::fs::remove_file(reference).unwrap();

                let direct = object(&["--native-object"]);
                let text_path = object(&["--no-native-object"]);
                let context = format!("{target} {optimize:?}");
                assert!(text_path == clang_object, "{context}: not clang's object");
                assert!(
                    object(&["--no-native-object", "--native-object"]) == direct,
                    "{context}: the last flag must win"
                );
                assert!(
                    object(&["--native-object", "--no-native-object"]) == text_path,
                    "{context}: the last flag must win"
                );
                let (default, path) = if target.contains("windows") {
                    (&direct, "direct")
                } else {
                    (&text_path, "assembler")
                };
                assert!(
                    object(&[]) == *default,
                    "{context}: the default is not the {path} path"
                );
            }
        }
    }

    /// The default direct COFF writer assembles a listing its encoder
    /// refuses, as every Windows default object was assembled before direct
    /// objects became the default; only `--native-object` keeps the refusal.
    #[test]
    fn default_windows_native_objects_assemble_what_the_encoder_refuses() {
        if !bundled_clang().is_file() {
            eprintln!("skipping: no bundled clang to assemble the listing");
            return;
        }
        let cases = [
            (
                "__attribute__((section(\"my data\"))) int x = 1;\n",
                "unsupported COFF section name",
            ),
            (
                "__attribute__((section(\".pdata\"))) const int x = 1;\n",
                "user section conflicts with generated COFF unwind section",
            ),
            (
                "__attribute__((dllexport)) int caf\\u00e9(void) { return 3; }\n",
                "unsupported COFF string character",
            ),
        ];
        for target in [
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-pc-windows-msvc",
        ] {
            for (source, refusal) in cases {
                let compile = |flags: &[&str]| {
                    let object = temporary("native-refused.o");
                    let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
                    _ = command
                        .args([target, "--backend=native", "-c", "-o"])
                        .arg(&object)
                        .args(flags)
                        .args(["--input", source])
                        .env("NO_COLOR", "1");
                    if target.ends_with("windows-gnu") {
                        // bcc's toolchain lookup asks for a MinGW root before
                        // clang assembles even an object, on every host; this
                        // header-free source needs none.
                        _ = command.env("MINGW_ROOT", env::temp_dir());
                    }
                    let output = command.output().unwrap();
                    let bytes = std::fs::read(&object).ok();
                    if bytes.is_some() {
                        std::fs::remove_file(object).unwrap();
                    }
                    (output, bytes)
                };
                let (refused, object) = compile(&["--native-object"]);
                assert_eq!(refused.status.code(), Some(1), "{target} {source}");
                assert!(
                    text(&refused.stderr).contains(refusal),
                    "{target} {source}: {}",
                    text(&refused.stderr)
                );
                assert!(object.is_none(), "{target} {source}");
                let (built, default) = compile(&[]);
                assert!(
                    built.status.success(),
                    "{target} {source}: {}",
                    text(&built.stderr)
                );
                let (built, assembled) = compile(&["--no-native-object"]);
                assert!(built.status.success(), "{}", text(&built.stderr));
                assert!(
                    default.is_some() && default == assembled,
                    "{target} {source}: the default is not the assembler's object"
                );
            }
        }
    }

    /// When the default writer cannot assemble a listing its encoder refused,
    /// here because only the assembler needs MinGW, the error gives the
    /// refusal as well as the assembler's failure: an encodable listing would
    /// have built without MinGW.
    #[test]
    fn default_windows_native_objects_report_the_refusal_when_assembly_fails() {
        let object = temporary("native-refused-no-mingw.o");
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["--target=x86_64-w64-windows-gnu", "--backend=native", "-c"])
            .args([
                "--input",
                "__attribute__((section(\"my data\"))) int x = 1;\n",
            ])
            .arg("-o")
            .arg(&object)
            .env_remove("MINGW_ROOT")
            .env("PATH", "")
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        let stderr = text(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        // Without the bundled clang the assembler fails before it looks for
        // MinGW, and the refusal must still be given.
        assert!(
            stderr.contains(
                "native object emission: unsupported COFF section name; assembling the listing \
                 instead failed: cannot find "
            ),
            "{stderr}"
        );
        if bundled_clang().is_file() {
            assert!(stderr.contains("cannot find MinGW"), "{stderr}");
        }
        assert!(!object.exists(), "{} was written", object.display());
    }

    /// The default writer gives the encoder's refusal only when assembling
    /// the refused listing fails. A missing output directory, the MinGW an
    /// executable links with, and a link error fail an encodable listing as
    /// well, so a refused one reports them in the same words, without the
    /// refusal.
    #[test]
    fn default_windows_native_fallback_reports_shared_failures_plainly() {
        const PROGRAM: &str =
            "extern int missing(void);\nint main(void) { return x + missing(); }\n";
        if !bundled_clang().is_file() {
            eprintln!("skipping: no bundled clang to assemble or link");
            return;
        }
        let refused = format!("__attribute__((section(\"my data\"))) int x = 1;\n{PROGRAM}");
        let encodable = format!("int x = 1;\n{PROGRAM}");
        let unwritable = temporary("native-refused-missing-directory").join("x.o");
        let executable = temporary("native-refused-shared-failure.exe");
        let cases: [(&str, &[&str], &Path, &str); 3] = [
            ("--target=x86_64-pc-windows-msvc", &["-c"], &unwritable, ""),
            (
                "--target=x86_64-w64-windows-gnu",
                &[],
                &executable,
                "cannot find MinGW",
            ),
            (
                "--target=x86_64-pc-windows-msvc",
                &[],
                &executable,
                "lld-link: error:",
            ),
        ];
        for (target, flags, output, cause) in cases {
            let build = |source: &str| {
                let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
                _ = command
                    .args([target, "--backend=native"])
                    .args(flags)
                    .arg("-o")
                    .arg(output)
                    .args(["--input", source])
                    .env_remove("MINGW_ROOT")
                    .env("NO_COLOR", "1");
                if target.ends_with("windows-gnu") {
                    // No MinGW on `PATH` either; clang never starts, since
                    // the toolchain lookup fails first.
                    _ = command.env("PATH", "");
                }
                let built = command.output().unwrap();
                assert_eq!(built.status.code(), Some(1), "{target} {flags:?}");
                assert!(!output.exists(), "{} was written", output.display());
                text(&built.stderr)
            };
            let (refused, encodable) = (build(&refused), build(&encodable));
            assert!(
                !refused.contains("native object emission") && refused.contains(cause),
                "{target} {flags:?}: {refused}"
            );
            assert_eq!(
                refused.lines().next(),
                encodable.lines().next(),
                "{target} {flags:?}"
            );
        }
    }

    /// The bundled clang, which assembles the native listing.
    fn bundled_clang() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        })
    }

    /// The MinGW-w64 installation root and its C library include directory,
    /// from `MINGW_ROOT` or a `gcc.exe` on `PATH`, as the build finds them.
    fn mingw() -> Option<(PathBuf, PathBuf)> {
        let installation = |root: &Path| {
            ["include", "x86_64-w64-mingw32/include"]
                .into_iter()
                .map(|relative| root.join(relative))
                .find(|include| {
                    include.join("_mingw.h").is_file() && include.join("windows.h").is_file()
                })
                .map(|include| (root.to_path_buf(), include))
        };
        if let Some(root) = env::var_os("MINGW_ROOT") {
            return installation(Path::new(&root));
        }
        let path = env::var_os("PATH")?;
        env::split_paths(&path).find_map(|directory| {
            let root = directory.parent()?;
            directory
                .join("gcc.exe")
                .is_file()
                .then(|| installation(root))
                .flatten()
        })
    }

    /// GitHub issue #57: MinGW-w64's `_mingw.h` declares its `__CRT_INLINE`
    /// helpers `extern inline __attribute__((__gnu_inline__))` only for GCC
    /// 4.3 or later or for Clang. bcc claims Clang, so two translation units
    /// that include `<string.h>` emit no external `strncasecmp` and link.
    #[test]
    fn mingw_crt_inline_helpers_link_from_two_translation_units() {
        if host() != Some("x86_64-w64-windows-gnu") {
            eprintln!("skipping: the host is not Windows GNU");
            return;
        }
        let Some((root, include)) = mingw() else {
            eprintln!("skipping: no MinGW-w64 installation in MINGW_ROOT or PATH");
            return;
        };
        let units = [
            "#include <string.h>\nint one(void) { return strcasecmp(\"a\", \"A\"); }\n",
            "#include <stdio.h>\n#include <string.h>\nint one(void);\nint main(void) {\n  \
             printf(\"%d %d\\n\", one(), strncasecmp(\"ab\", \"AB\", 2));\n  return 0;\n}\n",
        ];
        let flags = [
            "--target=x86_64-w64-windows-gnu",
            "--idirafter",
            include.to_str().unwrap(),
        ];
        let mut objects = Vec::new();
        for (index, unit) in units.iter().enumerate() {
            let llvm = bcc(unit, &[&flags[..], &["--emit=llvm"]].concat());
            assert!(llvm.status.success(), "{}", text(&llvm.stderr));
            let module = text(&llvm.stdout);
            assert!(
                !module.lines().any(|line| line.starts_with("define")
                    && (line.contains("@strcasecmp(") || line.contains("@strncasecmp("))),
                "unit {index} defines a CRT inline helper:\n{module}"
            );
            let object = temporary(&format!("crt-inline-{index}.o"));
            let compiled = bcc(
                unit,
                &[&flags[..], &["-c", "-o", object.to_str().unwrap()]].concat(),
            );
            assert!(compiled.status.success(), "{}", text(&compiled.stderr));
            objects.push(object);
        }
        let executable = temporary("crt-inline.exe");
        let linked =
            Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/llvm/bin/clang.exe"))
                .arg("--target=x86_64-w64-windows-gnu")
                .arg(format!("--sysroot={}", root.display()))
                .arg("-fuse-ld=lld")
                .args(&objects)
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap();
        assert!(linked.status.success(), "{}", text(&linked.stderr));
        let run = run_native(&mut Command::new(&executable)).unwrap();
        assert_eq!(run.status.code(), Some(0));
        assert_eq!(text(&run.stdout), "0 0\n");
        for path in objects.into_iter().chain([executable]) {
            drop(std::fs::remove_file(path));
        }
    }

    #[test]
    fn looping_native_program_reports_timeout() {
        let Some(target) = host() else {
            return;
        };
        let executable = temporary("loop.exe");
        // A volatile access keeps the deliberate infinite loop defined in C.
        let built = bcc(
            "int printf(const char *, ...); int fflush(void *); int main(void) { volatile int x = \
             0; printf(\"loop started\\n\"); fflush(0); for (;;) x = 0; }",
            &[
                &format!("--target={target}"),
                "-o",
                executable.to_str().unwrap(),
            ],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let report = run_until_timeout(&mut Command::new(&executable), "loop started");
        assert!(report.contains("native execution timed out"), "{report}");
        assert!(report.contains("loop.exe"), "{report}");
        std::fs::remove_file(executable).unwrap();
    }

    #[test]
    fn native_output_exceeding_pipe_capacity_is_captured() {
        let Some(target) = host() else {
            return;
        };
        let executable = temporary("large-output.exe");
        let built = bcc(
            "int putchar(int); int main(void) { for (int i = 0; i < 200000; i++) putchar('x'); \
             return 7; }",
            &[
                &format!("--target={target}"),
                "-o",
                executable.to_str().unwrap(),
            ],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let output = run_native(&mut Command::new(&executable)).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, vec![b'x'; 200_000]);
        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
        std::fs::remove_file(executable).unwrap();
    }
}
