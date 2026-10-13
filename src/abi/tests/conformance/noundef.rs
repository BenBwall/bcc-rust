//! Clang's defined-value policy at -O0 for both ABI families.

use super::*;
use crate::ir::ParamAbi;

const SOURCE: &str = "
struct One { int x; }; struct Pair { long long x, y; };
struct Big { long long x, y, z; }; struct Float { float x; };
_Bool boolean(_Bool b) { return b; }
signed char narrow(signed char x, unsigned short y) { return x + y; }
int scalar(int x, double y, void *p) { return x; }
struct One one(struct One x) { return x; }
struct Pair pair(struct Pair x) { return x; }
struct Big big(struct Big x) { return x; }
struct Float floating(struct Float x) { return x; }
__int128 wide(__int128 x) { return x; }
int variadic(int x, ...) { return x; }
int calls(void) { return variadic(1, 2, 3.0); }
int indirect(int (*f)(int, ...)) { return f(1, 2, 3.0); }
";

#[test]
fn c_noundef_signatures_and_calls_match_bundled_clang() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        // Emitting LLVM IR for an explicit target needs only the clang
        // binary, not the Windows GNU sysroot that linking would.
        let toolchain = Toolchain::bundled(Target::LinuxGnu)
            .expect("bundled Clang is required for this comparison");
        let directory = TempDir::new("noundef-signatures");
        let source = directory.join("signatures.c");
        std::fs::write(&source, SOURCE).unwrap();
        let clang = Command::new(toolchain.clang())
            .args(["-O0", "-S", "-emit-llvm", "-o", "-"])
            .arg(format!("--target={}", target.triple()))
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            clang.status.success(),
            "{}",
            String::from_utf8_lossy(&clang.stderr)
        );
        let clang = String::from_utf8(clang.stdout).unwrap();
        let tu = Bump::new();
        let arena = Bump::new();
        let mut module = lower_c(&tu, &arena, SOURCE, target);
        for (_, function) in module.functions() {
            let sig = module.signature(function.signature);
            assert!(!sig.result_noundef);
            for index in 0..sig.params.len() {
                assert_eq!(
                    sig.param_noundef(index),
                    !sig.param_abi(index).is_some_and(ParamAbi::is_result)
                );
            }
        }
        lower_abi(&mut module, target, &Bump::new()).unwrap();
        assert_verified(&module, Profile::PostAbi, &module.to_string());
        let mut bcc = String::new();
        emit_module(&module, target, &mut bcc).unwrap();
        for name in [
            "boolean", "narrow", "scalar", "one", "pair", "big", "floating", "wide", "variadic",
            "calls", "indirect",
        ] {
            let marker = format!("@{name}(");
            let signature = |text: &str| {
                text.lines()
                    .find(|line| line.starts_with("define ") && line.contains(&marker))
                    .unwrap()
                    .to_owned()
            };
            let (ours, reference) = (signature(&bcc), signature(&clang));
            assert_eq!(
                mask(&ours, &marker),
                mask(&reference, &marker),
                "{target:?} {name}\nbcc: {ours}\nclang: {reference}"
            );
            assert_eq!(
                ours.split_once('@').unwrap().0.contains("noundef"),
                reference.split_once('@').unwrap().0.contains("noundef")
            );
        }
        let direct = |text: &str| {
            text.lines()
                .find(|line| line.contains("call ") && line.contains("@variadic("))
                .unwrap()
                .to_owned()
        };
        assert_eq!(mask(&direct(&bcc), "@variadic("), vec![true, true, true]);
        assert_eq!(
            mask(&direct(&bcc), "@variadic("),
            mask(&direct(&clang), "@variadic(")
        );
        // The call through a parameter has no @name. Each of its three
        // scalar arguments must carry the signature's variadic promise.
        for text in [&bcc, &clang] {
            let indirect = text
                .lines()
                .find(|line| line.contains("call ") && !line.contains('@'))
                .unwrap();
            assert_eq!(indirect.matches("noundef").count(), 3, "{indirect}");
        }
    }
}

/// Attribute mask in these scalar/coerced signatures. byval/sret contain
/// only a named type or an array type, so commas separate parameters.
fn mask(line: &str, marker: &str) -> Vec<bool> {
    let args = line.split_once(marker).unwrap().1;
    let end = args.rfind(')').unwrap();
    args[..end]
        .split(',')
        .filter(|param| !param.trim().is_empty() && param.trim() != "...")
        .map(|param| param.contains("noundef"))
        .collect()
}

#[test]
fn mixed_aggregate_variadic_attribute_masks_match_clang() {
    let source = "struct S { int x,y; }; struct B { long long x,y,z; }; int take(int,...); int \
                  direct(struct S s, struct B b) { return take(0,s,123,b,456); } int indirect(int \
                  (*p)(int,...),struct S s,struct B b) { return p(0,s,123,b,456); }";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let toolchain = Toolchain::bundled(Target::LinuxGnu).expect("bundled Clang");
        let directory = TempDir::new("mixed-tail-noundef");
        let path = directory.join("source.c");
        std::fs::write(&path, source).unwrap();
        let clang = crate::test_support::native::run_native(
            Command::new("python")
                .arg("scripts/cargo_runner.py")
                .arg(toolchain.clang())
                .args(["-O0", "-S", "-emit-llvm", "-o", "-"])
                .arg(format!("--target={}", target.triple()))
                .arg(&path),
        )
        .unwrap();
        assert!(
            clang.status.success(),
            "{}",
            String::from_utf8_lossy(&clang.stderr)
        );
        let clang = String::from_utf8(clang.stdout).unwrap();
        let tu = Bump::new();
        let arena = Bump::new();
        let mut module = lower_c(&tu, &arena, source, target);
        lower_abi(&mut module, target, &Bump::new()).unwrap();
        let mut bcc = String::new();
        emit_module(&module, target, &mut bcc).unwrap();
        for indirect in [false, true] {
            let call = |text: &str| {
                text.lines()
                    .find(|line| {
                        line.contains("call ")
                            && if indirect {
                                !line.contains('@')
                            } else {
                                line.contains("@take(")
                            }
                    })
                    .unwrap()
                    .to_owned()
            };
            let (ours, reference) = (call(&bcc), call(&clang));
            let attributes = |line: &str| {
                let call = line.split_once("call ").unwrap().1;
                let callee = call.find(['@', '%']).unwrap();
                let args = call[callee..].split_once('(').unwrap().1;
                let end = args.rfind(')').unwrap();
                args[..end]
                    .split(',')
                    .map(|arg| arg.contains("noundef"))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                attributes(&ours),
                attributes(&reference),
                "{target:?} {indirect}\nbcc: {bcc}\nclang: {clang}"
            );
        }
    }
}
