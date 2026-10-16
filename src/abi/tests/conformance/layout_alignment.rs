//! Storage alignment and natural ABI alignment at independently compiled
//! boundaries.
use super::*;

#[test]
fn pragma_packed_struct_abi_exchange_matches_clang() {
    let decl = "#pragma pack(push, boundary, 1)\nstruct S { char c; int x; };\n#pragma pack(pop, \
                boundary)\n";
    let body = "struct S b_echo(struct S s) { s.x++; s.c++; return s; }";
    let bcc = format!(
        "{decl}{body} struct S c_echo(struct S); int b_call(void) {{ struct S s={{7,41}}; struct \
         S r=c_echo(s); return r.c!=8 || r.x!=42; }}"
    );
    let clang = format!(
        "{decl}{} struct S b_echo(struct S); int b_call(void); int main(void) {{ struct S \
         s={{7,41}}; struct S r=b_echo(s); return b_call() || r.c!=8 || r.x!=42; }}",
        body.replace("b_echo", "c_echo")
    );
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for optimized in [false, true] {
            let tu = Bump::new();
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, &bcc, target);
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
            }
            lower_abi(&mut module, target, &Bump::new()).unwrap();
            assert_conforms("pragma-pack-exchange", target, &module, &clang);
        }
    }
}

#[test]
fn reduced_typedef_alignment_uses_memory_for_sysv() {
    let decl = "typedef int I __attribute__((aligned(1))); struct S { char c; I x; };";
    let body = "struct S b_echo(struct S s) { s.x++; return s; }";
    let bcc = format!(
        "{decl}{body} struct S c_echo(struct S); int b_call(void) {{ struct S s={{7,41}}; struct \
         S r=c_echo(s); return r.c!=7 || r.x!=42; }}"
    );
    let clang = format!(
        "{decl}{} struct S b_echo(struct S); int b_call(void); int main(void) {{ struct S \
         s={{7,41}}; struct S r=b_echo(s); return b_call() || r.c!=7 || r.x!=42; }}",
        body.replace("b_echo", "c_echo")
    );
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let reference = clang_ir("reduced-typedef-alignment", &bcc, target);
        let signature = reference
            .lines()
            .find(|line| line.starts_with("define ") && line.contains("@b_echo("))
            .unwrap();
        if target == Target::LinuxGnu {
            assert!(
                signature.contains("byval(%struct.S) align 8"),
                "{reference}"
            );
        } else {
            assert!(signature.contains("i64 @b_echo(i64"), "{reference}");
        }
        for optimized in [false, true] {
            let tu = Bump::new();
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, &bcc, target);
            assert_eq!(
                module.to_string().contains("unaligned"),
                target == Target::LinuxGnu,
                "{module}"
            );
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
            }
            lower_abi(&mut module, target, &Bump::new()).unwrap();
            let mut llvm = String::new();
            emit_module(&module, target, &mut llvm).unwrap();
            if target == Target::LinuxGnu {
                assert!(llvm.contains("byval([5 x i8]) align 8"), "{llvm}");
            }
            assert_conforms("reduced-typedef-alignment", target, &module, &clang);
        }
    }
}

#[test]
fn packed_aggregate_places_keep_alignment_through_conversions() {
    let decl = "struct S { int x,y; }; struct __attribute__((packed)) P { char c; struct S s; }; \
                struct W { struct S s; };";
    let body = "int b_take(struct S s) { return s.x!=17 || s.y!=23; } struct S b_return(struct P \
                *p) { return p->s; }";
    let calls = "struct P p={7,{17,23}}, q={0,{0,0}}; struct S a=p.s, b; b=p.s; struct W w={p.s}; \
                 struct W *c=&(struct W){p.s}; q.s=a; struct S r=c_return(&p); return a.x!=17 || \
                 b.y!=23 || w.s.x!=17 || c->s.y!=23 || q.s.x!=17 || r.y!=23 || c_take(p.s);";
    let bcc = format!(
        "{decl}{body} int c_take(struct S); struct S c_return(struct P *); int \
         b_call(void){{{calls}}}"
    );
    let clang = format!(
        "{decl}{} int b_call(void); int b_take(struct S); struct S b_return(struct P *); int \
         main(void) {{ struct P p={{7,{{17,23}}}}; struct S r=b_return(&p); return b_call() || \
         r.x!=17 || r.y!=23 || b_take(p.s); }}",
        body.replace("b_take", "c_take")
            .replace("b_return", "c_return")
    );
    let interpreted = format!("{bcc}{}", clang.strip_prefix(decl).unwrap());
    check_alignment_execution("packed-aggregate-conversions", &bcc, &clang, &interpreted);
}

