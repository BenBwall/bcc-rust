//! GNU vectors in memory, computed lane by lane, and passed at calls as
//! the claimed compiler's convention passes them.

use super::*;

/// The values the refusal once covered lower now: assignment copies the
/// object, a load through a vector pointer copies from it, a literal stores
/// its lanes and a subscript loads one lane at its offset, with the
/// alignment the offset leaves.
#[test]
fn vector_values_lower_in_memory() {
    for (source, wanted) in [
        (
            "typedef int V __attribute__((vector_size(16))); V x, y; void f(void) { x = y; }",
            "copy",
        ),
        (
            "typedef int V __attribute__((vector_size(16))); V x; void f(int *p) { x = *(V *)p; }",
            "copy",
        ),
        (
            "typedef char V __attribute__((vector_size(4))); char a[4]; void f(void) { *(V *)a = \
             (V){0, 1, 2, 3}; }",
            "store.i8",
        ),
        (
            "typedef int V __attribute__((vector_size(16))); V x; int f(void) { return x[2]; }",
            "load.i32 v3, align 8",
        ),
    ] {
        let text = functions(&lowered(source));
        assert!(text.contains(wanted), "{source}\n{text}");
    }
}

/// An operator works on each lane without the integer promotions and
/// without `nsw`; a comparison sign-extends its truth into a mask lane; a
/// scalar operand is converted once and stored into every lane.
#[test]
fn operators_work_lane_by_lane() {
    let text = functions(&lowered(
        "typedef signed char V __attribute__((vector_size(4)));\nV add(V a, V b) { return a + b; \
         }\nV less(V a, V b) { return a < b; }\nV splat(V a, signed char s) { return a * s; }",
    ));
    assert_eq!(text.matches("iadd.i8 v").count(), 4, "{text}");
    assert!(!text.contains("nsw"), "{text}");
    assert_eq!(text.matches("icmp.i8 slt").count(), 4, "{text}");
    assert_eq!(text.matches("sext.i8").count(), 4, "{text}");
    assert_eq!(text.matches("imul.i8").count(), 4, "{text}");
}

/// System V parameters and results record the registers each compiler
/// uses: one SSE register for 16 bytes, a general register for 4, and, for
/// Clang only, XMM0 and XMM1 for a 32-byte result and an integer for a
/// one-lane `float`, which GCC passes in memory.
#[test]
fn vector_parameters_record_each_compilers_registers() {
    use crate::configuration::{
        CompilerConfiguration,
        CompilerIdentity,
    };
    let source = "typedef float V __attribute__((vector_size(16)));\ntypedef float W \
                  __attribute__((vector_size(32)));\ntypedef float S \
                  __attribute__((vector_size(4)));\ntypedef char C \
                  __attribute__((vector_size(4)));\nV f(V a) { return a; }\nW g(W b) { return b; \
                  }\nS h(S c) { return c; }\nC k(C c) { return c; }";
    for (identity, wanted) in [
        (
            CompilerIdentity::Clang,
            [
                "@f(ptr result(size 16, align 16, vector, [0: f128]), ptr aggregate(size 16, \
                 align 16, vector, [0: f128]) noundef)",
                "@g(ptr result(size 32, align 32, vector, [0: f128, 16: f128]), ptr \
                 aggregate(size 32, align 32, vector) noundef)",
                "@h(ptr result(size 4, align 4, vector, [0: i32]), ptr aggregate(size 4, align 4, \
                 vector, [0: i32]) noundef)",
                "@k(ptr result(size 4, align 4, vector, [0: i32]), ptr aggregate(size 4, align 4, \
                 vector, [0: i32]) noundef)",
            ],
        ),
        (
            CompilerIdentity::Gcc,
            [
                "@f(ptr result(size 16, align 16, vector, [0: f128]), ptr aggregate(size 16, \
                 align 16, vector, [0: f128]) noundef)",
                "@g(ptr result(size 32, align 32, vector), ptr aggregate(size 32, align 32, \
                 vector) noundef)",
                "@h(ptr result(size 4, align 4, vector), ptr aggregate(size 4, align 4, vector) \
                 noundef)",
                "@k(ptr result(size 4, align 4, vector, [0: i32]), ptr aggregate(size 4, align 4, \
                 vector, [0: i32]) noundef)",
            ],
        ),
    ] {
        let configuration = CompilerConfiguration::default().with_compiler_identity(identity);
        let text = lower_with(source, configuration).expect("lowers");
        for signature in wanted {
            assert!(
                text.contains(signature),
                "{identity:?}: {signature}\n{text}"
            );
        }
    }
}

