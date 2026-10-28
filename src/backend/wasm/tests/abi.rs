//! The WebAssembly C ABI against the bundled Clang: every structure and
//! union of `single_element.c` is passed and returned as Clang passes it
//! on `wasm32-wasip1`, directly as one scalar or indirectly, and values
//! survive the trip.

use std::{
    path::Path,
    process::Command,
};

use super::execution::{
    compile,
    node_available,
    run,
};
use crate::{
    configuration::CompilerConfiguration,
    headers::HeaderSearch,
    ir::{
        ParamAbi,
        Profile,
    },
    pipeline::{
        InlineCopies,
        analyze_translation_unit,
        lower_translation_unit_with_locals,
        parse_translation_unit,
    },
    target::Target,
    translation_phases::Context,
    util::bump::Bump,
};

const SOURCE: &str = include_str!("single_element.c");

/// How a function's single aggregate travels: `indirect`, or the scalar's
/// IR type, for its result and its argument.
fn bcc_passing() -> Vec<(String, String, String)> {
    let configuration = CompilerConfiguration::default().with_target(Target::Wasm32Wasi);
    let tu = Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    let source = tu.alloc_str(SOURCE);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("single_element.c"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    assert!(sema.lowerable(&context));
    let arena = Bump::new();
    let module = lower_translation_unit_with_locals(
        &context,
        &unit,
        &sema,
        &arena,
        None,
        Profile::PostAbi,
        (InlineCopies::Resolve, false),
    )
    .unwrap()
    .module;
    module
        .functions()
        .map(|(_, function)| {
            let signature = module.signature(function.signature);
            let result = match signature.param_abi(0) {
                | Some(ParamAbi::Registers(registers)) => registers.first.name().to_owned(),
                | Some(ParamAbi::Sret(_)) => "indirect".to_owned(),
                | other => panic!("{}: {other:?}", function.name),
            };
            let argument = match signature.param_abi(1) {
                | Some(ParamAbi::Aggregate(_) | ParamAbi::ByVal(_)) | None
                    if signature.params.get(1) == Some(&crate::ir::Type::Ptr) =>
                    "indirect".to_owned(),
                | _ => signature.params[1].name().to_owned(),
            };
            (function.name.to_owned(), result, argument)
        })
        .collect()
}

/// The same, from the bundled Clang's LLVM IR at `-O1`.
fn clang_passing() -> Vec<(String, String, String)> {
    let clang = Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
        "target/llvm/bin/clang.exe"
    } else {
        "target/llvm/bin/clang"
    });
    let output = Command::new(clang)
        .args([
            "--target=wasm32-wasip1",
            "-O1",
            "-S",
            "-emit-llvm",
            "-o",
            "-",
            "-x",
            "c",
            "-",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.take().unwrap().write_all(SOURCE.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(output.status.success());
    let ir = String::from_utf8(output.stdout).unwrap();
    let scalar = |ty: &str| {
        match ty {
            | "i1" | "i8" => "i8",
            | "i16" => "i16",
            | "i32" | "ptr" => "i32",
            | "i64" => "i64",
            | "half" => "f16",
            | "float" => "f32",
            | "fp128" => "f128",
            | other => panic!("unexpected Clang type {other}"),
        }
        .to_owned()
    };
    ir.lines()
        .filter_map(|line| line.strip_prefix("define hidden "))
        .map(|line| {
            let (result, rest) = line.split_once(" @").unwrap();
            let (name, parameters) = rest.split_once('(').unwrap();
            let (result, argument) = if result == "void" {
                ("indirect".to_owned(), "indirect".to_owned())
            } else {
                let first = parameters.split_whitespace().next().unwrap();
                (scalar(result), scalar(first))
            };
            if result == "indirect" {
                assert!(
                    parameters.contains("sret(") && parameters.contains("byval("),
                    "{line}"
                );
            }
            (name.to_owned(), result, argument)
        })
        .collect()
}

#[test]
fn records_travel_as_clang_passes_them() {
    let mut expected = clang_passing();
    let mut actual = bcc_passing();
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected);
}

#[test]
fn single_bit_fields_and_records_keep_their_values() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the WebAssembly ABI execution test");
        return;
    }
    let source = format!(
        "{SOURCE}
__attribute__((noinline)) static struct bf id_bf(struct bf a) {{ a.x = a.x - 1; return a; }}
__attribute__((noinline)) static struct bfll id_bfll(struct bfll a) {{ a.x = a.x * 2; return a; }}
__attribute__((noinline)) static union u2 id_u2(union u2 a) {{ a.b++; return a; }}
__attribute__((noinline)) static struct n1 id_n1(struct n1 a) {{ a.inner.x = -a.inner.x; return a; \
         }}
int main(void) {{
    struct bf b = {{ -3 }};
    struct bfll l = {{ 0x12345678ULL }};
    union u2 u = {{ 41 }};
    struct n1 n = {{ {{ 2 }} }};
    if (id_bf(b).x != -4) return 1;
    if (id_bfll(l).x != 0x2468ACF0ULL) return 2;
    if (id_u2(u).a != 42) return 3;
    if (id_n1(n).inner.x != -2) return 4;
    return f_bfb((struct bfb){{ 1 }}).b + f_ac1((struct ac1){{ {{ 5 }} }}).a[0];
}}
"
    );
    for optimize in [false, true] {
        let bytes = compile(&source, Target::Wasm32Wasi, optimize).unwrap();
        let run = run("single-element", &bytes);
        assert_eq!(run.status, 6, "{}", run.stderr);
    }
}
