//! Building functions and printing them.

use super::*;

/// Builds the two examples of `middle-end.md` the way lowering would.
fn build_plan_examples(module: &mut Module<'_>) {
    let binary = module.intern_signature(&[Type::I32, Type::I32], Some(Type::I32), false);
    let add = module.declare_function("add", binary, Linkage::External);
    let mut builder = FunctionBuilder::new(module, add);
    let entry = builder.create_entry_block();
    builder.switch_to_block(entry);
    let [a, b] = [0, 1].map(|index| builder.body().block_params(entry)[index]);
    let sum = builder.binary(Opcode::Iadd, InstFlags::NSW, a, b);
    builder.ret(Some(sum));
    builder.finish();

    let unary = module.intern_signature(&[Type::I32], Some(Type::I32), false);
    let count = module.declare_function("count", unary, Linkage::External);
    let mut builder = FunctionBuilder::new(module, count);
    _ = builder.create_stack_slot(4, Align::from_bytes(4).unwrap());
    let entry = builder.create_entry_block();
    let header = builder.create_block();
    let body = builder.create_block();
    let exit = builder.create_block();
    builder.switch_to_block(entry);
    let limit = builder.body().block_params(entry)[0];
    let zero = builder.iconst(Type::I32, 0);
    let index = builder.append_block_param(header, Type::I32);
    let total = builder.append_block_param(header, Type::I32);
    builder.jump(header, &[zero, zero]);
    builder.switch_to_block(header);
    let more = builder.icmp(IntCC::Slt, index, limit);
    builder.brif(more, (body, &[]), (exit, &[]));
    builder.switch_to_block(body);
    let next_total = builder.binary(Opcode::Iadd, InstFlags::NSW, total, index);
    let one = builder.iconst(Type::I32, 1);
    let next_index = builder.binary(Opcode::Iadd, InstFlags::NSW, index, one);
    builder.jump(header, &[next_index, next_total]);
    builder.switch_to_block(exit);
    builder.ret(Some(total));
    assert_eq!(builder.current_block(), Some(exit));
    builder.finish();
}

#[test]
fn builder_reproduces_the_plan_examples() {
    let arena = Bump::new();
    let mut module = Module::new(&arena);
    build_plan_examples(&mut module);
    pretty_assertions::assert_eq!(module.to_string(), text::PLAN_EXAMPLES);
    assert_eq!(verify(&module, Profile::PreAbi), Vec::<String>::new());
}

#[test]
fn builder_covers_memory_calls_and_switches() {
    let arena = Bump::new();
    let mut module = Module::new(&arena);
    module.set_target("x86_64-pc-windows-msvc", "e-m:w");
    let word = GlobalDecl {
        linkage:  Linkage::Internal,
        size:     8,
        align:    Align::from_bytes(8).unwrap(),
        constant: false,
    };
    let cell = module.declare_global("cell", word);
    module.define_global(cell, GlobalInit::Zero);
    let pointer = module.declare_global(
        "pointer",
        GlobalDecl {
            constant: true,
            ..word
        },
    );
    module.define_global(
        pointer,
        GlobalInit::Bytes {
            bytes:       &[0; 8],
            relocations: &[Relocation {
                offset: 0,
                symbol: Symbol::Global(cell),
                addend: 4,
            }],
        },
    );
    let callback = module.intern_signature(&[Type::I64], None, false);
    let sink = module.declare_function("sink", callback, Linkage::External);
    let main_sig = module.intern_signature(&[Type::I64], Some(Type::I64), false);
    let main = module.declare_function("main", main_sig, Linkage::External);

    let mut builder = FunctionBuilder::new(&mut module, main);
    let slot = builder.create_stack_slot(16, Align::from_bytes(16).unwrap());
    let entry = builder.create_entry_block();
    let small = builder.create_block();
    let large = builder.create_block();
    let done = builder.create_block();
    builder.switch_to_block(entry);
    let input = builder.body().block_params(entry)[0];
    let frame = builder.stack_addr(slot);
    let eight = builder.iconst(Type::I64, 8);
    let field = builder.ptr_add(InstFlags::INBOUNDS, frame, eight);
    let aligned = MemFlags::aligned(Align::from_bytes(8).unwrap());
    builder.store(input, field, aligned);
    let loaded = builder.load(
        Type::I64,
        field,
        MemFlags {
            volatile: true,
            ..aligned
        },
    );
    let address = builder.global_addr(cell);
    let byte = builder.iconst(Type::I8, -1);
    builder.fill(
        InstFlags::empty(),
        [address, byte, eight],
        Align::from_bytes(8).unwrap(),
    );
    builder.copy(InstFlags::MAY_OVERLAP, [frame, address, eight], Align::BYTE);
    let callee = builder.func_addr(sink);
    let direct = builder.call(sink, &[loaded]);
    assert_eq!(builder.inst_result(direct), None);
    let indirect = builder.intern_signature(&[Type::I64], None, false);
    assert_eq!(indirect, callback, "signatures are interned");
    _ = builder.call_indirect(indirect, callee, &[input]);
    let wide = builder.iconst(Type::I128, -2);
    let narrow = builder.convert(Opcode::Trunc, Type::I64, wide);
    builder.switch(
        narrow,
        (done, &[eight]),
        &[(-2, small, &[]), (1 << 40, large, &[])],
    );
    builder.switch_to_block(small);
    builder.jump(done, &[input]);
    builder.switch_to_block(large);
    builder.unreachable();
    let result = builder.append_block_param(done, Type::I64);
    builder.switch_to_block(done);
    builder.ret(Some(result));
    builder.finish();

    let expected = "\
target triple = \"x86_64-pc-windows-msvc\"
target datalayout = \"e-m:w\"

global @cell internal size 8, align 8 = zero
global @pointer internal constant size 8, align 8 = bytes \"0000000000000000\" relocs [0: @cell + \
                    4]

function @sink(i64) external

function @main(i64) -> i64 external {
    slot0 = stack_slot 16, align 16
block0(v0: i64):
    v1 = stack_addr slot0
    v2 = iconst.i64 8
    v3 = ptr_add inbounds v1, v2
    store.i64 v0, v3, align 8
    v4 = load.i64 volatile v3, align 8
    v5 = global_addr @cell
    v6 = iconst.i8 -1
    fill v5, v6, v2, align 8
    copy may_overlap v1, v5, v2, align 1
    v7 = func_addr @sink
    call @sink(v4)
    call_indirect v7(v0) : (i64)
    v8 = iconst.i128 -2
    v9 = trunc.i64 v8
    switch v9, block3(v2), [-2: block1, 1099511627776: block2]
block1:
    jump block3(v0)
block2:
    unreachable
block3(v10: i64):
    return v10
}
";
    pretty_assertions::assert_eq!(module.to_string(), expected);
    assert_eq!(verify(&module, Profile::PostAbi), Vec::<String>::new());
    assert_round_trip(expected);
}

