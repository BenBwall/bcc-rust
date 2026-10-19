//! Inline-copy resolution for the interpreter and the native back end
//! (#109): LLVM's always-inliner walk, then external calls.
//! C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.

use super::*;

/// Resolves the copies of `text` and returns the module text.
fn resolve(text: &str) -> String {
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse(&arena, text);
    assert_verifies(&module, "the input");
    resolve_inline_copies(&mut module, &scratch);
    assert_verifies(&module, "the output");
    module.to_string()
}

/// The printed function `name`, header to closing brace.
fn function<'t>(text: &'t str, name: &str) -> &'t str {
    let start = text
        .find(&format!("function @{name}("))
        .unwrap_or_else(|| panic!("@{name} is printed\n{text}"));
    let rest = &text[start..];
    rest.find("\n\n").map_or(rest, |end| &rest[..end])
}

/// A copy of `name` that calls the function `callee` and returns `x * callee(x
/// - 1)`.
fn copy(name: &str, callee: &str, control: &str) -> String {
    format!(
        "\
function @{name}.copy(i32) -> i32 internal {control} inline_copy_of @{name} {{
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = isub.i32 v0, v1
    v3 = call @{callee}(v2)
    v4 = imul.i32 v0, v3
    return v4
}}

function @{name}(i32) -> i32 external {control}

"
    )
}

const USER: &str = "\
function @u(i32) -> i32 external {
block0(v0: i32):
    v1 = call @f.copy(v0)
    return v1
}
";

/// A recursive inline body is not inline viable: its caller calls the
/// external definition, as LLVM O0 and Clang O0 do (104, not 24).
#[test]
fn a_recursive_always_inline_copy_calls_the_external_function() {
    let text = resolve(&format!("{}{USER}", copy("f", "f.copy", "alwaysinline")));
    assert!(function(&text, "u").contains("call @f("), "{text}");
    assert!(!function(&text, "f.copy").contains('{'), "{text}");
}

/// With `f` first in module order, `f` is expanded into `u` and `g`, and
/// then `g` calls itself, so `u` calls the external `g` (Clang's 812).
/// With `g` first, `g` is expanded into `f`, and `u` calls the external `f`.
#[test]
fn mutual_recursion_follows_llvm_module_order() {
    let f_first = resolve(&format!(
        "{}{}{USER}",
        copy("f", "g.copy", "alwaysinline"),
        copy("g", "f.copy", "alwaysinline")
    ));
    let u = function(&f_first, "u");
    assert!(
        u.contains("call @g(") && !u.contains("call @f"),
        "{f_first}"
    );
    let g_first = resolve(&format!(
        "{}{}{USER}",
        copy("g", "f.copy", "alwaysinline"),
        copy("f", "g.copy", "alwaysinline")
    ));
    let u = function(&g_first, "u");
    assert!(
        u.contains("call @f(") && !u.contains("call @g"),
        "{g_first}"
    );
}

/// A body that calls neither itself nor its external function is expanded
/// at every call, so no call of the external definition remains.
#[test]
fn a_viable_always_inline_copy_is_expanded() {
    let text = resolve(&format!(
        "{}function @h(i32) -> i32 external\n\n{USER}",
        copy("f", "h", "alwaysinline")
    ));
    let u = function(&text, "u");
    assert!(u.contains("call @h(") && !u.contains("call @f"), "{text}");
}

/// A plain inline copy is only the LLVM back end's `available_externally`
/// body: nothing expands it, and the module loses its body.
#[test]
fn a_plain_inline_copy_is_dropped() {
    let text = resolve(&format!(
        "{}function @h(i32) -> i32 external\n\n{USER}",
        copy("f", "h", "")
    ));
    assert!(function(&text, "u").contains("call @f("), "{text}");
    assert!(!function(&text, "f.copy").contains('{'), "{text}");
}

/// LLVM expands a weak `always_inline` definition, although the linker may
/// choose a strong definition for calls that remain.
#[test]
fn a_weak_always_inline_definition_is_expanded() {
    let text = resolve(
        "\
function @f(i32) -> i32 weak alwaysinline {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = iadd.i32 v0, v1
    return v2
}

function @u(i32) -> i32 external {
block0(v0: i32):
    v1 = call @f(v0)
    return v1
}
",
    );
    assert!(!function(&text, "u").contains("call"), "{text}");
}
