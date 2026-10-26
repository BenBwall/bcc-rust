//! Inline assembly: the textual form round-trips every operand kind and
//! clobber, the builder adds descriptors to the module, and the verifier
//! keeps the descriptor's operand rules.

use super::*;

/// Register, memory, immediate, tied and flag operands, an aggregate memory
/// operand and every clobber kind.
const ASM: &str = "\
function @f(i32, ptr, ptr) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0(v0: i32, v1: ptr, v2: ptr):
    v3 = stack_addr slot0
    v4 = stack_addr slot1
    v5 = iconst.i32 5
    v6 = func_addr @f
    asm sideeffect \"op $0 ${1:k} $$5 ${:uid} $(a$|b$) \\u{22}\" (out \"&r\" i32 v3, out \"m\" i32 \
                   v1, out \"@ccz\" i32 v4, in \"{rax,rbx}\" i32 v0, in \"i\" i32 v5, in \"i\" \
                   ptr v6, in \"m\" i8 bytes 16 v2, in \"0\" i32 v0) clobber(memory, flags, rcx, \
                   r12, xmm3, st, mm1, k2)
    asm \"\" ()
    v7 = load.i32 v3, align 4
    return v7
}
";

#[test]
fn inline_asm_round_trips_and_verifies() {
    assert_round_trip(ASM);
    assert!(verify_text(ASM).is_empty(), "{:?}", verify_text(ASM));
}

#[test]
fn builder_adds_the_descriptor_to_the_module() {
    let arena = Bump::new();
    let mut module = Module::new(&arena);
    let signature = module.intern_signature(&[], None, false);
    let func = module.declare_function("g", signature, Linkage::External);
    let mut builder = FunctionBuilder::new(&mut module, func);
    let entry = builder.create_entry_block();
    builder.switch_to_block(entry);
    let operands = [AsmOperand {
        output:        false,
        early_clobber: false,
        constraint:    AsmConstraint::Sse,
        ty:            Type::F64,
        bytes:         8,
    }];
    let value = builder.fconst(Type::F64, 0);
    _ = builder.inline_asm(
        &InlineAsm {
            template:     "nop # $0",
            operands:     &operands,
            clobbers:     AsmClobbers::default(),
            side_effects: true,
        },
        &[value],
    );
    builder.ret(None);
    builder.finish();
    let text = module.to_string();
    assert!(
        text.contains("asm sideeffect \"nop # $0\" (in \"x\" f64 v0)"),
        "{text}"
    );
    assert_eq!(verify(&module, Profile::PreAbi), Vec::<String>::new());
}

#[test]
fn verifier_keeps_the_operand_rules() {
    let cases = [
        ("asm \"\" (in \"r\" i32 v0, in \"r\" i32 v0)", ""),
        (
            "asm \"\" (in \"r\" i64 v0)",
            "operand 0 is i32, expected i64",
        ),
        (
            "asm \"\" (out \"r\" i32 v0)",
            "operand 0 is i32, expected ptr",
        ),
        (
            "asm \"\" (in \"r\" i32 v0, out \"r\" i32 v1)",
            "an output follows an input",
        ),
        (
            "asm \"\" (in \"x\" i32 v0)",
            "an SSE register holds a f32 or f64",
        ),
        (
            "asm \"\" (in \"i\" i32 v0)",
            "an immediate is not an iconst, global_addr or func_addr",
        ),
        (
            "asm \"\" (in \"0\" i32 v0)",
            "a tied input names no register output of its type",
        ),
        (
            "asm \"\" (out \"r\" i64 v1, in \"0\" i32 v0)",
            "a tied input names no register output of its type",
        ),
        (
            "asm \"\" (in \"@ccz\" i32 v0)",
            "a flag operand is not an integer output",
        ),
        ("asm \"\" (in \"&r\" i32 v0)", "an input is early-clobber"),
        (
            "asm \"$1\" (in \"r\" i32 v0)",
            "the template names a missing operand",
        ),
        (
            "asm \"$x\" (in \"r\" i32 v0)",
            "the template has a malformed `$` escape",
        ),
        (
            "asm \"${0\" (in \"r\" i32 v0)",
            "the template has a malformed `$` escape",
        ),
    ];
    for (inst, expected) in cases {
        let text = format!(
            "function @f(i32, ptr) external {{\nblock0(v0: i32, v1: ptr):\n    {inst}\n    \
             return\n}}\n"
        );
        let errors = verify_text(&text);
        if expected.is_empty() {
            assert!(errors.is_empty(), "{inst}: {errors:?}");
        } else {
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "{inst}: expected {expected}, got {errors:?}"
            );
        }
    }
}

#[test]
fn malformed_operands_do_not_parse() {
    for inst in [
        "asm \"\" (both \"r\" i32 v0)",
        "asm \"\" (in \"q\" i32 v0)",
        "asm \"\" (in \"{rsx}\" i32 v0)",
        "asm \"\" (in \"r\" i32 v0) clobber(rfoo)",
    ] {
        let text =
            format!("function @f(i32) external {{\nblock0(v0: i32):\n    {inst}\n    return\n}}\n");
        let arena = Bump::new();
        assert!(parse_module(&arena, &text).is_err(), "{inst}");
    }
}

#[test]
fn template_pieces_follow_llvm_syntax() {
    use super::super::inline_asm::{
        TemplatePiece,
        template_pieces,
    };
    let pieces: Vec<_> = template_pieces("a$0 ${12:k}$$ ${:uid}$(x$|y$)")
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        pieces,
        [
            TemplatePiece::Text("a"),
            TemplatePiece::Operand {
                operand:  0,
                modifier: None,
            },
            TemplatePiece::Text(" "),
            TemplatePiece::Operand {
                operand:  12,
                modifier: Some(b'k'),
            },
            TemplatePiece::Text("$"),
            TemplatePiece::Text(" "),
            TemplatePiece::UniqueId,
            TemplatePiece::DialectOpen,
            TemplatePiece::Text("x"),
            TemplatePiece::DialectNext,
            TemplatePiece::Text("y"),
            TemplatePiece::DialectClose,
        ]
    );
}