/// A configuration claiming `compiler` with `options` on the command line.
fn with_options(
    compiler: crate::configuration::CompilerIdentity,
    options: &[&str],
) -> crate::configuration::CompilerConfiguration {
    let configuration =
        crate::configuration::CompilerConfiguration::default().with_compiler_identity(compiler);
    let mut code = configuration.code_options();
    code.x86_features = crate::target::X86Features::from_options(
        &options.iter().copied(),
        compiler != crate::configuration::CompilerIdentity::Gcc,
    );
    configuration.with_code_options(code)
}

/// With AVX, vectors of 32 bytes and the records each compiler lets wrap one
/// travel whole, as `gcc -mavx2 -S` and `clang -mavx2 -S` were measured to pass
/// them; the rest stay in memory. Without AVX nothing does.
#[test]
fn avx_lets_vectors_and_their_wrappers_travel_whole() {
    use crate::configuration::CompilerIdentity;
    let types = "typedef int V __attribute__((vector_size(32))); typedef double D                  __attribute__((vector_size(32))); typedef int W __attribute__((vector_size(16)));                  typedef int N __attribute__((vector_size(8))); typedef double L                  __attribute__((vector_size(8))); typedef __int128 T                  __attribute__((vector_size(32))); ";
    // The declarator of `x`, and whether GCC and Clang pass it whole.
    for (declaration, gcc, clang) in [
        ("V x", true, true),
        ("T x", false, false),
        ("struct { V v; } x", true, true),
        ("struct { struct { V v; } s; } x", true, true),
        ("struct { D v[1]; } x", true, true),
        ("struct { union { D v; } u; } x", true, true),
        ("struct { V v; int :0; } x", true, true),
        ("struct __attribute__((packed)) { V v; } x", true, true),
        ("struct { V v; int z[0]; } x", true, false),
        ("struct { V v; int z[]; } x", true, false),
        ("struct { V v; struct {} e; } x", true, false),
        ("struct { struct {} e; V v; } x", true, false),
        ("struct { W a; W b; } x", false, false),
        ("struct { double a[4]; } x", false, false),
        ("struct { V v; int i; } x", false, false),
        ("union { V v; float f; } x", true, true),
        ("union { D v; N n; } x", true, true),
        ("union { D v; W w; } x", true, true),
        ("union { __float128 q; D v; } x", true, true),
        ("union { D v; _Complex float c; } x", true, true),
        ("union { D v; L l; } x", false, false),
        ("union { D v; int i; } x", false, false),
        ("union { D v; double d[2]; } x", false, false),
        ("union { D v; _Complex double c; } x", false, false),
        ("union { D v; long double l; } x", false, false),
        ("union { D v; double d[4]; } x", false, true),
        ("union { W a[2]; D v; } x", false, true),
        ("union { V v; int a[8]; } x", false, true),
        ("union { D v; struct { double d[4]; } s; } x", false, false),
        ("union { char c[32]; float f[8]; } x", false, false),
    ] {
        for (compiler, wanted) in [
            (CompilerIdentity::Gcc, gcc),
            (CompilerIdentity::Clang, clang),
        ] {
            let source = format!("{types}void f({declaration}) {{ (void)&x; }}");
            let text = lower_with(&source, with_options(compiler, &["-mavx2"])).expect("lowers");
            assert_eq!(
                text.contains(", wide"),
                wanted,
                "{compiler:?} {declaration}\n{text}"
            );
            let text = lower_with(&source, with_options(compiler, &[])).expect("lowers");
            assert!(
                !text.contains(", wide"),
                "{compiler:?} {declaration}\n{text}"
            );
        }
    }
}

/// With AVX both compilers return 32 bytes in a register of their own, and
/// Clang 64 bytes in two; on Windows Clang's are the only ones measured.
#[test]
fn avx_vector_results_follow_each_compiler() {
    use crate::configuration::CompilerIdentity;
    let source = "typedef int V __attribute__((vector_size(32))); typedef int W \
                  __attribute__((vector_size(64))); V f(V *p) { return *p; } W g(W *p) { return \
                  *p; }";
    let result = |bytes: u32, wide: bool, align: u32| {
        format!(
            "ptr result(size {bytes}, align {align}, vector{})",
            if wide { ", wide" } else { "" }
        )
    };
    let gcc = lower_with(source, with_options(CompilerIdentity::Gcc, &["-mavx2"])).expect("lowers");
    assert!(gcc.contains(&result(32, true, 32)), "{gcc}");
    assert!(gcc.contains(&result(64, false, 64)), "{gcc}");
    let clang =
        lower_with(source, with_options(CompilerIdentity::Clang, &["-mavx2"])).expect("lowers");
    assert!(clang.contains(&result(32, true, 32)), "{clang}");
    assert!(clang.contains(&result(64, true, 64)), "{clang}");
}

