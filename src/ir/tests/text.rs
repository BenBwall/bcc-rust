//! The textual form: parse-print round trips and parse errors.

use super::*;

/// The two examples of `middle-end.md`, "Textual form".
pub(super) const PLAN_EXAMPLES: &str = "\
function @add(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 nsw v0, v1
    return v2
}

function @count(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 slt v2, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 nsw v3, v2
    v6 = iconst.i32 1
    v7 = iadd.i32 nsw v2, v6
    jump block1(v7, v5)
block3:
    return v3
}
";

/// A module that uses every opcode, flag and kind of global.
const EVERY_FORM: &str = "\
target triple = \"x86_64-unknown-linux-gnu\"
target datalayout = \"e-m:e-i64:64-n8:16:32:64-S128\"

global @counter internal size 4, align 4 = zero
global @message external constant size 6, align 1 = bytes \"68656c6c6f00\"
global @table internal constant size 16, align 8 = bytes \"00000000000000000000000000000000\" \
                          relocs [0: @message + 2, 8: @add - 1]
global @errno external size 4, align 4

function @add(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 nsw v0, v1
    return v2
}

function @printf(ptr, ...) -> i32 external

function @everything(i64, ptr, f64, ...) internal {
    slot0 = stack_slot 16, align 8
    slot1 = stack_slot 24, align 8
block0(v0: i64, v1: ptr, v2: f64):
    v3 = iconst.i64 -1
    v4 = isub.i64 nsw nuw v0, v3
    v5 = sdiv.i64 exact v4, v3
    v6 = and.i64 v5, v0
    v7 = lshr.i64 exact v6, v3
    v8 = fadd.f64 v2, v2
    v9 = fneg.f64 v8
    v10 = icmp.i64 ult v0, v4
    v11 = fcmp.f64 uno v8, v9
    v12 = select.i64 v10, v0, v4
    v13 = freeze.i64 v12
    v14 = trunc.i32 v13
    v15 = sext.i64 v14
    v16 = zext.i128 v15
    v17 = fptosi.i32 v2
    v18 = sitofp.f32 v17
    v19 = fpext.f80 v18
    v20 = ptrtoint.i64 v1
    v21 = inttoptr.ptr v20
    v22 = bitcast.i64 v2
    v23 = iconst.i128 -170141183460469231731687303715884105728
    v24 = fconst.f64 0x3FF0000000000000
    v25 = fconst.f80 0x3FFF8000000000000000
    v26 = poison.i32
    v27 = null
    v28 = stack_addr slot0
    v29 = global_addr @counter
    v30 = func_addr @add
    v31 = ptr_add inbounds v28, v0
    v32 = load.i32 volatile v29, align 4, tag 7
    store.i32 v32, v31, align 4
    store.i64 volatile v0, v28, align 8, tag 0
    v33 = iconst.i64 16
    copy volatile may_overlap v28, v31, v33, align 8
    v34 = iconst.i8 0
    fill v28, v34, v33, align 1
    v35 = call @add(v14, v32)
    v36 = call @printf(v1, v35, v2)
    v37 = call_indirect v30(v32, v35) : (i32, i32) -> i32
    v38 = stack_addr slot1
    va_start v38
    v39 = va_arg.i32 v38
    va_copy v28, v38
    va_end v38
    brif v11, block1, block2(v39)
block1:
    switch v32, block3, [-1: block2(v26), 0: block3, 7: block4]
block2(v40: i32):
    jump block3
block3:
    return
block4:
    unreachable
}
";

#[test]
fn plan_examples_round_trip_and_verify() {
    assert_round_trip(PLAN_EXAMPLES);
    assert_eq!(verify_text(PLAN_EXAMPLES), Vec::<String>::new());
}

#[test]
fn every_form_round_trips_and_verifies() {
    assert_round_trip(EVERY_FORM);
    assert_eq!(verify_text(EVERY_FORM), Vec::<String>::new());
}

#[test]
fn parsed_module_holds_what_the_text_says() {
    let arena = Bump::new();
    let module = parse(&arena, EVERY_FORM);
    assert_eq!(module.triple(), "x86_64-unknown-linux-gnu");
    let Some(Symbol::Function(func)) = module.symbol("everything") else {
        panic!("@everything is a function");
    };
    assert_eq!(module.symbol_name(Symbol::Function(func)), "everything");
    let signature = module.signature(module.function(func).signature);
    assert_eq!(signature.params, [Type::I64, Type::Ptr, Type::F64]);
    assert!(signature.variadic);
    assert_eq!(module.function(func).linkage, Linkage::Internal);
    let body = module.function(func).body.as_ref().unwrap();
    assert_eq!(body.block_count(), 5);
    assert_eq!(body.value_count(), 41);
    assert_eq!(
        body.stack_slot(StackSlot::new(1)),
        StackSlotData {
            size:  24,
            align: Align::from_bytes(8).unwrap(),
        }
    );
    assert_eq!(
        body.constant(ConstId::new(0)),
        (1 << 127),
        "the i128 constant is first in the pool"
    );
    // The i128 and f80 constants went to the wide pool.
    assert_eq!(body.constant_count(), 2 + 3);
    let load = body.block_insts(Block::new(0))[29];
    assert_eq!(
        body.mem_flags(load),
        Some(MemFlags {
            volatile: true,
            align:    Align::from_bytes(4).unwrap(),
            tag:      AccessTag::new(7),
        })
    );
    let InstData::Switch { cases, .. } = *body.inst(body.terminator(Block::new(1)).unwrap()) else {
        panic!("block1 ends in a switch");
    };
    let constants: Vec<u128> = body
        .switch_cases(cases)
        .map(|(bits, _): (u128, BlockCall)| bits)
        .collect();
    let call = body.block_insts(Block::new(0))[36];
    let InstData::Call { args, .. } = *body.inst(call) else {
        panic!("the 37th instruction is a call");
    };
    let args: ValueList = args;
    assert_eq!(body.value_list(args), [Value::new(14), Value::new(32)]);
    assert_eq!(
        constants,
        [0xFFFF_FFFF, 0, 7],
        "case constants are truncated to i32"
    );
    let Some(Symbol::Global(table)) = module.symbol("table") else {
        panic!("@table is a global");
    };
    let Some(GlobalInit::Bytes { relocations, .. }) = module.global(table).init else {
        panic!("@table has bytes");
    };
    assert_eq!(relocations[1].addend, -1);
    assert_eq!(
        relocations[1].symbol,
        module.symbol("add").unwrap(),
        "a relocation may name a function declared later"
    );
}