#[test]
fn builder_records_definitions_and_constants() {
    let arena = Bump::new();
    let mut module = Module::new(&arena);
    let signature = module.intern_signature(&[Type::F64], Some(Type::F128), true);
    let func = module.declare_function("widen", signature, Linkage::Internal);
    let mut builder = FunctionBuilder::new(&mut module, func);
    let entry = builder.create_entry_block();
    builder.switch_to_block(entry);
    let input = builder.body().block_params(entry)[0];
    let list_slot = builder.create_stack_slot(24, Align::from_bytes(8).unwrap());
    let list = builder.stack_addr(list_slot);
    let copy_slot = builder.create_stack_slot(24, Align::from_bytes(8).unwrap());
    let copy = builder.stack_addr(copy_slot);
    builder.va_start(list);
    builder.va_copy(copy, list);
    let extra = builder.va_arg(Type::F64, copy);
    builder.va_end(copy);
    builder.va_end(list);
    let sum = builder.binary(Opcode::Fadd, InstFlags::empty(), input, extra);
    let negated = builder.unary(Opcode::Fneg, sum);
    let frozen = builder.unary(Opcode::Freeze, negated);
    let is_nan = builder.fcmp(FloatCC::Uno, frozen, frozen);
    let quiet = builder.fconst(Type::F128, 0x7FFF_8000_0000_0000_0000_0000_0000_0000);
    let widened = builder.convert(Opcode::Fpext, Type::F128, frozen);
    let chosen = builder.select(is_nan, quiet, widened);
    let missing = builder.poison(Type::F128);
    let pointer = builder.null();
    _ = (missing, pointer);
    builder.ret(Some(chosen));
    let body = builder.body();
    assert_eq!(body.value_def(input), ValueDef::Param(entry, 0));
    let ValueDef::Result(inst) = body.value_def(quiet) else {
        panic!("a constant is an instruction result");
    };
    assert_eq!(body.inst_result(inst), Some(quiet));
    assert!(matches!(body.inst(inst), InstData::WideConst { .. }));
    assert_eq!(
        body.const_bits(body.inst(inst)),
        Some(0x7FFF_8000_0000_0000_0000_0000_0000_0000)
    );
    assert_eq!(body.value_type(is_nan), Type::I1);
    assert_eq!(body.inst(inst).controlling_type(), Some(Type::F128));
    builder.finish();
    assert_eq!(verify(&module, Profile::PreAbi), Vec::<String>::new());
    assert_eq!(
        verify(&module, Profile::PostAbi),
        ["@widen, inst4: va_arg is not allowed in the PostAbi profile"]
    );
    assert_round_trip(&module.to_string());
}

#[test]
fn types_and_names_are_consistent() {
    for ty in Type::ALL {
        assert_eq!(Type::from_name(ty.name()), Some(ty));
        assert_eq!(ty.to_string(), ty.name());
        assert_eq!(ty.bytes(), ty.bits().div_ceil(8));
        assert_eq!(
            u8::from(ty.is_int()) + u8::from(ty.is_float()) + u8::from(ty == Type::Ptr),
            1
        );
    }
    assert_eq!(Type::F80.bytes(), 10);
    for &opcode in Opcode::ALL {
        assert_eq!(Opcode::from_name(opcode.name()), Some(opcode));
    }
    for &cond in IntCC::ALL {
        assert_eq!(IntCC::from_name(&cond.to_string()), Some(cond));
    }
    for &cond in FloatCC::ALL {
        assert_eq!(FloatCC::from_name(cond.name()), Some(cond));
    }
    for (flag, name) in InstFlags::NAMES {
        assert_eq!(InstFlags::from_keyword(name), Some(flag));
    }
    assert_eq!(Align::from_bytes(3), None);
    assert_eq!(Align::from_bytes(1 << 33), None);
    assert_eq!(Align::from_bytes(64).map(Align::log2), Some(6));
    assert_eq!(AccessTag::new(u32::MAX), None);
    assert_eq!(Bits64::new(u64::MAX - 1).get(), u64::MAX - 1);
    assert_eq!(Linkage::from_name("internal"), Some(Linkage::Internal));
    assert_eq!(Value::new(3).to_string(), "v3");
    assert_eq!(format!("{:?}", Block::new(2)), "block2");
}
