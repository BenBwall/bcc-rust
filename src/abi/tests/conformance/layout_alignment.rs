//! Storage alignment and natural ABI alignment at independently compiled
//! boundaries.
use super::*;

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
