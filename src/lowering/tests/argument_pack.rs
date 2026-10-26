//! GNU `__builtin_va_arg_pack ()` and `__builtin_va_arg_pack_len ()`: each
//! direct call of a function using them calls a copy specialized for its
//! anonymous arguments, and every other use is refused as GCC refuses it.
//! GNU extension: GCC manual, "Constructing Calls".
use super::*;

/// glibc's GCC `printf` and `open` wrappers (`bits/stdio2.h`,
/// `bits/fcntl2.h`), with their library functions declared as glibc does.
const WRAPPERS: &str = r#"
#if !__has_builtin(__builtin_va_arg_pack) || !__has_builtin(__builtin_va_arg_pack_len)
#error the argument pack builtins are missing
#endif
extern int printf(const char *, ...);
extern int __printf_chk(int, const char *, ...);
extern __inline __attribute__((__always_inline__, __gnu_inline__, __artificial__)) int
printf(const char *fmt, ...) { return __printf_chk(1, fmt, __builtin_va_arg_pack()); }
extern int __open_2(const char *, int);
extern int __open_alias(const char *, int, ...) __asm__("open");
extern void __open_too_many_args(void) __attribute__((__error__("too many")));
extern __inline __attribute__((__always_inline__, __gnu_inline__, __artificial__)) int
open(const char *path, int flags, ...) {
    if (__builtin_va_arg_pack_len() > 1) __open_too_many_args();
    if (__builtin_va_arg_pack_len() < 1) return __open_2(path, flags);
    return __open_alias(path, flags, __builtin_va_arg_pack());
}
"#;

/// The body of the function `name` in `text`, from its header line to the
/// line closing it.
fn body<'t>(text: &'t str, name: &str) -> &'t str {
    let start = text
        .find(&format!("function @{name}("))
        .unwrap_or_else(|| panic!("no function @{name}:\n{text}"));
    let end = text[start..]
        .find("\n}")
        .map_or(text.len(), |end| start + end + 2);
    &text[start..end]
}

