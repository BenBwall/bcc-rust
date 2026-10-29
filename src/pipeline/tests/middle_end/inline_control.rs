//! GNU inline controls and the x86/Microsoft timing and breakpoint
//! intrinsics through lowering, the bcc optimizer, the interpreter and LLVM.
//! GNU extension: GCC manual, "Common Function Attributes", noinline, noipa,
//! `always_inline`, and "Inline".

use super::*;
use crate::configuration::{
    CStandard,
    ExtensionPolicy,
};

/// Whether `module` keeps a direct call to `callee` in `main`.
fn calls(module: &Module<'_>, callee: &str) -> bool {
    let text = module.to_string();
    let main = text
        .split("function @main(")
        .nth(1)
        .expect("main is defined");
    let main = main.split("\nfunction ").next().unwrap_or(main);
    main.contains(&format!("call @{callee}("))
}

/// `noinline` and `noipa` keep their call under the bcc optimizer, which
/// otherwise expands the small leaf (GCC's torture `frame-address.c`
/// depends on it).
#[test]
fn noinline_and_noipa_keep_their_calls() {
    for (attribute, inlined) in [("", true), ("noinline", false), ("noipa", false)] {
        let source = format!(
            "__attribute__(({attribute})) static int f(int x) {{ return x + 1; }} int main(void) \
             {{ return f(41) - 42; }}"
        );
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            with_module(&source, target, true, Profile::PostAbi, |module, target| {
                assert_eq!(calls(module, "f"), !inlined, "{attribute}\n{module}");
                assert_eq!(interpret(module, target).0, 0);
                if !inlined {
                    let mut llvm = String::new();
                    crate::backend::llvm::emit_module(module, target, &mut llvm).unwrap();
                    assert!(llvm.contains("noinline"), "{llvm}");
                }
            });
        }
    }
}

/// MinGW-w64's `FORCEINLINE` `intsafe.h` functions are `extern inline`
/// `always_inline` GNU bodies that no library defines. bcc keeps each body
/// as a private function that direct calls reach and the inliner expands,
/// while a body with inline assembly keeps calling the external definition.
#[test]
fn always_inline_gnu_bodies_reach_their_calls() {
    let source = "extern __inline__ __attribute__((__always_inline__, __gnu_inline__)) int \
                  add(int a, int b) { return a + b; } int (*address)(int, int) = add; int \
                  main(void) { return add(40, 2) - 42; }";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                assert_eq!(interpret_main_only(module, target), 0, "{module}");
                // The external symbol stays a declaration, for the address.
                let Some(crate::ir::Symbol::Function(add)) = module.symbol("add") else {
                    panic!("add is declared\n{module}");
                };
                assert!(module.function(add).body.is_none());
                assert!(!calls(module, "add"), "{module}");
                if optimize {
                    assert!(!calls(module, "add.always_inline"), "{module}");
                }
            });
        }
    }
    let assembly = "extern __inline__ __attribute__((__always_inline__, __gnu_inline__)) int \
                    get(void) { __asm__(\"nop\"); return 1; } int main(void) { return get(); }";
    with_module(
        assembly,
        Target::WindowsGnu,
        false,
        Profile::PostAbi,
        |module, _| {
            assert!(calls(module, "get"), "{module}");
            assert!(module.symbol("get.always_inline").is_none(), "{module}");
        },
    );
}

/// glibc's `__REDIRECT` names an inline function's external symbol from
/// another entity with an assembler label; a call through that entity runs
/// the external definition, never the private body, which would otherwise
/// recurse into itself.
/// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125. GNU extension: GCC manual,
/// "Asm Labels".
#[test]
fn redirected_symbols_keep_their_external_definition() {
    let source = "int twice(int x) __asm__(\"wrapped\"); int twice(int x) { return 2 * x; } \
                  extern __inline__ __attribute__((__always_inline__, __gnu_inline__)) int \
                  wrapped(int x) { return twice(x) + 1; } int main(void) { return wrapped(20) - \
                  41; }";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                assert_eq!(interpret_main_only(module, target), 0, "{module}");
            });
        }
    }
}

/// Inline-only definitions keep their entity's returns-twice contract, so
/// `always_inline` cannot bypass the optimizer's frame safeguards, and the
/// native back end still calls the private body as one that returns twice.
#[test]
fn private_inline_bodies_keep_returns_twice_contracts() {
    let source = "extern inline __attribute__((gnu_inline, always_inline, returns_twice)) int \
                  save(void) { return 7; } int main(void) { return save(); }";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, _| {
                let Some(crate::ir::Symbol::Function(private)) =
                    module.symbol("save.always_inline")
                else {
                    panic!("private body is defined\n{module}");
                };
                assert!(module.returns_twice(private), "{module}");
                assert!(calls(module, "save.always_inline"), "{module}");
                if profile == Profile::PostAbi {
                    let mut assembly = String::new();
                    crate::backend::native::emit_assembly(module, target, &mut assembly)
                        .unwrap_or_else(|error| panic!("{error}\n{module}"));
                    assert!(assembly.contains("callq\tsave.always_inline"), "{assembly}");
                }
            });
        }
    }
}

/// The private definition receives the same hidden object-size parameters
/// as the declaration that selected it at a direct call.
#[test]
fn private_inline_bodies_keep_hidden_object_sizes() {
    let source = "extern inline __attribute__((gnu_inline, always_inline)) unsigned long long \
                  size(const void *const p __attribute__((pass_object_size(0)))) { return \
                  __builtin_object_size(p, 0); } int main(void) { char buffer[11]; return \
                  size(buffer) != 11; }";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                assert_eq!(interpret(module, target).0, 0, "{module}");
            });
        }
    }
}

