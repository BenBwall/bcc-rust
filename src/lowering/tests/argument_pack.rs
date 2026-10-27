//! GNU `__builtin_va_arg_pack ()` and `__builtin_va_arg_pack_len ()`: each
//! direct call of a function using them calls a copy specialized for its
//! anonymous arguments, and every other use is refused as GCC refuses it.
//! GNU extension: GCC manual, "Constructing Calls".
use std::fmt::Write as _;

use super::*;
use crate::backend::interpreter::{
    RuntimeValue,
    Termination,
    run_text,
};

/// The IR of `source`, after checking that its `main` returns `expected`
/// in the interpreter.
#[track_caller]
fn returns(source: &str, expected: u128) -> String {
    let text = lowered(source);
    assert_eq!(
        run_text(&text, "main").result.unwrap().termination,
        Termination::Returned(Some(RuntimeValue::Int(expected))),
        "{source}\n{text}"
    );
    text
}

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

/// A later definition replaces a GNU `extern inline` body, as in GCC, so
/// calls reach it and not copies of the replaced body.
#[test]
fn a_replacing_definition_wins_over_a_pack_using_inline_body() {
    let text = returns(
        "extern inline __attribute__((gnu_inline, always_inline))
int f(int n, ...) { return __builtin_va_arg_pack_len(); }
int f(int n, ...) { return 42; }
int main(void) { return f(0, 1, 2); }",
        42,
    );
    assert!(!text.contains("va_arg_pack"), "{text}");
}

/// A copy requested only from a branch its caller's pack length rules out
/// is never lowered: its calls of undefined functions are not emitted, and
/// the copies it would request do not count toward the nesting limit.
#[test]
fn copies_requested_from_folded_branches_are_not_lowered() {
    let text = returns(
        "extern int missing(int, ...);
static inline __attribute__((always_inline)) int g(int n, ...)
{ return missing(n, __builtin_va_arg_pack()); }
static inline __attribute__((always_inline)) int f(int n, ...)
{ if (__builtin_va_arg_pack_len() < 1) return g(n, __builtin_va_arg_pack()); return 3; }
int main(void) { return f(0, 1); }",
        3,
    );
    assert!(!text.contains("call @missing"), "{text}");
    assert!(
        text.contains(
            "function @g.va_arg_pack(i32 noundef, i32 noundef) -> i32 internal alwaysinline\n"
        ),
        "{text}"
    );
    let text = returns(
        "static inline __attribute__((always_inline)) int f(int n, ...)
{ if (__builtin_va_arg_pack_len() < 1) return f(n, 1, __builtin_va_arg_pack()); return 7; }
int main(void) { return f(0, 1); }",
        7,
    );
    assert!(
        text.contains(
            "function @f.va_arg_pack.1(i32 noundef, i32 noundef, i32 noundef) -> i32 internal \
             alwaysinline\n"
        ),
        "{text}"
    );
}

/// GCC finds the pack through what it folds away: `__extension__`, a cast
/// or unary `+` to `int`, and a `_Generic` or `__builtin_choose_expr`
/// selection; and `(*f)`, `(&f)` and a cast of `f` to a pointer to its own
/// type call a function using the pack as `f` does.
#[test]
fn transparent_packs_and_designators_reach_copies() {
    let text = returns(
        "int g(int n, ...) { __builtin_va_list ap; __builtin_va_start(ap, n);
 int v = __builtin_va_arg(ap, int); __builtin_va_end(ap); return n * v; }
static inline __attribute__((always_inline)) int a(int n, ...)
{ return g(n, __extension__ __builtin_va_arg_pack()); }
static inline __attribute__((always_inline)) int b(int n, ...)
{ return g(n, (const int)(__builtin_va_arg_pack())); }
static inline __attribute__((always_inline)) int c(int n, ...)
{ return g(n, _Generic(0, int: __builtin_va_arg_pack(), default: 0)); }
static inline __attribute__((always_inline)) int d(int n, ...)
{ return g(n, __builtin_choose_expr(1, __builtin_va_arg_pack(), 0)); }
static inline __attribute__((always_inline)) int e(int n, ...)
{ return g(n + __builtin_va_arg_pack_len(), +__builtin_va_arg_pack()); }
typedef int variadic(int, ...);
int main(void) { return a(1, 2) + b(1, 3) + c(1, 4) + d(1, 5) + (*e)(0, 6) + (&e)(1, 7) + (**e)(2, \
         1) + ((variadic *)e)(0, 2) + ((variadic *)&e)(0, 3); }",
        2 + 3 + 4 + 5 + 6 + 14 + 3 + 2 + 3,
    );
    // Every call reaches a copy, none an indirect call of `e`.
    assert!(
        !text.contains("call_indirect") && !text.contains("func_addr @e"),
        "{text}"
    );
}