/// What a `target` attribute or an option bcc only warns about would change
/// across objects is refused, and what stays among bcc's own functions isn't.
#[test]
fn foreign_vector_conventions_are_refused() {
    use crate::configuration::CompilerIdentity;
    let refused = |source: &str, compiler, options: &[&str], part: &str| {
        let errors = lower_with(
            &format!("typedef int V __attribute__((vector_size(32))); {source}"),
            with_options(compiler, options),
        )
        .expect_err(source);
        assert!(
            errors.iter().any(|error| error.contains(part)),
            "{errors:?}"
        );
    };
    let accepted = |source: &str, compiler, options: &[&str]| {
        let lowered = lower_with(
            &format!("typedef int V __attribute__((vector_size(32))); {source}"),
            with_options(compiler, options),
        );
        assert!(lowered.is_ok(), "{source}: {lowered:?}");
    };
    for compiler in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        let foreign = "ignores changes how it travels";
        let definition = "V f(V a) { return a; }";
        refused(definition, compiler, &["-mavx512f"], foreign);
        refused(definition, compiler, &["-march=haswell"], foreign);
        accepted(definition, compiler, &["-mavx512f", "-mno-avx"]);
        accepted(definition, compiler, &["-march=haswell", "-march=x86-64"]);
        let local = "static V f(V a) { return a; } V *g(V *p) { *p = f(*p); return p; }";
        accepted(local, compiler, &["-mavx512f"]);
        // An option outside the table leaves AVX on until AVX goes, or until
        // its own later opposite.
        for options in [
            &["-mavx512f", "-mno-avx512bw"][..],
            &["-mavx512bw", "-mno-avx512f"],
            &["-mavx512f", "-mavx512bw", "-mno-avx512f"],
            &["-mxop", "-mno-fma4"],
            &["-mavx512f", "-mno-avx2"],
            &["-mavx512f", "-mno-avx", "-mavx"],
            &["-mno-avx", "-mavx512f"],
        ] {
            refused(definition, compiler, options, foreign);
        }
        for options in [
            &["-mavx512f", "-mno-avx512f"][..],
            &["-mavx512f", "-mno-sse4.2"],
            &["-mavx256-split-unaligned-load"],
        ] {
            accepted(definition, compiler, options);
        }
        let target = "__attribute__((target(\"avx2\"))) V f(V a) { return a; }";
        refused(target, compiler, &[], foreign);
        accepted(target, compiler, &["-mavx2"]);
        let target = "__attribute__((target(\"no-avx\"))) V f(V a) { return a; }";
        refused(target, compiler, &["-mavx2"], foreign);
        accepted(target, compiler, &[]);
        let local = "__attribute__((target(\"avx2\"))) static V f(V a) { return a; } V *g(V *p) { \
                     *p = f(*p); return p; }";
        accepted(local, compiler, &[]);
        let call = "V e(V); __attribute__((target(\"avx2\"))) void g(V *p) { *p = e(*p); }";
        refused(call, compiler, &[], foreign);
        accepted(
            "__attribute__((target(\"avx2\"))) int f(int a) { return a; }",
            compiler,
            &[],
        );
    }
    let variadic = "int f(int n, V a, ...) { return a[n]; }";
    let named = "a named 32-byte vector parameter";
    refused(variadic, CompilerIdentity::Gcc, &["-mavx2"], named);
    accepted(variadic, CompilerIdentity::Clang, &["-mavx2"]);
    accepted(variadic, CompilerIdentity::Gcc, &[]);
    accepted(
        "static int f(int n, V a, ...) { return a[n]; } int g(V *p) { return f(0, *p); }",
        CompilerIdentity::Gcc,
        &["-mavx2"],
    );
    // XSAVE: AVX needs it in GCC only.
    let definition = "V f(V a) { return a; }";
    let foreign = "ignores changes how it travels";
    refused(
        definition,
        CompilerIdentity::Clang,
        &["-mavx512f", "-mno-xsave"],
        foreign,
    );
    accepted(
        definition,
        CompilerIdentity::Gcc,
        &["-mavx512f", "-mno-xsave"],
    );
    // A `target` string ends at a NUL, as GCC reads it.
    refused(
        "__attribute__((target(\"no-avx\\0junk\"))) V f(V a) { return a; }",
        CompilerIdentity::Gcc,
        &["-mavx2"],
        foreign,
    );
}

/// The vector builtins lowering does not expand are refused by name rather
/// than called.
#[test]
fn unexpanded_vector_builtins_are_refused() {
    let errors = lower(
        "typedef int V __attribute__((vector_size(16))); V f(V a, V b) { return \
         __builtin_elementwise_max(a, b); }",
    )
    .expect_err("refused");
    assert_eq!(errors, ["not yet supported by lowering: this builtin"]);
}
