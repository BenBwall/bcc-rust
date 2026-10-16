//! Weak linkage, aliases, sections, retention, visibility, function
//! alignment and constructor/destructor lists in the textual form and the
//! verifier.

use super::*;

const SYMBOLS: &str = "\
global @value external size 4, align 4 hidden section \".data.mine\" used = bytes \"05000000\"
global @other external size 4, align 4 alias @value
global @maybe weak size 4, align 4
global @tls external thread_local size 4, align 4 = zero
global @tls_alias external thread_local size 4, align 4 hidden alias @tls
global @tls_maybe weak thread_local size 4, align 4

constructor @init priority 101
destructor @fini priority 65535

function @init() internal align 16 used {
block0:
    return
}

function @fini() weak section \".text.fini\" {
block0:
    return
}

function @real(i32) -> i32 external {
block0(v0: i32):
    return v0
}

function @copy(i32) -> i32 weak protected alias @real

function @missing() -> i32 weak
";

#[test]
fn symbol_attributes_round_trip_and_verify() {
    assert_round_trip(SYMBOLS);
    assert_eq!(verify_text(SYMBOLS), Vec::<String>::new());
    let arena = Bump::new();
    let module = parse(&arena, SYMBOLS);
    let Some(Symbol::Function(copy)) = module.symbol("copy") else {
        panic!()
    };
    let Some(Symbol::Function(real)) = module.symbol("real") else {
        panic!()
    };
    assert_eq!(
        module.function(copy).attributes.alias,
        Some(Symbol::Function(real))
    );
    assert_eq!(module.function(copy).linkage, Linkage::Weak);
    assert_eq!(module.init_functions().len(), 2);
    assert_eq!(module.init_functions()[0].kind, InitKind::Constructor);
}

#[test]
fn verifier_rejects_broken_aliases_sections_and_init_functions() {
    let rule = |item: &str, text: &str| format!("@{item}: invalid symbol attribute: {text}");
    let cases: &[(&str, &[String])] = &[
        (
            "global @g external size 4, align 4 = zero\nfunction @f() external alias @g\n",
            &[rule(
                "f",
                "an alias and its target are both functions or both globals",
            )],
        ),
        (
            "function @f() external\nfunction @a() external alias @f\n",
            &[rule(
                "a",
                "an alias target is a definition that is not itself an alias",
            )],
        ),
        (
            "global @g external size 4, align 4 = zero\nglobal @a external size 4, align 4 alias \
             @g = zero\n",
            &[rule("a", "an alias has no body or initializer of its own")],
        ),
        (
            "global @g external size 4, align 4 = zero\nglobal @a external size 4, align 4 alias \
             @g\nglobal @b external size 4, align 4 alias @a\n",
            &[rule(
                "b",
                "an alias target is a definition that is not itself an alias",
            )],
        ),
        (
            "global @g internal size 4, align 4 hidden = zero\nfunction @f() internal protected \
             {\nblock0:\n    return\n}\n",
            &[
                rule("g", "an internal symbol has default visibility"),
                rule("f", "an internal symbol has default visibility"),
            ],
        ),
        (
            "global @g external size 4, align 4 section \"\" = zero\n",
            &[rule(
                "g",
                "a section name is nonempty and has no quote, newline or NUL",
            )],
        ),
        (
            "global @g external thread_local size 4, align 4 = zero\nglobal @a external size 4, \
             align 4 alias @g\nglobal @h external size 4, align 4 = zero\nglobal @b external \
             thread_local size 4, align 4 alias @h\n",
            &[
                rule("a", "an alias is thread-local exactly when its target is"),
                rule("b", "an alias is thread-local exactly when its target is"),
            ],
        ),
        (
            "function @f(i32) external {\nblock0(v0: i32):\n    return\n}\nconstructor @f \
             priority 1\n",
            &[rule(
                "f",
                "a constructor or destructor is a defined function without parameters",
            )],
        ),
        (
            "function @f() external\ndestructor @f priority 1\n",
            &[rule(
                "f",
                "a constructor or destructor is a defined function without parameters",
            )],
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(&verify_text(text), expected, "{text}");
    }
}

#[test]
fn priorities_beyond_sixteen_bits_are_parse_errors() {
    let arena = Bump::new();
    let text = "function @f() external {\nblock0:\n    return\n}\nconstructor @f priority 65536\n";
    _ = parse_module(&arena, text).unwrap_err();
}