/// Only live code expands copies: an internal helper whose only call a
/// copy's folded branch drops is not lowered, nor are the copies it calls,
/// so neither an ever deeper pack nor an undefined callee is reached, as in
/// GCC at -O2. The nesting limit counts the shallowest live call, whatever
/// call requested the copy first.
#[test]
fn only_live_code_expands_copies() {
    let helper = |g: &str| {
        format!(
            "#define A static inline __attribute__((always_inline)) int
extern int missing(int, ...);
A g(int n, ...) {{ return {g}; }}
static int helper(void) {{ return g(0, 1); }}
A f(int n, ...) {{ if (__builtin_va_arg_pack_len() < 1) return helper(); return 3; }}
int main(void) {{ return f(0, 1); }}"
        )
    };
    let text = returns(&helper("g(n, 1, __builtin_va_arg_pack())"), 3);
    assert!(!text.contains("call @g.va_arg_pack"), "{text}");
    let text = returns(&helper("missing(n, __builtin_va_arg_pack())"), 3);
    assert!(!text.contains("call @missing"), "{text}");
    assert!(
        text.contains("function @helper() -> i32 internal\n"),
        "an unreached helper keeps no body:\n{text}"
    );
    // `c64` is 64 copies deep and calls `g` only when it has no anonymous
    // arguments; `shortpath` calls it directly. The dead deep request must
    // not count, whichever call comes first.
    let mut chain = String::from(
        "#define A static inline __attribute__((always_inline)) int
A g(int n, ...) { return n + __builtin_va_arg_pack_len(); }
A c64(int n, ...) { if (__builtin_va_arg_pack_len() < 1) return g(n, __builtin_va_arg_pack()); \
         return 7; }
",
    );
    for i in (1..64).rev() {
        writeln!(
            chain,
            "A c{i}(int n, ...) {{ return c{}(n, __builtin_va_arg_pack()); }}",
            i + 1
        )
        .unwrap();
    }
    chain.push_str("A shortpath(int n, ...) { return g(n, __builtin_va_arg_pack()); }\n");
    for main in [
        "int main(void) { return shortpath(0, 1) + c1(0, 1); }",
        "int main(void) { return c1(0, 1) + shortpath(0, 1); }",
    ] {
        let text = returns(&format!("{chain}{main}"), 8);
        assert!(text.contains("call @c1.va_arg_pack("), "{text}");
    }
}

