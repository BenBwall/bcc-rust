//! General draft instruction edits and dense rebuilding.
use super::{
    super::draft::Constant,
    *,
};
use crate::ir::{
    Block,
    Entity,
    InstData,
    InstFlags,
    Opcode,
    Type,
    Value,
};

#[test]
fn large_switch_predecessors_and_dce_are_linear() {
    use std::fmt::Write as _;
    let targets = 20_000;
    let mut text =
        format!("function @f(i32) external {{\nblock0(v0: i32):\n    switch v0, block{targets}, [");
    for target in 1..=targets {
        if target != 1 {
            text.push_str(", ");
        }
        write!(
            text,
            "{}: block{target}, {}: block{target}",
            target * 2,
            target * 2 + 1
        )
        .unwrap();
    }
    text.push_str("]\n");
    for target in 1..=targets {
        writeln!(text, "block{target}:").unwrap();
        text.push_str(if target == 2 {
            "    jump block1\n"
        } else {
            "    return\n"
        });
    }
    text.push_str("}\n");
    let arena = Bump::new();
    let mut module = parse(&arena, &text);
    let body = module.functions().next().unwrap().1.body.as_ref().unwrap();
    let start = Instant::now();
    for _ in 0..4 {
        let scratch = Bump::new();
        let draft = Draft::lift(body, &scratch);
        let predecessors = draft.predecessors();
        assert_eq!(predecessors.of(Block::new(0)), []);
        assert_eq!(
            predecessors.of(Block::new(1)),
            [Block::new(0), Block::new(2)]
        );
        for target in 2..=targets {
            assert_eq!(predecessors.of(Block::new(target)), [Block::new(0)]);
        }
    }
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "four draft predecessor constructions took {:?}",
        start.elapsed()
    );
    let scratch = Bump::new();
    let start = Instant::now();
    _ = optimize(&mut module, &only(&[Pass::Dce]), &scratch);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "DCE took {:?}",
        start.elapsed()
    );
    assert_verifies(&module, "large switch after DCE");
}

#[test]
fn insert_rewrite_move_and_replace_lower_densely() {
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse(
        &arena,
        "function @f(i32) -> i32 external {
block0(v0: i32):
    jump block1
block1:
    v1 = iadd.i32 v0, v0
    return v1
}
",
    );
    let (func, _) = module.functions().next().unwrap();
    let body = module.function_mut(func).body.take().unwrap();
    let mut draft = Draft::lift(&body, &scratch);
    let inst = draft.block(Block::new(1)).insts[0];
    let added = draft
        .add_inst(
            Block::new(1),
            0,
            InstData::Select {
                ty:   Type::I32,
                args: [Value::new(0); 3],
            },
            Some(Type::I32),
        )
        .unwrap();
    // Rewrite the inserted instruction before using it; its result stays.
    let crate::ir::ValueDef::Result(added_inst) = draft.value_def(added) else {
        unreachable!()
    };
    draft.set_constant(added_inst, Constant::Int(Type::I32, 17));
    draft.replace_inst(
        added_inst,
        InstData::Binary {
            opcode: Opcode::Imul,
            ty:     Type::I32,
            flags:  InstFlags::empty(),
            args:   [Value::new(0); 2],
        },
    );
    assert!(draft.inst_constant(added_inst).is_none());
    draft.move_inst(added_inst, Block::new(1), Block::new(0));
    draft.replace(draft.inst_result(inst).unwrap(), added);
    draft.remove_inst(inst);
    let c = draft.add_constant(Constant::Int(Type::I128, 1 << 100));
    let crate::ir::ValueDef::Result(ci) = draft.value_def(c) else {
        unreachable!()
    };
    draft.remove_inst(ci);
    draft.sweep();
    assert_eq!(draft.use_counts()[0], 2);
    draft.lower(&mut module, func);
    assert_verifies(&module, "edited draft");
    assert_eq!(
        module.to_string(),
        "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = imul.i32 v0, v0
    jump block1
block1:
    return v1
}
"
    );
}

#[test]
fn inserted_select_and_owned_call_operands_are_visited_and_rebuilt() {
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse(
        &arena,
        "function @g(i32, i32, i32) -> i32 external

function @f(i1, i32, i32) -> i32 external {
block0(v0: i1, v1: i32, v2: i32):
    v3 = call @g(v1, v2, v1)
    return v3
}
",
    );
    let func = module.functions().find(|(_, f)| f.name == "f").unwrap().0;
    let body = module.function_mut(func).body.take().unwrap();
    let mut draft = Draft::lift(&body, &scratch);
    let block = Draft::entry();
    let call = draft.block(block).insts[0];
    let value = draft
        .add_inst(
            block,
            0,
            InstData::Select {
                ty:   Type::I32,
                args: [Value::new(0), Value::new(1), Value::new(2)],
            },
            Some(Type::I32),
        )
        .unwrap();
    let call_data = *draft.inst(call);
    let result = draft
        .add_inst(block, 1, call_data, Some(Type::I32))
        .unwrap();
    let crate::ir::ValueDef::Result(new_call) = draft.value_def(result) else {
        unreachable!()
    };
    draft.set_call_args(new_call, &[value, Value::new(2), value]);
    draft.replace(Value::new(3), result);
    draft.remove_inst(call);
    assert_eq!(draft.use_counts()[value.index()], 2);
    draft.lower(&mut module, func);
    assert_verifies(&module, "new select and call");
    assert!(module.to_string().contains(
        "v3 = select.i32 v0, v1, v2
    v4 = call @g(v3, v2, v3)"
    ));
}

#[test]
fn inserted_void_call_and_wide_constant_survive_lowering() {
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse(
        &arena,
        "function @g(i128) external

function @f() -> i128 external {
block0:
    v0 = iconst.i128 0
    call @g(v0)
    return v0
}
",
    );
    let func = module.functions().find(|(_, f)| f.name == "f").unwrap().0;
    let body = module.function_mut(func).body.take().unwrap();
    let mut draft = Draft::lift(&body, &scratch);
    let block = Draft::entry();
    let call = draft.block(block).insts[1];
    let data = *draft.inst(call);
    draft.remove_inst(call);
    draft.sweep();
    let wide = draft.add_constant(Constant::Int(Type::I128, 1 << 100));
    assert_eq!(draft.add_inst(block, 1, data, None), None);
    let call = draft.block(block).insts[1];
    draft.set_call_args(call, &[wide]);
    draft.replace(Value::new(0), wide);
    draft.lower(&mut module, func);
    assert_verifies(&module, "void call and wide constant");
    assert!(module.to_string().contains("call @g(v0)"));
    assert!(
        module
            .to_string()
            .contains("1267650600228229401496703205376")
    );
}
