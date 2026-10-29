//! Calling-convention and assembler-name extensions at the lowering boundary.
use super::*;
use crate::backend::interpreter::{
    RuntimeValue,
    Termination,
    run_text,
};

#[test]
fn generated_globals_skip_all_user_symbol_names() {
    for source in [
        "char *p=\"X\"; int a __asm__(\".str\")=42; int main(void){return a;}",
        "int a __asm__(\".str\")=42; char *p=\"X\"; int main(void){return a;}",
        "char *p=\"X\"; int main(void){static int a __asm__(\".str\")=42; return a;}",
        "int main(void){{static int x=1;} static int a __asm__(\"main.x\")=42; return a;}",
        "int main(void){static int a __asm__(\"main.x\")=42; {static int x=1;} return a;}",
        "int main(void){char *p=\"X\"; static int a __asm__(\".str\")=42; return a;}",
        "extern int a __asm__(\".str\"); char *p=\"X\"; int a=42; int main(void){return a;}",
        "char *p=\"X\"; int a __asm__(\".str\")=42; int b __asm__(\".str.1\")=1; int \
         main(void){return a;}",
    ] {
        let text = lowered(source);
        assert_eq!(
            run_text(&text, "main").result.unwrap().termination,
            Termination::Returned(Some(RuntimeValue::Int(42))),
            "{source}\n{text}"
        );
    }
}

/// The header line of `@name` in the module text, or "" if it has none.
fn header<'t>(text: &'t str, name: &str) -> &'t str {
    let prefix = format!("function @{name}(");
    text.lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_default()
}

#[test]
fn overloaded_aliases_emit_both_bodies_and_windows_exports() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let windows = matches!(target, Target::WindowsGnu | Target::WindowsMsvc);
        for export in [false, true] {
            let annotation = if export && windows {
                "__declspec(dllexport)"
            } else {
                ""
            };
            let (integer, real) = if target == Target::WindowsMsvc {
                ("?f@@$$J0YAHH@Z", "?f@@$$J0YANN@Z")
            } else {
                ("_Z1fi", "_Z1fd")
            };
            let source = format!(
                "int ti(int n) {{return n+3;}} double td(double n) {{return n+5;}} {annotation} \
                 int f(int) __attribute__((overloadable,alias(\"ti\"))); {annotation} double \
                 f(double) __attribute__((overloadable,alias(\"td\"))); int main(void) {{return \
                 f(1)+(int)f(2.0);}}"
            );
            let text = lower_with(
                &source,
                crate::configuration::CompilerConfiguration::default()
                    .with_target(target)
                    .with_msvc_extensions(true),
            )
            .unwrap_or_else(|errors| panic!("{target:?} export={export}: {errors:?}\n{source}"));
            let arena = Bump::new();
            let module = parse_module(&arena, &text).unwrap();
            for (name, target_name) in [(integer, "ti"), (real, "td")] {
                let Some(crate::ir::Symbol::Function(id)) = module.symbol(name) else {
                    panic!("missing {name}: {text}");
                };
                let function = module.function(id);
                assert_eq!(
                    function.attributes.alias,
                    module.symbol(target_name),
                    "{text}"
                );
                assert_eq!(
                    function.attributes.dll,
                    if export && windows {
                        crate::ir::DllStorage::Export
                    } else {
                        crate::ir::DllStorage::Default
                    },
                    "{text}"
                );
            }
            assert_eq!(
                run_text(&text, "main").result.unwrap().termination,
                Termination::Returned(Some(RuntimeValue::Int(11))),
                "{text}"
            );
        }
    }
}