#[test]
fn uses_may_precede_definitions_in_the_text() {
    let text = "\
function @f() -> i32 external {
block0:
    jump block2
block1:
    v1 = iadd.i32 v0, v0
    return v1
block2:
    v0 = iconst.i32 3
    jump block1
}
";
    assert_round_trip(text);
    assert_eq!(verify_text(text), Vec::<String>::new());
}

#[test]
fn whitespace_and_comments_are_not_canonical() {
    let arena = Bump::new();
    let module = parse(
        &arena,
        "; a comment\nfunction @f(i8)->i8 internal{\nblock0(v0:i8): ; entry\nv1=iconst.i8 255\n  \
         return v1}",
    );
    assert_eq!(
        module.to_string(),
        "function @f(i8) -> i8 internal {\nblock0(v0: i8):\n    v1 = iconst.i8 -1\n    return \
         v1\n}\n"
    );
}

#[test]
fn bad_text_reports_where_and_why() {
    let cases = [
        (
            "function @f() external {\nblock0:\n    v0 = iconst.i32 1\n    v0 = iconst.i32 2\n    \
             return\n}",
            "4:5: `v0` is defined twice",
        ),
        (
            "function @f() external {\nblock0:\n    v1 = iconst.i32 1\n    return\n}",
            "3:5: `v1` skips a number; blocks, values and slots are numbered from 0 without gaps",
        ),
        (
            "function @f() -> i32 external {\nblock0:\n    return v5\n}",
            "3:12: `v5` is never defined",
        ),
        (
            "function @f() external {\nblock0:\n    jump block4\n}",
            "3:10: `block4` is never defined",
        ),
        (
            "function @f() external {\nblock0:\n    v0 = frobnicate.i32 1\n}",
            "3:10: unknown opcode `frobnicate`",
        ),
        (
            "function @f() external {\nblock0:\n    v0 = iconst.i33 1\n}",
            "3:17: unknown type `i33`",
        ),
        (
            "function @f() external {\nblock0:\n    iconst.i32 1\n}",
            "3:5: `iconst` defines a value that needs a name",
        ),
        (
            "function @f() external {\nblock0:\n    v0 = return\n}",
            "3:10: `return` defines no value",
        ),
        (
            "function @f() external {\nblock0:\n    v0 = iconst.i8 256\n}",
            "3:20: the constant does not fit in i8",
        ),
        (
            "function @f() external {\nblock0:\n    v0 = iconst.i8 -129\n}",
            "3:20: the constant does not fit in i8",
        ),
        (
            "function @f() external {\n    v0 = iconst.i8 1\n}",
            "2:10: an instruction must follow a block header",
        ),
        (
            "function @f() external\nfunction @f() external",
            "2:10: `@f` is declared twice",
        ),
        (
            "global @g internal size 4, align 4\nfunction @f() external {\nblock0:\n    call \
             @g()\n}",
            "4:10: `@g` is not a declared function",
        ),
        (
            "global @g internal size 4, align 3",
            "1:34: an alignment is a power of two up to 2^32",
        ),
        (
            "global @g internal size 1, align 1 = bytes \"0g\"",
            "1:44: bytes are written as pairs of hexadecimal digits",
        ),
        (
            "global @g internal size 8, align 8 = bytes \"0000000000000000\" relocs [0: @h]",
            "1:74: `@h` is not declared",
        ),
        (
            "function @f() external {\nblock0:\n    v0 = iadd.i32 v0 v0\n}",
            "3:22: expected `,`, found `v0`",
        ),
        (
            "function @f() external {\nblock0:\n    v0 = iconst.i32 1",
            "3:22: expected `}`, found the end",
        ),
        ("target triple = \"x86", "1:17: unterminated string"),
        ("function @f() external #", "1:24: unexpected character '#'"),
        (
            "function @f() external {\nblock0:\n    v0 = iconst.i32 \
             99999999999999999999999999999999999999999\n}",
            "3:21: number too large",
        ),
    ];
    for (text, expected) in cases {
        let arena = Bump::new();
        match parse_module(&arena, text) {
            | Ok(module) => panic!("parsed {text:?} as\n{module}"),
            | Err(error) => assert_eq!(error.to_string(), expected, "for {text:?}"),
        }
    }
}

#[test]
fn parse_errors_carry_structured_kinds() {
    let arena = Bump::new();
    let error: ParseError<'_> = parse_module(
        &arena,
        "function @f() external {\nblock0:\n    v0 = icmp.i32 v1, v1\n}",
    )
    .unwrap_err();
    assert_eq!(error.line, 3);
    assert_eq!(error.column, 19);
    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "an integer condition",
            found:    "v1",
        }
    );
}
