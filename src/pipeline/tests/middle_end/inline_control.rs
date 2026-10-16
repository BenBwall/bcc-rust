//! GNU inline controls through lowering, the bcc optimizer, the interpreter
//! and LLVM.
//! GNU extension: GCC manual, "Common Function Attributes", noinline, noipa,
//! `always_inline`, and "Inline".

use super::*;
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

/// Runs `main` and returns its status, ignoring output.
fn interpret_main_only(module: &Module<'_>, target: Target) -> i32 {
    interpret(module, target).0
}
