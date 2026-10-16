//! Complex scalars cross separately compiled C boundaries in both directions.
use super::*;

#[test]
fn complex_scalars_cross_both_directions() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for real in ["float", "double", "long double"] {
            let declarations = format!("typedef {real} REAL; typedef {real} _Complex Z;\n");
            let bcc = format!("{declarations}{BCC}");
            let clang = format!("{declarations}{CLANG}");
            for optimized in [false, true] {
                let arena = Bump::new();
                let tu = Bump::new();
                let scratch = Bump::new();
                let mut module = lower_c(&tu, &arena, &bcc, target);
                if optimized {
                    _ = optimize(&mut module, &OptimizerOptions::default(), &scratch);
                }
                lower_abi(&mut module, target, &scratch).unwrap();
                assert_verified(&module, Profile::PostAbi, &module.to_string());
                // Emission is checked even where the host cannot execute the
                // target.
                let mut llvm = String::new();
                emit_module(&module, target, &mut llvm).unwrap();
                assert_conforms(
                    &format!("complex-{}-{optimized}", real.replace(' ', "-")),
                    target,
                    &module,
                    &clang,
                );
                if real != "long double" || target == Target::WindowsMsvc {
                    assert_native_conforms(target, &module, &clang);
                }
            }
        }
    }
}

fn assert_native_conforms(target: Target, module: &Module<'_>, c: &str) {
    let Some(runner) = Runner::for_target(target) else {
        return;
    };
    let toolchain = match Toolchain::bundled(target) {
        | Ok(toolchain) => toolchain,
        | Err(error) => {
            eprintln!("skipping complex native for {target:?}: {error}");
            return;
        },
    };
    let directory = TempDir::new("complex-native-conformance");
    let object = directory.join("bcc.o");
    let c_path = directory.join("clang.c");
    let c = if runner == Runner::Wsl {
        format!("{c}{FREESTANDING}")
    } else {
        c.to_owned()
    };
    std::fs::write(&c_path, c).unwrap();
    let mut assembly = String::new();
    crate::backend::native::emit_assembly(module, target, &mut assembly).unwrap();
    crate::backend::native::assemble(&assembly, &object, target, OutputKind::Object).unwrap();
    for opt in [OptLevel::O0, OptLevel::O2] {
        let executable = directory.join(format!("complex-{opt:?}.exe"));
        let mut command = toolchain.command(
            &object,
            &executable,
            LlvmOptions {
                code: crate::target::CodeOptions::default(),
                opt: Some(opt),
                disable_llvm_passes: false,
                target,
                kind: OutputKind::Executable,
            },
        );
        _ = command.args(["-x", "c"]).arg(&c_path);
        if runner == Runner::Wsl {
            _ = command.args([
                "-nostdlib",
                "-static",
                "-ffreestanding",
                "-fno-stack-protector",
            ]);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            runner.run(&executable).unwrap(),
            Some(0),
            "{target:?} {opt:?}"
        );
    }
}

const BCC: &str = r"
Z b_pair(Z z) { return __builtin_complex(__imag__ z + 1, __real__ z - 2); }
Z c_pair(Z);
Z b_var(int n, ...) {
    __builtin_va_list ap; __builtin_va_start(ap,n);
    for(int i=0;i<n;i++) (void)__builtin_va_arg(ap,double);
    Z a=__builtin_va_arg(ap,Z), b=__builtin_va_arg(ap,Z);
    int tail=__builtin_va_arg(ap,int); double last=__builtin_va_arg(ap,double);
    __builtin_va_end(ap); return a+b+tail+last;
}
Z c_var(int,...);
int b_call(void) {
    Z z=__builtin_complex((REAL)3,(REAL)-4);
    Z (*p)(Z)=c_pair;
    if(c_pair(z)!=__builtin_complex((REAL)-3,(REAL)1) || p(z)!=c_pair(z)) return 10;
    if(c_var(0,z,z,7,4.)!=__builtin_complex((REAL)17,(REAL)-8)) return 11;
    if(c_var(7,1.,2.,3.,4.,5.,6.,7.,z,z,7,4.)!=__builtin_complex((REAL)17,(REAL)-8)) return 12;
    if(c_var(8,1.,2.,3.,4.,5.,6.,7.,8.,z,z,7,4.)!=__builtin_complex((REAL)17,(REAL)-8)) return 13;
    return 0;
}
";

const CLANG: &str = r"
Z b_pair(Z); Z b_var(int,...); int b_call(void);
Z c_pair(Z z) { return __builtin_complex(__imag__ z + 1, __real__ z - 2); }
Z c_var(int n, ...) {
    __builtin_va_list ap; __builtin_va_start(ap,n);
    for(int i=0;i<n;i++) (void)__builtin_va_arg(ap,double);
    Z a=__builtin_va_arg(ap,Z), b=__builtin_va_arg(ap,Z);
    int tail=__builtin_va_arg(ap,int); double last=__builtin_va_arg(ap,double);
    __builtin_va_end(ap); return a+b+tail+last;
}
int main(void) {
    Z z=__builtin_complex((REAL)3,(REAL)-4);
    Z (*p)(Z)=b_pair;
    if(b_pair(z)!=__builtin_complex((REAL)-3,(REAL)1) || p(z)!=b_pair(z)) return 1;
    if(b_var(0,z,z,7,4.)!=__builtin_complex((REAL)17,(REAL)-8)) return 2;
    if(b_var(7,1.,2.,3.,4.,5.,6.,7.,z,z,7,4.)!=__builtin_complex((REAL)17,(REAL)-8)) return 3;
    if(b_var(8,1.,2.,3.,4.,5.,6.,7.,8.,z,z,7,4.)!=__builtin_complex((REAL)17,(REAL)-8)) return 4;
    return b_call();
}
";