/// A private `always_inline` body is emitted with what it references:
/// another private body, a static helper and static data, and the cleanup
/// function of a variable, which compiler-generated calls reach as direct
/// calls do. No external `outer`, `inner` or `drop` is left to the linker.
#[test]
fn private_inline_bodies_bring_their_dependencies() {
    let source = "static int base = 40; static int helper(int x) { return x + base; } extern \
                  __inline__ __attribute__((__gnu_inline__, __always_inline__)) int inner(int x) \
                  { return helper(x) + 1; } extern __inline__ __attribute__((__gnu_inline__, \
                  __always_inline__)) int outer(int x) { return inner(x); } static int dropped; \
                  extern __inline__ __attribute__((__gnu_inline__, __always_inline__)) void \
                  drop(int *p) { dropped += *p; } int main(void) { { int x \
                  __attribute__((cleanup(drop))) = 2; } return outer(1) + dropped - 44; }";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                assert_eq!(interpret(module, target).0, 0, "{module}");
                let text = module.to_string();
                for external in ["call @inner(", "call @outer(", "call @drop("] {
                    assert!(!text.contains(external), "{external}\n{module}");
                }
                if profile == Profile::PostAbi {
                    let mut llvm = String::new();
                    crate::backend::llvm::emit_module(module, target, &mut llvm).unwrap();
                    for symbol in ["@inner(", "@outer(", "@drop("] {
                        assert!(
                            !llvm
                                .lines()
                                .any(|line| line.starts_with("declare") && line.contains(symbol)),
                            "{symbol}\n{llvm}"
                        );
                    }
                }
            });
        }
    }
}

/// `__rdtsc` reads an increasing counter in every mode; `__debugbreak`
/// is Microsoft's breakpoint, `llvm.debugtrap` as Clang emits it.
#[test]
fn timestamp_and_breakpoint_intrinsics_lower() {
    let source =
        "int main(void) { unsigned long long a = __rdtsc(), b = __rdtsc(); return b > a ? 0 : 1; }";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let configuration =
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow).with_target(target);
        for (optimize, profile) in STAGES {
            with_configured_module(
                source,
                target,
                optimize,
                profile,
                configuration,
                |module, target| {
                    assert_eq!(interpret(module, target).0, 0, "{module}");
                    if profile == Profile::PostAbi {
                        let mut llvm = String::new();
                        crate::backend::llvm::emit_module(module, target, &mut llvm).unwrap();
                        assert!(llvm.contains("@llvm.x86.rdtsc()"), "{llvm}");
                    }
                },
            );
        }
    }
    with_configured_module(
        "void stop(void) { __debugbreak(); __annotation(L\"TlgWrite\", L\"probe\"); } int \
         main(void) { return 0; }",
        Target::WindowsMsvc,
        false,
        Profile::PostAbi,
        CompilerConfiguration::default()
            .with_target(Target::WindowsMsvc)
            .with_msvc_extensions(true),
        |module, target| {
            assert!(module.to_string().contains("debug_trap"), "{module}");
            let mut llvm = String::new();
            crate::backend::llvm::emit_module(module, target, &mut llvm).unwrap();
            assert!(llvm.contains("call void @llvm.debugtrap()"), "{llvm}");
        },
    );
}

/// Microsoft's `__va_start(&ap, last)`, through which `<vadefs.h>` defines
/// `va_start` on x64, starts the list `ap` points to, and stays the
/// intrinsic, as `__debugbreak` does, when a header declares it with a
/// compatible type, as the MSVC headers do. An incompatible declaration
/// names an ordinary function, and without Microsoft extensions the name
/// is ordinary too.
#[test]
fn declared_microsoft_intrinsics_stay_intrinsics() {
    let source = r"typedef char *va_list;
void __cdecl __va_start(va_list *, ...);
void __cdecl __debugbreak(void);
int sum(int n, ...) {
    va_list ap;
    ((void)(__va_start(&ap, n)));
    int s = 0;
    for (int i = 0; i < n; i++) s += *(int *)((ap += 8) - 8);
    ap = 0;
    return s;
}
void stop(void) { __debugbreak(); }
int main(void) { return sum(3, 1, 2, 30) - 33; }
";
    let configuration = CompilerConfiguration::default()
        .with_target(Target::WindowsMsvc)
        .with_msvc_extensions(true);
    // `<vadefs.h>`'s `va_arg` reads the list as Win64 lays it out, which
    // the interpreter models only after ABI lowering.
    for optimize in [false, true] {
        with_configured_module(
            source,
            Target::WindowsMsvc,
            optimize,
            Profile::PostAbi,
            configuration,
            |module, target| {
                assert_eq!(interpret(module, target).0, 0, "{module}");
                let text = module.to_string();
                assert!(!text.contains("@__va_start"), "{text}");
                assert!(!text.contains("@__debugbreak"), "{text}");
            },
        );
    }
    let incompatible = "int __va_start(int);\nint f(void) { return __va_start(1); }\n";
    for configuration in [configuration, configuration.with_msvc_extensions(false)] {
        with_configured_module(
            incompatible,
            Target::WindowsMsvc,
            false,
            Profile::PostAbi,
            configuration,
            |module, _| assert!(calls_from(module, "f", "__va_start"), "{module}"),
        );
    }
}

/// Whether `function` in `module` calls `callee` directly.
fn calls_from(module: &Module<'_>, function: &str, callee: &str) -> bool {
    let text = module.to_string();
    text.split(&format!("function @{function}("))
        .nth(1)
        .and_then(|body| body.split("\nfunction ").next())
        .is_some_and(|body| body.contains(&format!("call @{callee}(")))
}

/// Runs `main` and returns its status, ignoring output.
fn interpret_main_only(module: &Module<'_>, target: Target) -> i32 {
    interpret(module, target).0
}