#[test]
fn aliases_to_overloads_use_emitted_symbols_and_keep_internal_bodies() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for internal in [false, true] {
            let name = match (target == Target::WindowsMsvc, internal) {
                | (true, true) => "?f@@YAHH@Z",
                | (true, false) => "?f@@$$J0YAHH@Z",
                | (false, true) => "_ZL1fi",
                | (false, false) => "_Z1fi",
            };
            let source = format!(
                "{} int f(int n) __attribute__((overloadable)) {{return n+4;}} int a(int) \
                 __attribute__((alias(\"{name}\"))); int main(void) {{return a(7);}}",
                if internal { "static" } else { "" }
            );
            let text = lower_for(&source, target).unwrap();
            let arena = Bump::new();
            let module = parse_module(&arena, &text).unwrap();
            let Some(crate::ir::Symbol::Function(alias)) = module.symbol("a") else {
                panic!("{text}");
            };
            assert_eq!(
                module.function(alias).attributes.alias,
                module.symbol(name),
                "{text}"
            );
            assert_eq!(
                run_text(&text, "main").result.unwrap().termination,
                Termination::Returned(Some(RuntimeValue::Int(11))),
                "{text}"
            );
        }
    }
}

#[test]
fn returns_twice_marks_declarations_from_the_retained_fact() {
    // MinGW's <setjmp.h> declares `_setjmp` and friends with the attribute;
    // glibc declares `_setjmp` without it, and the external catalog
    // supplies the same fact. C99 §7.13.2.1p4.
    let attributed = "int __attribute__((__nothrow__, __returns_twice__)) save(void *, void *);";
    let glibc = "typedef struct { long b[25]; } jmp_buf[1]; extern int _setjmp(jmp_buf); extern \
                 void longjmp(jmp_buf, int) __attribute__((__noreturn__));";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let source = format!("{attributed} int main(void) {{ return save(0, 0); }}");
        let text = lower_for(&source, target).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        assert!(
            header(&text, "save").ends_with(" external returns_twice"),
            "{text}"
        );
        let source = format!(
            "{glibc} int main(void) {{ jmp_buf b; volatile int n = 0;\n if (_setjmp(b) == 0) {{ n \
             = 5; longjmp(b, 3); }}\n return n; }}"
        );
        let text = lower_for(&source, target).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        assert!(
            header(&text, "_setjmp").ends_with(" external returns_twice"),
            "{text}"
        );
        assert!(header(&text, "longjmp").ends_with(" external"), "{text}");
        assert_eq!(
            run_text(&text, "main").result.unwrap().termination,
            Termination::Returned(Some(RuntimeValue::Int(5))),
            "{text}"
        );
    }
    // A local function of a cataloged spelling is another entity, but an
    // assembler label naming the symbol is the symbol.
    let source = "static int vfork(void) { return 1; } int main(void) { return vfork(); }";
    let text = lower_for(source, Target::LinuxGnu).unwrap();
    assert!(!text.contains("returns_twice"), "{text}");
    let source = "int save(void *) __asm__(\"_setjmp\"); int main(void) { return save(0); }";
    let text = lower_for(source, Target::LinuxGnu).unwrap();
    assert!(
        header(&text, "_setjmp").ends_with(" external returns_twice"),
        "{text}"
    );
}

/// The MSVC runtime's x64 `_setjmp` and `_setjmpex`, which its
/// `<setjmp.h>` declares with the buffer alone, also take the caller's
/// frame, as `cl` passes it and Clang passes `llvm.frameaddress(0)`; the
/// same declaration for MinGW-w64, whose header passes the frame itself,
/// keeps one parameter. C99 §7.13.1.1.
#[test]
fn msvc_setjmp_takes_the_callers_frame() {
    let source = "typedef struct { unsigned long long p[2]; } F; typedef F jmp_buf[16];\nint \
                  _setjmp(jmp_buf); int _setjmpex(jmp_buf);\nvoid longjmp(jmp_buf, int) \
                  __attribute__((noreturn));\njmp_buf b, c;\nint main(void) { volatile int n = \
                  1;\nif (_setjmp(b) == 0) { n = 5; longjmp(b, 3); }\nreturn n + _setjmpex(c); }";
    for (target, parameters) in [
        (Target::WindowsMsvc, "(ptr noundef, ptr noundef)"),
        (Target::WindowsGnu, "(ptr noundef)"),
    ] {
        let text = lower_for(source, target).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        for name in ["_setjmp", "_setjmpex"] {
            assert_eq!(
                header(&text, name),
                format!("function @{name}{parameters} -> i32 external returns_twice"),
                "{text}"
            );
        }
        assert_eq!(
            text.matches("frame_address").count(),
            if target == Target::WindowsMsvc { 2 } else { 0 },
            "{text}"
        );
        assert_eq!(
            run_text(&text, "main").result.unwrap().termination,
            Termination::Returned(Some(RuntimeValue::Int(5))),
            "{text}"
        );
    }
}

