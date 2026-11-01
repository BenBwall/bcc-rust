//! Wrong code the GCC 15.2 testsuite survey under `--gcc-mode` found, each
//! checked on every back end: the IR interpreter, LLVM at `-O0` and `-O2`,
//! and the native back end. Each program aborts on a wrong value, so it
//! checks itself; the ones that print print what GCC and Clang print.
//!
//! - Bytes of malformed UTF-8 in literals stay bytes, as GCC and Clang keep
//!   them (gcc.c-torture/execute/20000227-1.c); the source files are written at
//!   run time so the repository holds no malformed UTF-8.
//! - A union whose first member is an x87 `long double` initialized with `{0}`
//!   is zeroed whole, and one initialized with another constant is copied from
//!   a constant image (gcc.dg/torture/pr58416.c).
//! - `__alignof__` of a function reads its `aligned` attribute
//!   (gcc.c-torture/execute/align-3.c).
//! - `va_arg` evaluates the size expressions of its type name
//!   (gcc.dg/vla-14.c), and under GCC's rules an `alloca` block outlives the
//!   VLA scope it was made in (gcc.dg/vla-24.c).
//! - Under C90 a selection or iteration statement forms no block, so a tag or
//!   enumeration constant declared in its controlling expression stays visible
//!   after it (gcc.dg/c90-scope-1.c).
//! - Under GCC's rules a floating constant converted out of an integer's range
//!   folds to the bound GCC saturates it to
//!   (gcc.c-torture/execute/20031003-1.c).
//! - A trailing array reached through a pointer has no subobject size of its
//!   own for `__builtin_object_size` mode 1 (`gcc.dg/pr101836_5.c`).

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

    const IDENTITIES: [&str; 2] = ["--compiler-identity=clang", "--compiler-identity=gcc"];

    #[test]
    fn malformed_utf8_bytes_stay_bytes_in_narrow_and_utf8_literals() {
        let source = b"#include <stdio.h>\n\
            static const unsigned char f[] = \"\\0\\377\";\n\
            static const unsigned char g[] = \"\\0\xff\";\n\
            int main(void) {\n\
            const char *h = \"a\xe2\x82z\";\n\
            int c = '\xff', m = '\xe2\x82';\n\
            if (sizeof f != 3 || sizeof g != 3 || f[1] != g[1]) return 1;\n\
            if (h[1] != '\\342' || h[2] != '\\202' || h[3] != 'z') return 2;\n\
            if (c != (char)0xff || m != 0xe282) return 3;\n\
            puts(\"ok\");\n\
            return 0;\n\
            }\n";
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, source, &["-w"], "ok\n");
        }
        let utf8 = b"int main(void) {\n\
            const char *s = (const char *)u8\"\xff\";\n\
            return !(sizeof u8\"\xff\" == 2 && (unsigned char)s[0] == 0xff);\n\
            }\n";
        prints_on_every_back_end("--compiler-identity=gcc", utf8, &["-std=gnu23"], "");
    }

    /// A `#line` name keeps its bytes, so `__FILE__` spells them, as in GCC
    /// and Clang.
    #[test]
    fn line_names_keep_malformed_utf8_bytes() {
        let source = b"#include <stdio.h>\n#line 7 \"f\xff.c\"\n\
            int main(void) {\n\
            for (const char *p = __FILE__; *p; p++) printf(\"%02x \", (unsigned char)*p);\n\
            printf(\"%d\\n\", __LINE__);\n\
            return 0;\n\
            }\n";
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, source, &["-w"], "66 ff 2e 63 9\n");
        }
    }

    /// A quoted header name holding bytes of malformed UTF-8 opens the file
    /// whose name holds those bytes, and `__FILE__` keeps them.
    #[cfg(unix)]
    #[test]
    fn include_names_keep_malformed_utf8_bytes() {
        use std::{
            fmt::Write as _,
            os::unix::ffi::OsStrExt as _,
        };
        let name = [
            format!("bcc-survey-{}-h", std::process::id()).as_bytes(),
            b"\xff.h",
        ]
        .concat();
        let header = std::env::temp_dir().join(std::ffi::OsStr::from_bytes(&name));
        std::fs::write(&header, b"static const char *header_file = __FILE__;\n").unwrap();
        let source = [
            &b"#include <stdio.h>\n#include <string.h>\n#include \""[..],
            &name,
            b"\"\n\
            int main(void) {\n\
            for (const char *p = strrchr(header_file, '/') + 1; *p; p++)\n\
            printf(\"%02x \", (unsigned char)*p);\n\
            return 0;\n\
            }\n",
        ]
        .concat();
        let expected = name.iter().fold(String::new(), |mut text, byte| {
            write!(text, "{byte:02x} ").unwrap();
            text
        });
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, &source, &["-w"], &expected);
        }
        std::fs::remove_file(header).unwrap();
    }

    /// Clang warns once per unprefixed literal and rejects prefixed ones;
    /// GCC is silent but cannot convert them to the wide encodings.
    #[test]
    fn malformed_utf8_literals_are_diagnosed_as_each_identity() {
        let narrow = b"const char *s = \"a\xffb\";\nint c = '\xff';\n";
        let output = check("--compiler-identity=clang", narrow, &[]);
        let stderr = text(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert_eq!(
            stderr
                .matches(
                    "warning: illegal character encoding in string literal \
                     [-Winvalid-source-encoding]"
                )
                .count(),
            1,
            "{stderr}"
        );
        assert!(
            stderr.contains(
                "warning: illegal character encoding in character literal \
                 [-Winvalid-source-encoding]"
            ),
            "{stderr}"
        );
        assert!(stderr.contains("\"a<FF>b\""), "{stderr}");
        let output = check(
            "--compiler-identity=clang",
            narrow,
            &["-Wno-invalid-source-encoding"],
        );
        assert_eq!(text(&output.stderr), "", "the group silences it");
        let output = check("--compiler-identity=clang", narrow, &["-pedantic-errors"]);
        let stderr = text(&output.stderr);
        assert!(!output.status.success(), "{stderr}");
        assert_eq!(
            stderr
                .matches("error: illegal character encoding in ")
                .count(),
            2,
            "-pedantic-errors promotes it: {stderr}"
        );
        let output = check(
            "--compiler-identity=clang",
            narrow,
            &["-pedantic-errors", "-Wno-invalid-source-encoding"],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        let output = check("--compiler-identity=gcc", narrow, &["-pedantic-errors"]);
        assert!(output.status.success(), "{}", text(&output.stderr));
        let output = check("--compiler-identity=gcc", narrow, &[]);
        assert!(output.status.success());
        assert_eq!(text(&output.stderr), "");
        for prefix in ["u8", "u", "U", "L"] {
            let source = [
                format!("const void *s = {prefix}\"a").as_bytes(),
                b"\xff\";\n",
            ]
            .concat();
            let clang = check("--compiler-identity=clang", &source, &["-std=gnu23"]);
            assert!(!clang.status.success(), "{prefix}");
            assert!(
                text(&clang.stderr)
                    .contains("error: illegal character encoding in string literal\n"),
                "{prefix}: {}",
                text(&clang.stderr)
            );
            let gcc = check("--compiler-identity=gcc", &source, &["-std=gnu23"]);
            assert_eq!(gcc.status.success(), prefix == "u8", "{prefix}");
            if prefix != "u8" {
                assert!(
                    text(&gcc.stderr).contains(
                        "error: converting to execution character set: Invalid or incomplete \
                         multibyte or wide character"
                    ),
                    "{prefix}: {}",
                    text(&gcc.stderr)
                );
            }
        }
        let stray = check("--compiler-identity=gcc", b"int x;\n\xff\n", &[]);
        assert!(
            text(&stray.stderr).contains("error: stray byte <FF> in source"),
            "{}",
            text(&stray.stderr)
        );
    }

    /// An unprefixed literal joined to a prefixed one takes its prefix:
    /// Clang's warning becomes the prefixed literal's error, and GCC
    /// cannot convert the bytes to a wide encoding but keeps them in `u8`.
    #[test]
    fn malformed_utf8_joined_to_prefixed_literals_is_diagnosed() {
        for (prefix, gcc_error) in [("L", true), ("u", true), ("u8", false)] {
            let source = [
                format!("const void *s = {prefix}\"a\" \"b").as_bytes(),
                b"\xff\" ",
                format!("{prefix}\"w\";\nconst char *n = \"c").as_bytes(),
                b"\xff\" \"d\";\n",
            ]
            .concat();
            let clang = check("--compiler-identity=clang", &source, &["-std=gnu23"]);
            let stderr = text(&clang.stderr);
            assert!(!clang.status.success(), "{prefix}: {stderr}");
            assert_eq!(
                stderr
                    .matches("error: illegal character encoding in string literal\n")
                    .count(),
                1,
                "{prefix}: {stderr}"
            );
            assert_eq!(
                stderr
                    .matches(
                        "warning: illegal character encoding in string literal \
                         [-Winvalid-source-encoding]"
                    )
                    .count(),
                1,
                "{prefix}: only the unprefixed join warns: {stderr}"
            );
            let clang = check(
                "--compiler-identity=clang",
                &source,
                &["-std=gnu23", "-Wno-invalid-source-encoding"],
            );
            assert!(
                !clang.status.success(),
                "{prefix}: the group leaves the error"
            );
            let gcc = check("--compiler-identity=gcc", &source, &["-std=gnu23"]);
            let stderr = text(&gcc.stderr);
            assert_eq!(gcc.status.success(), !gcc_error, "{prefix}: {stderr}");
            assert_eq!(
                stderr.contains("error: converting to execution character set"),
                gcc_error,
                "{prefix}: {stderr}"
            );
        }
    }

    /// `-E` copies the bytes of a string literal that is not UTF-8, as GCC
    /// does.
    #[test]
    fn preprocessing_keeps_malformed_utf8_bytes() {
        let input = temporary("raw.c");
        std::fs::write(&input, b"#define S(x) #x\nconst char *t = S(\xffz);\n").unwrap();
        let output = bcc(&["--gcc-mode", "-E", "-P", input.to_str().unwrap()]);
        std::fs::remove_file(&input).unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert_eq!(output.stdout, b"const char *t = \"\xffz\";\n");
    }

    /// An initializer that a later one overrides entirely is not evaluated,
    /// as GCC and Clang leave it (C11 §6.7.9p19, footnote 151; GCC's
    /// gcc.dg/cpp/embed-12.c): `f` runs only for the values that reach the
    /// object.
    #[test]
    fn overridden_initializers_are_not_evaluated() {
        let source = b"#include <stdio.h>\n\
            int n;\n\
            int f(int x) { n++; return x; }\n\
            struct p { int a, b; };\n\
            int main(void) {\n\
            int a[3] = { [1] = f(1), [1] = 5, [2] = f(2) };\n\
            struct p s = { .a = f(3), .b = 1, .a = 4 };\n\
            struct p t[2] = { [0] = { f(5), f(6) }, [0].a = 9, [1] = { f(7) }, [1] = { 4, 5 } };\n\
            printf(\"%d %d %d %d %d %d %d\\n\", a[1], a[2], s.a, t[0].a, t[0].b, t[1].a, n);\n\
            return 0;\n\
            }\n";
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, source, &["-w"], "5 2 4 9 6 4 2\n");
        }
    }

    #[test]
    fn long_double_union_zero_initializer_clears_every_byte() {
        let source = b"struct s { char s[sizeof(long double)]; };\n\
            union u { long double d; struct s s; };\n\
            struct t { long double d; int k; };\n\
            __attribute__((noinline)) static int zero(const void *p, unsigned n) {\n\
            const unsigned char *b = p;\n\
            for (unsigned i = 0; i < n; i++) if (b[i]) return 0;\n\
            return 1;\n\
            }\n\
            int main(void) {\n\
            union u x = {0};\n\
            if (!zero(&x, sizeof x)) return 1;\n\
            x.s = (struct s){\"xxxxxxxxxxxxxxxx\"};\n\
            union u y = x;\n\
            for (unsigned char *p = (unsigned char *)&y + sizeof y; p-- > (unsigned char *)&y;)\n\
            if (*p != 'x') return 2;\n\
            struct t z = {0.0L, 0};\n\
            if (!zero(&z, sizeof z)) return 3;\n\
            long double w[3] = {0};\n\
            if (!zero(w, sizeof w)) return 4;\n\
            return 0;\n\
            }\n";
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, source, &[], "");
        }
    }

    /// A constant initializer that stores a non-zero `long double` is copied
    /// from a constant image, so a later copy of the union keeps the bytes
    /// another member wrote over it. Reading the `long double` member of the
    /// same union first breaks Clang's copy too, so other objects check the
    /// values.
    #[test]
    fn long_double_union_constant_initializer_keeps_later_bytes() {
        let source = b"union u { long double d; char s[sizeof(long double)]; };\n\
            union c { _Complex long double d; char s[sizeof(_Complex long double)]; };\n\
            struct t { int k; long double d; };\n\
            struct w { int k; union u u[2]; };\n\
            int main(void) {\n\
            union u v = {1.0L}, x = {1.0L};\n\
            if (v.d != 1.0L) return 1;\n\
            for (unsigned i = 0; i < sizeof x.s; i++) x.s[i] = 'x';\n\
            union u y = x;\n\
            for (unsigned i = 0; i < sizeof y.s; i++) if (y.s[i] != 'x') return 2;\n\
            union u o = {-0.0L}, n = {-0.0L};\n\
            if (o.d != 0.0L || !__builtin_signbit(o.d)) return 3;\n\
            for (unsigned i = 0; i < sizeof n.s; i++) n.s[i] = 'x';\n\
            union u m = n;\n\
            for (unsigned i = 0; i < sizeof m.s; i++) if (m.s[i] != 'x') return 4;\n\
            union c a = {1.0L}, z = {1.0L};\n\
            if (__real__ a.d != 1.0L || __imag__ a.d != 0.0L) return 5;\n\
            for (unsigned i = 0; i < sizeof z.s; i++) z.s[i] = 'x';\n\
            union c w = z;\n\
            for (unsigned i = 0; i < sizeof w.s; i++) if (w.s[i] != 'x') return 6;\n\
            struct t r = {7, 2.5L};\n\
            if (r.k != 7 || r.d != 2.5L) return 7;\n\
            struct w e = {1, {{2.0L}, {.d = 3.0L}}}, b = {1, {{2.0L}, {.d = 3.0L}}};\n\
            if (e.k != 1 || e.u[0].d != 2.0L || e.u[1].d != 3.0L) return 8;\n\
            for (unsigned i = 0; i < sizeof b.u[1].s; i++) b.u[1].s[i] = 'x';\n\
            struct w g = b;\n\
            for (unsigned i = 0; i < sizeof g.u[1].s; i++) if (g.u[1].s[i] != 'x') return 9;\n\
            return 0;\n\
            }\n";
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, source, &[], "");
        }
    }

    /// As in Clang, only an object of at most 32 bytes, or one needing more
    /// than six stores besides its zeroes, is copied from a constant image;
    /// a large, mostly zero one is filled with zeroes and stores its `long
    /// double` as integers, with no image of its size.
    #[test]
    fn long_double_initializers_copy_an_image_only_when_small() {
        let large = check(
            "--emit=ir",
            b"long double use(long double *);\n\
              long double f(void) { long double a[1 << 22] = {1.0L}; return use(a); }\n",
            &[],
        );
        let ir = text(&large.stdout);
        assert!(large.status.success(), "{}", text(&large.stderr));
        assert!(!ir.contains("global "), "{ir}");
        assert!(ir.contains("fill "), "{ir}");
        assert!(ir.contains("store.i16 "), "{ir}");
        assert!(!ir.contains("store.f80 "), "{ir}");
        assert!(ir.len() < 4096, "{} bytes of IR", ir.len());
        let small = check(
            "--emit=ir",
            b"union u { long double d; char s[16]; };\n\
              void use(union u *);\n\
              void f(void) { union u x = {1.0L}; use(&x); }\n",
            &[],
        );
        let ir = text(&small.stdout);
        assert!(small.status.success(), "{}", text(&small.stderr));
        assert!(
            ir.contains("global @.initializer internal constant size 16"),
            "{ir}"
        );
    }

    /// GCC gives a function type the alignment 1, Clang 4; an `aligned`
    /// attribute replaces it in both.
    #[test]
    fn function_alignof_reads_the_aligned_attribute() {
        let source = b"#include <stdio.h>\n\
            void f(void) __attribute__((aligned(256)));\n\
            void f(void) {}\n\
            void h(void) __attribute__((aligned(2)));\n\
            void g(void);\n\
            int main(void) {\n\
            printf(\"%zu %zu %zu %zu\\n\", __alignof__(f), __alignof__(h), __alignof__(g),\n\
            __alignof__(void(void)));\n\
            return (unsigned long)&f % 256 != 0;\n\
            }\n\
            void h(void) {}\n";
        prints_on_every_back_end(IDENTITIES[0], source, &[], "256 2 4 4\n");
        prints_on_every_back_end(IDENTITIES[1], source, &[], "256 2 1 1\n");
    }

    #[test]
    fn va_arg_evaluates_its_type_names_bounds() {
        let source = b"#include <stdarg.h>\n\
            int a[10];\n\
            int i = 9;\n\
            static int f(int n, ...) {\n\
            va_list ap;\n\
            va_start(ap, n);\n\
            void *p = va_arg(ap, __typeof__(int (*)[++i]));\n\
            va_end(ap);\n\
            return p == a && i == n;\n\
            }\n\
            int main(void) { return !f(10, &a); }\n";
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, source, &["-std=gnu99"], "");
        }
    }

    /// GCC keeps an `alloca` block, and the VLA scope it was made in, until
    /// the function returns; Clang releases both with the scope.
    #[test]
    fn alloca_outlives_its_vla_scope_under_gcc() {
        let source = b"__attribute__((noinline)) static int kept(int n) {\n\
            char *p, *q;\n\
            if (1) {\n\
            char vla[n];\n\
            vla[0] = 0;\n\
            p = __builtin_alloca(8);\n\
            p[0] = 1;\n\
            }\n\
            q = __builtin_alloca(64);\n\
            __builtin_memset(q, 0, 64);\n\
            return p[0];\n\
            }\n\
            int main(void) { return !kept(48); }\n";
        prints_on_every_back_end("--compiler-identity=gcc", source, &["-std=gnu99"], "");
    }

    /// C90 has no blocks for selection and iteration statements: a tag or
    /// enumeration constant, which may hide a typedef name, declared in a
    /// controlling expression belongs to the enclosing block, as in GCC and
    /// Clang. A compound substatement is still a block of its own.
    #[test]
    fn c90_selection_and_iteration_statements_form_no_blocks() {
        let source = b"struct foo { char a; };\n\
            typedef int T;\n\
            int sfoo(void) {\n\
            if (sizeof (struct foo { int a; double b; }))\n\
            (void) 0;\n\
            return sizeof (struct foo);\n\
            }\n\
            int counted(void) {\n\
            int n = 0;\n\
            while (sizeof (enum { E = 3 }) && n < E)\n\
            n++;\n\
            return E + n;\n\
            }\n\
            int shadowed(void) {\n\
            if (sizeof (enum { T = 2 }))\n\
            ;\n\
            return T;\n\
            }\n\
            int compound(void) {\n\
            if (1) {\n\
            struct foo { int w[4]; };\n\
            }\n\
            return sizeof (struct foo);\n\
            }\n\
            int main(void) {\n\
            return sfoo() != 16 || counted() != 6 || shadowed() != 2 || compound() != 1;\n\
            }\n";
        for identity in IDENTITIES {
            for standard in ["-std=c90", "-std=gnu89"] {
                prints_on_every_back_end(identity, source, &[standard, "-pedantic-errors"], "");
            }
        }
        let c99 = b"struct foo { char a; };\n\
            int main(void) {\n\
            if (sizeof (struct foo { int a; double b; }))\n\
            (void) 0;\n\
            for (int i = 0; i < (int)sizeof (struct foo { int w[4]; }); i++)\n\
            ;\n\
            return sizeof (struct foo) != 1;\n\
            }\n";
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, c99, &["-std=c99"], "");
        }
        // A `for` declaration, a C99 feature accepted under gnu89, keeps its
        // block.
        let declaration = b"typedef int T;\n\
            int main(void) {\n\
            for (int T = 0; T < 1; T++)\n\
            ;\n\
            T x = 0;\n\
            return x;\n\
            }\n";
        prints_on_every_back_end(IDENTITIES[0], declaration, &["-std=gnu89", "-w"], "");
    }

    /// GCC folds a floating constant converted out of an integer type's
    /// range to the bound it saturates to (0 for a NaN) in code, as in a
    /// static initializer; the conversion is undefined, and Clang converts
    /// at run time.
    #[test]
    fn out_of_range_float_constants_fold_to_gccs_bound() {
        let source = b"__attribute__((noinline)) int f1(void) { return (int)2147483648.0f; }\n\
            __attribute__((noinline)) int f2(void) { return (int)(float)2147483647; }\n\
            __attribute__((noinline)) unsigned f3(void) { return (unsigned)-1.0; }\n\
            __attribute__((noinline)) int f4(void) { return (int)__builtin_nan(\"\"); }\n\
            __attribute__((noinline)) int f5(void) { return (int)-1e10 == -2147483647 - 1; }\n\
            int main(void) {\n\
            return f1() != 2147483647 || f2() != 2147483647 || f3() != 0 || f4() != 0 || !f5();\n\
            }\n";
        prints_on_every_back_end("--compiler-identity=gcc", source, &["-w"], "");
    }

    /// A trailing array member reached through a pointer may extend past
    /// its bound, as a flexible array member does, so GCC and Clang answer
    /// `__builtin_object_size` mode 1 for it from the complete object
    /// (unknown here); mode 3 keeps its declared size, and so does an array
    /// that is no trailing one or that of a declared object.
    #[test]
    fn trailing_arrays_through_pointers_have_no_subobject_bound() {
        let source = b"#include <stdio.h>\n\
            struct a { int x; int c[4]; };\n\
            struct b { int c[4]; int x; };\n\
            struct n { int y; struct a in; };\n\
            union u { int c[2]; char d; };\n\
            __attribute__((noinline)) void f(struct a *p, struct b *q, struct n *r, union u *v) {\n\
            struct a s;\n\
            printf(\"%zd %zd %zd %zd %zd %zd %zd %zd\\n\", __builtin_object_size(p->c, 1),\n\
            __builtin_object_size(p->c, 3), __builtin_object_size((*p).c, 1),\n\
            __builtin_object_size(q->c, 1), __builtin_object_size(r->in.c, 1),\n\
            __builtin_object_size(v->c, 1), __builtin_object_size(s.c, 1),\n\
            __builtin_object_size(&p->c[1], 1));\n\
            }\n\
            int main(void) {\n\
            static char buf[64];\n\
            f((void *)buf, (void *)buf, (void *)buf, (void *)buf);\n\
            return 0;\n\
            }\n";
        for identity in IDENTITIES {
            prints_on_every_back_end(identity, source, &[], "-1 16 -1 16 -1 -1 16 -1\n");
        }
    }

    /// `source` exits 0 and prints `expected` interpreted (unoptimized and
    /// with bcc's optimizer) and built by LLVM at `-O0` and `-O2` and by
    /// the native back end. Only on an x86-64 GNU/Linux host: bcc's default
    /// target is Linux on every host, and the programs need its C library.
    fn prints_on_every_back_end(identity: &str, source: &[u8], flags: &[&str], expected: &str) {
        if !cfg!(all(
            target_os = "linux",
            target_arch = "x86_64",
            target_env = "gnu"
        )) {
            eprintln!("skipping: the programs run on an x86-64 GNU/Linux host only");
            return;
        }
        let input = temporary("program.c");
        std::fs::write(&input, source).unwrap();
        let path = input.to_str().unwrap();
        for mode in [
            &["--interpret", "--opt=none"][..],
            &["--interpret", "--opt=bcc"],
        ] {
            let output = bcc(&[&[identity, path][..], flags, mode].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{identity} {mode:?}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), expected, "{identity} {mode:?}");
        }
        for mode in [&["-O0"][..], &["-O2"], &["--backend=native"]] {
            let executable = temporary("program.exe");
            let built = bcc(&[
                &[identity, path, "-o", executable.to_str().unwrap()][..],
                flags,
                mode,
            ]
            .concat());
            assert!(
                built.status.success(),
                "{identity} {mode:?}: {}",
                text(&built.stderr)
            );
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{identity} {mode:?}");
            assert_eq!(text(&run.stdout), expected, "{identity} {mode:?}");
            std::fs::remove_file(executable).unwrap();
        }
        std::fs::remove_file(input).unwrap();
    }

    /// Semantic analysis of `source` alone, with its diagnostics.
    fn check(identity: &str, source: &[u8], flags: &[&str]) -> Output {
        let input = temporary("check.c");
        std::fs::write(&input, source).unwrap();
        let output = bcc(&[&[identity, input.to_str().unwrap()][..], flags].concat());
        std::fs::remove_file(input).unwrap();
        output
    }

    fn bcc(args: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH"),
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
            "bcc-survey-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