#[test]
fn each_call_gets_a_copy_for_its_anonymous_argument_types() {
    let text = lowered(&format!(
        "{WRAPPERS}
int main(void) {{
    printf(\"%d %s\\n\", 1, \"a\");
    printf(\"%d %s\\n\", 2, \"b\");
    printf(\"%f\\n\", 1.5f);
    printf(\"plain\\n\");
    return open(\"f\", 0) + open(\"f\", 0101, 0600);
}}"
    ));
    let main = body(&text, "main");
    // Calls with the same promoted argument types share a copy, numbered in
    // the order of their first calls; `float` is passed as `double`.
    for (copy, calls) in [
        ("printf.va_arg_pack(", 2),
        ("printf.va_arg_pack.1(", 1),
        ("printf.va_arg_pack.2(", 1),
        ("open.va_arg_pack(", 1),
        ("open.va_arg_pack.1(", 1),
    ] {
        assert_eq!(main.matches(copy).count(), calls, "{copy}\n{main}");
    }
    assert!(
        !main.contains("call @printf(") && !main.contains("call @open("),
        "{main}"
    );
    for (copy, signature) in [
        (
            "printf.va_arg_pack",
            "(ptr noundef, i32 noundef, ptr noundef) -> i32 internal alwaysinline",
        ),
        (
            "printf.va_arg_pack.1",
            "(ptr noundef, f64 noundef) -> i32 internal alwaysinline",
        ),
        (
            "printf.va_arg_pack.2",
            "(ptr noundef) -> i32 internal alwaysinline",
        ),
    ] {
        let copy = body(&text, copy);
        assert!(copy.contains(signature), "{copy}");
        assert_eq!(copy.matches("call @__printf_chk(").count(), 1, "{copy}");
    }
    // The pack follows the call's own arguments, in order.
    let copy = body(&text, "printf.va_arg_pack");
    assert!(
        copy.contains("call @__printf_chk(v3, v0, v1, v2)"),
        "{copy}"
    );
    // The length is a constant in each copy, and the branches on it are
    // folded without the optimizer: no call of the `error` function stays.
    let two = body(&text, "open.va_arg_pack");
    assert!(
        two.contains("call @__open_2(") && !two.contains("call @open("),
        "{two}"
    );
    let three = body(&text, "open.va_arg_pack.1");
    assert!(
        three.contains("call @open(v0, v1, v2)") && !three.contains("__open_2"),
        "{three}"
    );
    assert!(!text.contains("call @__open_too_many_args"), "{text}");
    // The inline-only wrappers themselves have no body and no private copy,
    // and nothing calls the external `printf`.
    assert!(!text.contains(".always_inline"), "{text}");
    assert!(!text.contains("function @printf("), "{text}");
}

#[test]
fn copies_pass_packs_on_to_copies_and_share_recursive_ones() {
    let text = lowered(
        "struct S { int a[5]; };
int g(int, ...);
static inline __attribute__((always_inline)) int inner(int n, ...) {
    return g(n, __builtin_va_arg_pack()) + __builtin_va_arg_pack_len();
}
static inline __attribute__((always_inline)) int outer(int n, ...) {
    return inner(n + 1, (__builtin_va_arg_pack()));
}
static inline int again(int n, ...) { return n ? again(n - 1, __builtin_va_arg_pack()) : 0; }
int main(void) { struct S s = {{1}}; return outer(1, s, 2.0) + again(3, 'x'); }",
    );
    let outer = body(&text, "outer.va_arg_pack");
    assert!(outer.contains("call @inner.va_arg_pack("), "{outer}");
    // The structure is copied again for the call that receives it.
    assert!(outer.contains("copy "), "{outer}");
    let inner = body(&text, "inner.va_arg_pack");
    assert!(inner.contains("iconst.i32 2"), "{inner}");
    // A recursive call with the same pack calls its own copy.
    let again = body(&text, "again.va_arg_pack");
    assert!(again.contains("call @again.va_arg_pack("), "{again}");
    assert!(
        !text.contains("function @outer(") && !text.contains("function @again("),
        "{text}"
    );
}

#[test]
fn unresolvable_packs_are_errors() {
    let invalid_pack = "invalid use of '__builtin_va_arg_pack ()'";
    let invalid_length = "invalid use of '__builtin_va_arg_pack_len ()'";
    for (source, message) in [
        // An external definition is emitted with its pack unresolved.
        (
            "int g(int, ...); int f(int a, ...) { return g(a, __builtin_va_arg_pack()); }",
            invalid_pack,
        ),
        (
            "int f(int a, ...) { return __builtin_va_arg_pack_len(); }",
            invalid_length,
        ),
        // The pack must be the last argument, after the named parameters.
        (
            "int g(int, ...); static inline __attribute__((always_inline)) int f(int a, ...) { \
             return g(__builtin_va_arg_pack(), a); } int main(void) { return f(1, 2); }",
            invalid_pack,
        ),
        (
            "int h(int, int); static inline __attribute__((always_inline)) int f(int a, ...) { \
             return h(a, __builtin_va_arg_pack()); } int main(void) { return f(1, 2); }",
            invalid_pack,
        ),
        (
            "static inline __attribute__((always_inline)) int f(int a, ...) { return \
             __builtin_va_arg_pack(); } int main(void) { return f(1, 2); }",
            invalid_pack,
        ),
        // Only direct calls reach the copies of an internal function.
        (
            "int g(int, ...); static inline int f(int a, ...) { return g(a, \
             __builtin_va_arg_pack()); } int (*p)(int, ...) = f;",
            invalid_pack,
        ),
        // A pack that grows with each expansion never ends.
        (
            "static inline int f(int a, ...) { return f(a, a, __builtin_va_arg_pack()); } int \
             main(void) { return f(1); }",
            invalid_pack,
        ),
    ] {
        let errors = lower(source).expect_err(source);
        assert!(
            errors.iter().all(|error| error == message),
            "{source}: {errors:?}"
        );
    }
}
