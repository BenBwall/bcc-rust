//! Text and verifier coverage of dynamic stack positions and aggregate fetches.
use super::*;

#[test]
fn stack_positions_round_trip_and_verify() {
    let text = "function @f(i64) -> i32 external {
block0(v0: i64):
    v1 = stack_save
    v2 = dyn_alloca.i64 v0, align 16
    v3 = iconst.i32 7
    store.i32 v3, v2, align 4
    v4 = load.i32 v2, align 4
    stack_restore v1
    return v4
}
";
    assert_round_trip(text);
    assert_eq!(verify_text(text), Vec::<String>::new());
    let bad = text.replace("stack_restore v1", "stack_restore v0");
    assert!(verify_text(&bad).iter().any(|e| e.contains("ptr")));
    let arena = Bump::new();
    let module = parse(&arena, text);
    let mut llvm = String::new();
    crate::backend::llvm::emit_module(&module, crate::target::Target::WindowsGnu, &mut llvm)
        .unwrap();
    assert!(llvm.contains("llvm.stacksave.p0"), "{llvm}");
    assert!(llvm.contains("llvm.stackrestore.p0"), "{llvm}");
}

#[test]
fn aggregate_fetches_and_typed_variadic_tails_round_trip() {
    let text = "function @f(ptr, ptr, ptr) external {
block0(v0: ptr, v1: ptr, v2: ptr):
    v3 = va_arg_aggregate v0 : (ptr aggregate(size 4, align 4, [0: i32]))
    call_indirect v1(v2, v3) : (ptr, ..., ptr aggregate(size 4, align 4, [0: i32]))
    return
}
";
    assert_round_trip(text);
    assert_eq!(verify_text(text), Vec::<String>::new());
    let arena = Bump::new();
    let module = parse(&arena, text);
    assert!(
        verify(&module, Profile::PostAbi)
            .iter()
            .any(|e| e.contains("va_arg_aggregate"))
    );
    let malformed = text.replace("size 4", "size 1");
    assert_ne!(verify_text(&malformed), Vec::<String>::new());
}

#[test]
fn aggregate_fetch_descriptor_properties_are_verified_at_the_fetch() {
    let aggregate = "ptr aggregate(size 4, align 4, [0: i32])";
    for (property, descriptor) in [
        ("missing aggregate metadata", "(ptr)".to_owned()),
        ("extra parameter", format!("({aggregate}, i32)")),
        ("variadic descriptor", format!("({aggregate}, ...)")),
        ("returned value", format!("({aggregate}) -> i32")),
        (
            "wrong pointee",
            "(i32 aggregate(size 4, align 4, [0: i32]))".to_owned(),
        ),
    ] {
        let text = format!(
            "function @f(ptr) external {{\nblock0(v0: ptr):\n    v1 = va_arg_aggregate v0 : \
             {descriptor}\n    return\n}}\n"
        );
        let arena = Bump::new();
        let module = parse(&arena, &text);
        let (_, function) = module.functions().next().unwrap();
        let body = function.body.as_ref().unwrap();
        let fetch = body.block_insts(body.blocks().next().unwrap())[0];
        let scratch = Bump::new();
        let errors = verify_module(&module, Profile::PreAbi, &scratch);
        assert!(
            !errors.is_empty(),
            "{property} reached the interpreter: {text}"
        );
        assert!(
            errors
                .iter()
                .all(|error| error.item == "f" && error.location == Location::Inst(fetch)),
            "{property}: {errors:?}"
        );
        assert!(
            errors.iter().any(|error| matches!(
                error.kind,
                VerifierErrorKind::InstructionType {
                    opcode: Opcode::VaArgAggregate,
                    ..
                }
            )),
            "{property}: {errors:?}"
        );
    }
}

#[test]
fn dynamic_stack_alignment_survives_text_and_llvm_emission() {
    for alignment in [1, 16, 32, 64] {
        let text = format!(
            "function @f(i64) -> ptr external {{\nblock0(v0: i64):\n    v1 = dyn_alloca.i64 v0, \
             align {alignment}\n    return v1\n}}\n"
        );
        assert_round_trip(&text);
        assert_eq!(verify_text(&text), Vec::<String>::new());
        let arena = Bump::new();
        let module = parse(&arena, &text);
        let mut llvm = String::new();
        crate::backend::llvm::emit_module(&module, crate::target::Target::WindowsGnu, &mut llvm)
            .unwrap();
        assert!(
            llvm.lines()
                .any(|line| line.contains("alloca i8")
                    && line.contains(&format!("align {alignment}"))),
            "{llvm}"
        );
    }
    let legacy = "function @f(i64) -> ptr external {\nblock0(v0: i64):\n v1 = dyn_alloca.i64 v0\n \
                  return v1\n}\n";
    assert!(
        parse(&Bump::new(), legacy)
            .to_string()
            .contains("dyn_alloca.i64 v0, align 16")
    );
    for alignment in ["0", "3", "4294967296"] {
        let text = format!(
            "function @f(i64) -> ptr external {{\nblock0(v0: i64):\n v1 = dyn_alloca.i64 v0, \
             align {alignment}\n return v1\n}}\n"
        );
        let _error = parse_module(&Bump::new(), &text).unwrap_err();
    }
}