#[test]
fn packed_union_cast_stores_use_temporary_alignment() {
    let decl = "typedef long long I __attribute__((aligned(64))); union __attribute__((packed)) U \
                { I x; }; union __attribute__((packed)) V { long long x; }; struct S { int x,y; \
                }; union __attribute__((packed)) W { struct S s; };";
    let body = "int b_cast(void) { I x=42; union U u=(union U)x; union V v=(union V)43LL; struct \
                S s={17,23}; union W w=(union W)s; return u.x!=42 || v.x!=43 || w.s.x!=17 || \
                w.s.y!=23; }";
    let bcc = format!("{decl}{body}");
    let clang = format!(
        "{decl}{} int b_cast(void); int main(void){{ return b_cast() || c_cast(); }}",
        body.replace("b_cast", "c_cast")
    );
    let interpreted = format!("{bcc}{}", clang.strip_prefix(decl).unwrap());
    check_alignment_execution("packed-union-cast", &bcc, &clang, &interpreted);
}

fn check_alignment_execution(name: &str, bcc: &str, clang: &str, source: &str) {
    use crate::backend::interpreter::{
        DefaultHost,
        RuntimeValue,
        Termination,
        run,
    };
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let reference = clang_ir(name, source, target);
        assert!(reference.contains("align 1"), "{reference}");
        for optimized in [false, true] {
            for profile in [Profile::PreAbi, Profile::PostAbi] {
                let tu = Bump::new();
                let arena = Bump::new();
                let mut module = lower_c(&tu, &arena, source, target);
                if optimized {
                    _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
                }
                if profile == Profile::PostAbi {
                    lower_abi(&mut module, target, &Bump::new()).unwrap();
                }
                assert_verified(&module, profile, &module.to_string());
                let execution = Bump::new();
                let mut host = DefaultHost::for_target(&execution, target);
                let result = run(&module, "main", &[], &mut host);
                assert_eq!(
                    result
                        .unwrap_or_else(|error| panic!(
                            "{name} {target:?} {optimized} {profile:?}: {error:?}\n{module}"
                        ))
                        .termination,
                    Termination::Returned(Some(RuntimeValue::Int(0)))
                );
            }
            let tu = Bump::new();
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, bcc, target);
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
            }
            lower_abi(&mut module, target, &Bump::new()).unwrap();
            assert_conforms(name, target, &module, clang);
        }
    }
}

