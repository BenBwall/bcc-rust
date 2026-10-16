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
