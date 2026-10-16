//! Weak linkage, aliases, sections, retention, visibility, function
//! alignment, returns-twice functions and constructor/destructor lists in
//! the textual form and the verifier.

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
fn returns_twice_round_trips_and_follows_the_external_catalog() {
    let text = "\
function @save(ptr) -> i32 external align 16 returns_twice hidden

function @_setjmp(ptr) -> i32 external returns_twice

function @plain(ptr) -> i32 external
";
    assert_round_trip(text);
    let arena = Bump::new();
    // Unmarked text still declares glibc's `_setjmp` and `vfork` as the
    // external-symbol contract describes them, but not an internal
    // function of that name or `longjmp`.
    let module = parse(
        &arena,
        "function @_setjmp(ptr) -> i32 external
function @vfork() -> i32 weak
function          @setjmp(ptr) -> i32 internal {
block0(v0: ptr):
    v1 = iconst.i32 0
    return          v1
}
function @longjmp(ptr, i32) external
",
    );
    let returns_twice = |name| {
        let Some(Symbol::Function(func)) = module.symbol(name) else {
            panic!("{name}")
        };
        module.function(func).returns_twice
    };
    assert!(returns_twice("_setjmp") && returns_twice("vfork"));
    assert!(!returns_twice("setjmp") && !returns_twice("longjmp"));
    assert!(
        module
            .to_string()
            .contains("function @_setjmp(ptr) -> i32 external returns_twice\n")
    );
}

#[test]
fn effective_returns_twice_follows_aliases_and_escapes() {
    let arena = Bump::new();
    let text = "\
global @table internal size 8, align 8 = bytes \"0000000000000000\" relocs [0: @save]
function @real(ptr) -> i32 external returns_twice {
block0(v0: ptr):
    v1 = iconst.i32 0
    return v1
}
function @save(ptr) -> i32 external alias @real
function @plain() external
";
    let module = parse(&arena, text);
    let func = |name| {
        let Some(Symbol::Function(func)) = module.symbol(name) else {
            panic!("{name}")
        };
        func
    };
    assert!(module.returns_twice(func("real")) && module.returns_twice(func("save")));
    assert!(!module.function(func("save")).returns_twice);
    assert!(!module.returns_twice(func("plain")));
    assert!(module.returns_twice_escapes());
    let plain = parse(
        &arena,
        &text.replace("relocs [0: @save]", "relocs [0: @plain]"),
    );
    assert!(!plain.returns_twice_escapes());
    let address = parse(
        &arena,
        "function @vfork() -> i32 external\nfunction @f() -> ptr external {\nblock0:\n    v0 = \
         func_addr @vfork\n    return v0\n}\n",
    );
    assert!(address.returns_twice_escapes());
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