fn clang_ir(name: &str, source: &str, target: Target) -> String {
    let toolchain = Toolchain::bundled(Target::LinuxGnu).expect("bundled Clang");
    let directory = TempDir::new(name);
    let path = directory.join("source.c");
    std::fs::write(&path, source).unwrap();
    let output = crate::test_support::native::run_native(
        Command::new("python")
            .arg("scripts/cargo_runner.py")
            .arg(toolchain.clang())
            .args(["-O0", "-S", "-emit-llvm", "-o", "-"])
            .arg(format!("--target={}", target.triple()))
            .arg(&path),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Array variables of at least 16 bytes for the System V large-array rule
/// (#69), with Clang 23's GNU array alignment and MSVC size-based global
/// minima. Automatic MSVC arrays retain their declared alignment.
const LARGE_ARRAYS: &str = "\
char g16[16]; char g15[15]; char gi[4][4]; struct M { char a[32]; } gm;
char ga[32] __attribute__((aligned(2))); _Alignas(4) char gb[32];
typedef char C1 __attribute__((aligned(1))); C1 gt[64];
extern char ext[16]; char *use_ext(void) { return ext; }
_Static_assert(_Alignof(char[32]) == 1, \"the C type keeps its alignment\");
void use(char *);
void f(int n) {
  char l16[16]; char l15[15]; static char ls[16]; char vla[n];
  char lx[32] __attribute__((aligned(2)));
  use(l16); use(l15); use(ls); use(vla); use(lx);
}
";

#[test]
fn large_array_variables_get_the_psabi_alignment() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let tu = Bump::new();
        let arena = Bump::new();
        let module = lower_c(&tu, &arena, LARGE_ARRAYS, target);
        let text = module.to_string();
        let gnu = target != Target::WindowsMsvc;
        let large = if gnu { 16 } else { 1 };
        let global_large = if gnu { 16 } else { 8 };
        let global_other = if gnu { 1 } else { 8 };
        let global = |name: &str| {
            let line = text
                .lines()
                .find(|line| line.starts_with(&format!("global @{name} ")))
                .unwrap_or_else(|| panic!("no @{name}\n{text}"));
            line[line.find(", align ").unwrap() + 8..]
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap()
                .parse::<u64>()
                .unwrap()
        };
        for (name, align) in [
            ("g16", global_large),
            ("g15", global_other),
            ("gi", global_large),
            ("gm", global_other),
            ("ga", 2),
            ("gb", 4),
            ("gt", 16),
            ("ext", global_large),
            ("f.ls", global_large),
        ] {
            assert_eq!(global(name), align, "@{name} for {target:?}\n{text}");
        }
        // `l16`, `l15`, `lx`, then the VLA.
        let body = &text[text.find("function @f(").unwrap()..];
        let slots: Vec<&str> = body
            .lines()
            .filter(|line| line.contains("stack_slot") || line.contains("dyn_alloca"))
            .collect();
        let expected = [
            format!("stack_slot 16, align {large}"),
            "stack_slot 15, align 1".to_string(),
            "stack_slot 32, align 2".to_string(),
            format!("align {large}"),
        ];
        for expected in &expected {
            assert!(
                slots.iter().any(|line| line.ends_with(expected.as_str())),
                "{expected} for {target:?}: {slots:?}\n{text}"
            );
        }
    }
}

/// Clang's code may assume the large-array alignment of an array variable
/// bcc defines: zeroing it with aligned vector stores faults on a
/// misaligned object. The bcc side checks its own arrays' addresses too.
#[test]
fn large_array_variables_link_with_clang_code_that_assumes_their_alignment() {
    let bcc = "char pad1 = 1; char g[32] = { 1 }; char pad2 = 2; char h[17];
static char s[48] = { 3 }; char *s_address(void) { return s; }
static int misaligned(const void *p) { return (int)((unsigned long)p % 16); }
int b_check(void) {
  char local[16]; char tail[24]; char *volatile keep = local; (void)keep;
  return misaligned(g) || misaligned(h) || misaligned(s) || misaligned(local)
      || misaligned(tail);
}";
    let clang = "extern char g[32], h[17]; char *s_address(void); int b_check(void);
__attribute__((noinline)) void c_clear(void) {
  __builtin_memset(g, 0, 32); __builtin_memset(h, 0, 16);
}
int main(void) {
  if (b_check()) return 1;
  c_clear();
  return g[0] || g[31] || s_address()[0] != 3;
}";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for optimized in [false, true] {
            let tu = Bump::new();
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, bcc, target);
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
            }
            lower_abi(&mut module, target, &Bump::new()).unwrap();
            assert_conforms("large-arrays", target, &module, clang);
            assert_native_conforms("large-arrays-native", target, &module, clang);
        }
    }
}