#[test]
fn nondefault_calling_conventions_are_refused() {
    for (target, attribute) in [
        (Target::LinuxGnu, "ms_abi"),
        (Target::WindowsGnu, "sysv_abi"),
        (Target::WindowsMsvc, "sysv_abi"),
        (Target::LinuxGnu, "vectorcall"),
        (Target::WindowsGnu, "preserve_most"),
        (Target::LinuxGnu, "regcall"),
    ] {
        for source in [
            format!("__attribute__(({attribute})) int f(int x) {{ return x; }}"),
            format!("extern int f(int) __attribute__(({attribute}));"),
            format!(
                "extern int f(int) __attribute__(({attribute})); int main(void) {{return f(42);}}"
            ),
            format!(
                "typedef int (__attribute__(({attribute})) *F)(int); int main(void) {{ F p=0; \
                 return p(42); }}"
            ),
            format!(
                "int main(void) {{ return ((int (__attribute__(({attribute})) *)(int))0)(42); }}"
            ),
        ] {
            let errors = lower_for(&source, target).expect_err(&source);
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("not yet supported by lowering: calling conventions")),
                "{target:?}: {source}: {errors:?}"
            );
        }
    }
}

#[test]
fn default_or_ignored_calling_conventions_are_accepted() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let default = if matches!(target, Target::LinuxGnu | Target::LinuxMusl) {
            "sysv_abi"
        } else {
            "ms_abi"
        };
        for attribute in [
            default,
            "stdcall",
            "cdecl",
            "fastcall",
            "thiscall",
            "regparm(3)",
        ] {
            let source = format!(
                "__attribute__(({attribute})) int f(int x) {{return x;}} int main(void) {{return \
                 f(42);}}"
            );
            let text = lower_for(&source, target).unwrap();
            assert_eq!(
                run_text(&text, "main").result.unwrap().termination,
                Termination::Returned(Some(RuntimeValue::Int(42)))
            );
        }
    }
    // Attribute arguments with convention spellings do not change the ABI.
    drop(
        lower("int f(void) __attribute__((annotate(\"ms_abi\"))); int main(void){return 0;}")
            .unwrap(),
    );
}

#[test]
fn calling_convention_keywords_are_checked_in_types_and_declarations() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let configuration = crate::configuration::CompilerConfiguration::default()
            .with_target(target)
            .with_msvc_extensions(true);
        for source in [
            "int __vectorcall f(int x){return x;}",
            "extern int __vectorcall f(int);",
            "typedef int (__vectorcall *F)(int); int main(void){F p=0; return p(42);}",
        ] {
            let errors = lower_with(source, configuration).expect_err(source);
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("not yet supported by lowering: calling conventions")),
                "{source}: {errors:?}"
            );
        }
        for keyword in ["__stdcall", "__cdecl", "__fastcall", "__thiscall"] {
            let source =
                format!("int {keyword} f(int x){{return x;}} int main(void){{return f(42);}}");
            assert!(lower_with(&source, configuration).is_ok(), "{source}");
        }
    }
}