/// Liveness roots every emitted symbol: an internal alias keeps its target's
/// body, and a `used` function that names the pack only in an unselected
/// generic association gets its own body. Depths relax to a fixed point,
/// so a copy reached first through a deep path and later through a
/// shallow one (via an ordinary helper) is checked on the shallow one.
#[test]
fn liveness_roots_aliases_and_used_functions_and_relaxes_depths() {
    let text = returns(
        "static int f(void) { return 42; }
static int a(void) __attribute__((alias(\"f\")));
static inline __attribute__((always_inline)) int p(int n, ...) { return \
         __builtin_va_arg_pack_len(); }
int main(void) { return p(0, 1); }",
        1,
    );
    assert!(
        text.contains("iconst.i32 42"),
        "the alias target keeps its body:\n{text}"
    );
    for selection in [
        "_Generic(1.0, int: __builtin_va_arg_pack_len(), default: 42)",
        "__builtin_choose_expr(0, __builtin_va_arg_pack_len(), 42)",
    ] {
        let text = lowered(&format!(
            "static __attribute__((used)) int f(void) {{ return {selection}; }}
int main(void) {{ return 0; }}"
        ));
        let used = body(&text, "f");
        assert!(used.contains("iconst.i32 42"), "{selection}:\n{text}");
    }
    let attribute = "static inline __attribute__((always_inline)) int";
    for c63 in [
        "c64(n, __builtin_va_arg_pack()) + g(n, __builtin_va_arg_pack())",
        "g(n, __builtin_va_arg_pack()) + c64(n, __builtin_va_arg_pack())",
    ] {
        let mut source = format!(
            "{attribute} h(int n, ...) {{ return __builtin_va_arg_pack_len(); }}
{attribute} g(int n, ...) {{ return h(n, __builtin_va_arg_pack()); }}
static int helper(void) {{ return g(0, 1); }}
{attribute} c64(int n, ...) {{ return helper() + __builtin_va_arg_pack_len(); }}
{attribute} c63(int n, ...) {{ return {c63}; }}
"
        );
        for i in (1..63).rev() {
            writeln!(
                source,
                "{attribute} c{i}(int n, ...) {{ return c{}(n, __builtin_va_arg_pack()); }}",
                i + 1
            )
            .unwrap();
        }
        source.push_str("int main(void) { return c1(0, 1); }");
        let text = returns(&source, 3);
        assert!(text.contains("call @h.va_arg_pack("), "{c63}:\n{text}");
    }
}

/// A function that names the pack only where it is not evaluated (`sizeof`,
/// unselected generic associations) is ordinary, and its address may be
/// taken. One whose address live code takes gets its own body, as in GCC,
/// which refuses a pack it evaluates where it is; a dead address does not.
#[test]
fn addresses_of_pack_functions_need_live_bodies() {
    let text = returns(
        "static int f(void) { return sizeof(__builtin_va_arg_pack()); }
static int h(void) { return _Generic(1.0, int: __builtin_va_arg_pack(), default: 2)
 + __builtin_choose_expr(0, __builtin_va_arg_pack_len(), 0); }
int (*p)(void) = f;
int (*q)(void) = h;
int main(void) { return p() + q() + f(); }",
        10,
    );
    // `f` does not use the pack, so its direct call needs no copy.
    assert!(!text.contains("va_arg_pack"), "{text}");
    let text = returns(
        "#define A static inline __attribute__((always_inline)) int
A g(int n, ...) { return __builtin_va_arg_pack_len(); }
A f(int n, ...) { if (__builtin_va_arg_pack_len() > 0) { int (*p)(int, ...) = g; return p(0, 1); } \
         return 7; }
int main(void) { return f(0); }",
        7,
    );
    assert!(!text.contains("func_addr @g"), "{text}");
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
        // A live address emits an internal function's own body.
        (
            "int g(int, ...); static inline int f(int a, ...) { return g(a, \
             __builtin_va_arg_pack()); } int (*p)(int, ...) = f;",
            invalid_pack,
        ),
        (
            "static inline int f(int a, ...) { return __builtin_va_arg_pack_len(); } int \
             main(void) { int (*p)(int, ...) = f; return p(1); }",
            invalid_length,
        ),
        // Deviations from GCC, which folds these away: arithmetic on the
        // pack and a comma before it are not transparent.
        (
            "int g(int, ...); static inline __attribute__((always_inline)) int f(int a, ...) { \
             return g(a, 0 + __builtin_va_arg_pack()); } int main(void) { return f(1, 2); }",
            invalid_pack,
        ),
        (
            "int g(int, ...); static inline __attribute__((always_inline)) int f(int a, ...) { \
             return g(a, ((void)0, __builtin_va_arg_pack())); } int main(void) { return f(1, 2); }",
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
