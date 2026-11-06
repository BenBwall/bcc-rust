//! x86 `interrupt`, `no_caller_saved_registers`, GCC's `naked` and
//! `-mgeneral-regs-only` through the command line, compiled and run on the
//! host through LLVM and the native back end. A processor enters a handler
//! only from the kernel, but user code can push the frame itself and `iretq`
//! back to itself, so `iret.c` round-trips one: each register comes back.

#[cfg(test)]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        fs,
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    const GCC: &[&str] = &["--gcc-mode", "-mgeneral-regs-only"];
    const CONFIGURATIONS: [(&str, &str); 4] = [
        ("llvm", "-O0"),
        ("llvm", "-O2"),
        ("native", "-O0"),
        ("native", "-O2"),
    ];

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn scratch(name: &str) -> PathBuf {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "bcc-x86-interrupt-{}-{number}-{name}",
            std::process::id()
        ))
    }

    fn bcc(args: &[&str], source: &Path, output: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(args)
            .arg(source)
            .arg("-o")
            .arg(output)
            .env("NO_COLOR", "1")
            .output()
            .expect("the bcc-rust test binary must run")
    }

    /// Compiles `source` with `args` for each back end and level, runs it,
    /// and returns nothing if each exits with 0.
    #[track_caller]
    fn runs(source: &Path, identity: &[&str], native: &[&str]) {
        for (back_end, level) in CONFIGURATIONS {
            let mut args = identity.to_vec();
            if back_end == "native" {
                args.extend_from_slice(native);
            }
            args.push(level);
            let executable = scratch("program");
            let compiled = bcc(&args, source, &executable);
            assert!(compiled.status.success(), "{args:?}: {compiled:?}");
            let ran = Command::new(&executable).output().unwrap();
            fs::remove_file(&executable).unwrap();
            assert!(ran.status.success(), "{args:?}: {ran:?}");
        }
    }

    fn program(text: &str) -> PathBuf {
        let path = scratch("program.c");
        fs::write(&path, text).unwrap();
        path
    }

    fn round_trip() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/x86_interrupt/iret.c")
    }

    /// GCC's handlers save no SSE state, under `-mgeneral-regs-only`.
    #[test]
    fn handlers_preserve_general_registers_under_gcc() {
        runs(&round_trip(), GCC, &["-Xbcc", "--backend=native"]);
    }

    /// Clang's save whatever they and their callees may change, SSE included.
    #[test]
    fn handlers_preserve_every_register_under_clang() {
        runs(
            &round_trip(),
            &[
                "--compiler-identity=clang",
                "-DXMM_PRESERVED",
                "-Wno-excessive-regsave",
            ],
            &["--backend=native"],
        );
    }

    /// GCC's `pr25967-1.c` in small: a naked function reads what the caller
    /// pushed through a register variable, with C around it.
    #[test]
    fn a_naked_body_reads_the_entry_stack() {
        let source = program(
            r#"
extern void exit(int);
__attribute__((naked, used)) void fn(void) {
  register unsigned long *sp __asm__("sp");
  unsigned long error = *sp;
  unsigned long *frame = sp + 1;
  if (error != 0x12345670) __builtin_abort();
  if (frame[0] != 0x12345671) __builtin_abort();
  if (frame[1] != 0x12345672) __builtin_abort();
  exit(0);
}
int main(void) {
  __asm__("push $0x12345672; push $0x12345671; push $0x12345670; jmp fn");
  return 1;
}
"#,
        );
        runs(&source, GCC, &["-Xbcc", "--backend=native"]);
        fs::remove_file(source).unwrap();
    }

    /// An assembly body is all of a naked function's code.
    #[test]
    fn a_naked_assembly_body_returns_by_itself() {
        let source = program(
            r#"
__attribute__((naked)) int seven(void) { __asm__("movl $7, %eax\n\tret"); }
int main(void) { return seven() != 7; }
"#,
        );
        runs(&source, &["--gcc-mode"], &["-Xbcc", "--backend=native"]);
        fs::remove_file(source).unwrap();
    }

    /// `-muintr` gives `<x86gprintrin.h>`'s frame and `uiret`.
    #[test]
    fn user_interrupt_handlers_return_with_uiret() {
        let source = program(
            r#"
#include <x86gprintrin.h>
#ifndef __UINTR__
#error "no user interrupts"
#endif
void __attribute__((interrupt)) fn(struct __uintr_frame *frame, unsigned long vector) {
  frame->rip = vector;
}
"#,
        );
        for native in [&[][..], &["-Xbcc", "--backend=native"][..]] {
            let mut args = vec!["--gcc-mode", "-mgeneral-regs-only", "-muintr", "-S"];
            args.extend_from_slice(native);
            let listing = scratch("listing.s");
            let compiled = bcc(&args, &source, &listing);
            assert!(compiled.status.success(), "{compiled:?}");
            let text = fs::read_to_string(&listing).unwrap();
            fs::remove_file(&listing).unwrap();
            assert!(text.contains("uiret"), "{text}");
            assert!(!text.contains("iretq"), "{text}");
        }
        fs::remove_file(source).unwrap();
    }

    /// The error of compiling `text` under GCC's `-mgeneral-regs-only`, if
    /// any, which each back end must give alike.
    fn gcc_error(text: &str) -> Option<String> {
        let source = program(text);
        let mut errors = Vec::new();
        for native in [&[][..], &["-Xbcc", "--backend=native"][..]] {
            let mut args = GCC.to_vec();
            args.push("-c");
            args.extend_from_slice(native);
            let object = scratch("program.o");
            let compiled = bcc(&args, &source, &object);
            if object.exists() {
                fs::remove_file(&object).unwrap();
            }
            let stderr = String::from_utf8_lossy(&compiled.stderr).into_owned();
            errors.push(
                stderr
                    .lines()
                    .find_map(|line| line.split("error: ").nth(1).map(str::to_owned)),
            );
        }
        fs::remove_file(source).unwrap();
        assert_eq!(errors[0], errors[1], "{text}");
        errors.pop().flatten()
    }

    /// Each line of the table is GCC 14's verdict, as far as it refuses.
    #[test]
    fn floating_code_is_refused_as_gcc_refuses_it() {
        const SSE: &str = "SSE register return with SSE disabled";
        const X87: &str = "x87 register return with x87 disabled";
        for (source, error) in [
            ("double f(void) { return 1.0; }", SSE),
            ("float f(void) { return 1.0f; }", SSE),
            ("long double f(void) { return 1.0L; }", X87),
            ("extern double e(void); void f(void) { e(); }", SSE),
            ("void f(void) { volatile double x = 1.0; x = x + 1; }", SSE),
            (
                "void f(void) { volatile long double x = 1.0L; x = x + 1; }",
                X87,
            ),
            (
                "struct s { double d; }; struct s f(void) { struct s v = {0}; return v; }",
                SSE,
            ),
            (
                "typedef int v __attribute__((vector_size(16))); v f(v a) { return a; }",
                SSE,
            ),
            (
                "void f(__builtin_va_list a) { (void) __builtin_va_arg(a, double); }",
                "SSE register argument with SSE disabled",
            ),
        ] {
            assert_eq!(gcc_error(source).as_deref(), Some(error), "{source}");
        }
        for source in [
            "double global = 1.5;",
            "extern double e(void); void f(void) { (void) &e; }",
            "int f(void) { return sizeof(double); }",
            "struct s { int i; float f; }; struct s f(void) { struct s v = {0}; return v; }",
            "struct s { double d[4]; }; struct s f(void) { struct s v = {0}; return v; }",
            "void f(int n, ...) { __builtin_va_list a; __builtin_va_start(a, n); \
             __builtin_va_end(a); }",
            "typedef int v __attribute__((vector_size(16))); void f(void) { volatile v a = {1, 2, \
             3, 4}; a = a + a; }",
            "void f(long *d, long *s) { __builtin_memcpy(d, s, 256); }",
        ] {
            assert_eq!(gcc_error(source), None, "{source}");
        }
        // GCC compiles these with general registers or libgcc routines that
        // x86-64 lacks; bcc refuses them by name.
        for source in [
            "void f(double d) {}",
            "void f(void) { extern void g(double); g(1.0); }",
            "int f(float *p) { return *p > 0; }",
        ] {
            assert_eq!(
                gcc_error(source).as_deref(),
                Some(
                    "not yet supported by lowering: floating-point values under \
                     -mgeneral-regs-only"
                ),
                "{source}"
            );
        }
    }

    fn macros(args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(args)
            .args(["-E", "-dM", "-x", "c", "/dev/null"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// GCC 14's macros with and without the option.
    #[test]
    fn general_registers_change_gcc_macros() {
        let plain = macros(&["--gcc-mode"]);
        let general = macros(GCC);
        for name in [
            "__SSE__",
            "__SSE2__",
            "__SSE_MATH__",
            "__SSE2_MATH__",
            "__MMX__",
        ] {
            assert!(plain.contains(&format!("#define {name} ")), "{name}");
            assert!(!general.contains(&format!("#define {name} ")), "{name}");
        }
        assert!(general.contains("#define _SOFT_FLOAT 1\n"));
        assert!(general.contains("#define __GCC_IEC_559 0\n"));
        assert!(general.contains("#define __GCC_IEC_559_COMPLEX 0\n"));
        assert!(!plain.contains("__UINTR__"));
        assert!(macros(&["--gcc-mode", "-muintr"]).contains("#define __UINTR__ 1\n"));
        assert!(macros(&["--clang-mode", "-muintr"]).contains("#define __UINTR__ 1\n"));
        assert!(!macros(&["--gcc-mode", "-muintr", "-mno-uintr"]).contains("__UINTR__"));
    }

    /// Both compilers compile one, though nothing can pass the arguments.
    #[test]
    fn a_variadic_handler_is_refused_by_name() {
        assert_eq!(
            gcc_error("__attribute__((interrupt)) void h(void *p, ...) {}").as_deref(),
            Some(
                "not yet supported by lowering: an interrupt service routine with variable \
                 arguments"
            )
        );
    }

    /// The option is GCC's and System V's here.
    #[test]
    fn other_drivers_and_targets_refuse_general_registers() {
        let source = program("int x;\n");
        for args in [
            &["--clang-mode", "-mgeneral-regs-only", "-c"][..],
            &[
                "--gcc-mode",
                "--target=x86_64-w64-mingw32",
                "-mgeneral-regs-only",
                "-c",
            ][..],
        ] {
            let object = scratch("program.o");
            let compiled = bcc(args, &source, &object);
            assert!(!compiled.status.success(), "{args:?}: {compiled:?}");
            assert!(
                String::from_utf8_lossy(&compiled.stderr).contains("'-mgeneral-regs-only'"),
                "{args:?}: {compiled:?}"
            );
        }
        fs::remove_file(source).unwrap();
    }

    /// Only compiled code can enter a handler.
    #[test]
    fn the_interpreter_refuses_handlers_and_naked_functions_by_name() {
        for (text, name) in [
            (
                "void __attribute__((interrupt)) h(void *p) {}\nint main(void) { return 0; }\n",
                "interrupt service routine",
            ),
            (
                "__attribute__((naked)) void n(void) { __asm__(\"ret\"); }\nint main(void) { \
                 return 0; }\n",
                "naked function",
            ),
        ] {
            let source = program(text);
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--compiler-identity=gcc", "--interpret"])
                .arg(&source)
                .env("NO_COLOR", "1")
                .output()
                .unwrap();
            fs::remove_file(source).unwrap();
            assert!(!output.status.success(), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(name),
                "{output:?}"
            );
        }
    }
}
