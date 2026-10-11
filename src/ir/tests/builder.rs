//! Building function bodies.

use super::*;

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
    let body = module.function(func).body.as_ref().unwrap();
    assert_eq!(body.block_count(), 1);
    assert_eq!(body.inst_count(), 17);
    assert!(body.terminator(entry).is_some());
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