#[test]
fn asm_labels_name_functions_and_objects() {
    for source in [
        "extern int f(void) __asm__(\"g\"); int g(void){return 42;} int main(void){return f();}",
        "int f(void) __asm__(\"g\"); int f(void){return 42;} int main(void){return f();}",
        "extern int a __asm__(\"b\"); int b=42; int main(void){return a;}",
        "extern int a[] __asm__(\"b\"); int *p=a; int b[1]={42}; int main(void){return p[0];}",
        "int a __asm__(\"b\")=42; int main(void){return a;}",
        "int main(void){static int a __asm__(\"b\")=42; return a;}",
        "int f(void); int f(void) __asm__(\"g\"); int f(void){return 42;} int (*p)(void)=f; int \
         main(void){return p();}",
    ] {
        for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
            let text = lower_for(source, target).unwrap();
            assert_eq!(
                run_text(&text, "main").result.unwrap().termination,
                Termination::Returned(Some(RuntimeValue::Int(42))),
                "{source}\n{text}"
            );
        }
    }
}

#[test]
fn nul_identifiers_cannot_reach_lowering() {
    for identifier in [r"f\u0000x", r"\u0000", r"f\U00000000x"] {
        drop(lower(&format!("int {identifier}(void) {{return 0;}}")).unwrap_err());
    }
}

#[test]
fn unsupported_asm_labels_are_refused() {
    for source in [
        "int f(void) __asm__(\"g\\0tail\"); int main(void){return f();}",
        "int f(void) __asm__(\"\"); int main(void){return f();}",
        "int main(void){int a __asm__(\"b\")=42; return a;}",
        "int f(void) __asm__(\"g\"); int f(void){return 1;} int g(void){return 2;}",
        "int b=1; int main(void){static int a __asm__(\"b\")=42; return a;}",
        "int a __asm__(\"b\")=1; int b=2; int main(void){return a;}",
        "int a __asm__(\"b\")=1; int b(void){return 42;}",
    ] {
        let errors = lower(source).expect_err(source);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("not yet supported by lowering: assembler labels")),
            "{source}: {errors:?}"
        );
    }
}

/// GCC's `__builtin_setjmp` stores the frame address and the stack position
/// in words 0 and 2 before `sjlj_setjmp`, as Clang does, and
/// `__builtin_longjmp` ends its block; the interpreter runs the pair. GNU
/// extension: GCC manual, "Nonlocal Gotos".
#[test]
fn builtin_setjmp_saves_the_frame_and_stack_before_the_jump_point() {
    let source = "static void *b[5];\nstatic void jump(void) { __builtin_longjmp(b, 1); }\nint \
                  main(void) { volatile int n = 0;\n if (__builtin_setjmp(b) == 0) { n = 4; \
                  jump(); }\n return n + __builtin_setjmp(b); }";
    let text = lowered(source);
    pretty_assertions::assert_eq!(
        functions(&text),
        "\
global @b internal size 40, align 16 = zero

function @jump() internal {
block0:
    v0 = global_addr @b
    v1 = iconst.i32 1
    sjlj_longjmp v0
    unreachable
}

function @main() -> i32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = iconst.i32 0
    v1 = stack_addr slot0
    store.i32 volatile v0, v1, align 4
    v2 = global_addr @b
    v3 = frame_address
    store.ptr v3, v2, align 16
    v4 = stack_save
    v5 = iconst.i64 16
    v6 = ptr_add inbounds v2, v5
    store.ptr v4, v6, align 16
    v7 = sjlj_setjmp v2
    v8 = iconst.i32 0
    v9 = icmp.i32 eq v7, v8
    brif v9, block1, block2
block1:
    v10 = stack_addr slot0
    v11 = iconst.i32 4
    store.i32 volatile v11, v10, align 4
    call @jump()
    jump block2
block2:
    v12 = stack_addr slot0
    v13 = load.i32 volatile v12, align 4
    v14 = global_addr @b
    v15 = frame_address
    store.ptr v15, v14, align 16
    v16 = stack_save
    v17 = iconst.i64 16
    v18 = ptr_add inbounds v14, v17
    store.ptr v16, v18, align 16
    v19 = sjlj_setjmp v14
    v20 = iadd.i32 nsw v13, v19
    return v20
}
"
    );
    assert_eq!(
        run_text(&text, "main").result.unwrap().termination,
        Termination::Returned(Some(RuntimeValue::Int(4))),
        "{text}"
    );
}
