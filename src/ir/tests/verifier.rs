//! The verifier accepts well-formed IR and reports each kind of breakage.

use super::*;

/// Asserts the pre-ABI verifier's messages for each function or global text.
fn assert_rejects(cases: &[(&str, &[&str])]) {
    for &(text, expected) in cases {
        assert_eq!(verify_text(text), expected, "for\n{text}");
    }
}

#[test]
fn structure_errors() {
    assert_rejects(&[
        (
            "function @f() external {\n}\n",
            &["@f: a defined function has no blocks"],
        ),
        (
            "function @f() external {\nblock0:\n    jump block1\nblock1:\n}\n",
            &["@f, block1: the block is empty"],
        ),
        (
            "function @f() external {\nblock0:\n    v0 = iconst.i32 1\n}\n",
            &["@f, block0: the block does not end in a terminator"],
        ),
        (
            "function @f() external {\nblock0:\n    return\n    v0 = iconst.i32 1\n}\n",
            &[
                "@f, inst0: a terminator is followed by instructions",
                "@f, block0: the block does not end in a terminator",
            ],
        ),
        (
            "function @f(i32) external {\nblock0:\n    return\n}\n",
            &["@f, block0: the entry block has 0 parameters but the signature has 1"],
        ),
        (
            "function @f(i32) external {\nblock0(v0: i64):\n    return\n}\n",
            &["@f, block0: entry parameter 0 is i64, but the signature says i32"],
        ),
        (
            "function @f() external {\nblock0:\n    jump block0\n}\n",
            &["@f, inst0: a branch targets the entry block"],
        ),
    ]);
}

#[test]
fn type_and_flag_errors() {
    assert_rejects(
        &[
            (
                "function @f(f32) external {\nblock0(v0: f32):\n    v1 = iadd.f32 v0, v0\n    \
                 return\n}\n",
                &["@f, inst0: iadd cannot have type f32"],
            ),
            (
                "function @f() external {\nblock0:\n    v0 = iconst.f32 1\n    v1 = fconst.i32 \
                 0x1\n    return\n}\n",
                &[
                    "@f, inst0: iconst cannot have type f32",
                    "@f, inst1: fconst cannot have type i32",
                ],
            ),
            (
                "function @f(i32, i64) external {\nblock0(v0: i32, v1: i64):\n    v2 = iadd.i32 \
                 v0, v1\n    return\n}\n",
                &["@f, inst0: operand 1 is i64, expected i32"],
            ),
            (
                "function @f(i32) external {\nblock0(v0: i32):\n    v1 = urem.i32 nuw exact v0, \
                 v0\n    v2 = sdiv.i32 exact v0, v0\n    return\n}\n",
                &["@f, inst0: urem cannot carry `nuw exact`"],
            ),
            (
                "function @f(ptr, i64) external {\nblock0(v0: ptr, v1: i64):\n    v2 = ptr_add \
                 nsw v0, v1\n    store.i64 inbounds v1, v2, align 8\n    return\n}\n",
                &[
                    "@f, inst0: ptr_add cannot carry `nsw`",
                    "@f, inst1: store cannot carry `inbounds`",
                ],
            ),
            (
                "function @f(i32) external {\nblock0(v0: i32):\n    v1 = zext.i8 v0\n    v2 = \
                 bitcast.ptr v0\n    return\n}\n",
                &[
                    "@f, inst0: zext cannot convert i32 to i8",
                    "@f, inst1: bitcast cannot convert i32 to ptr",
                ],
            ),
            (
                "function @f(ptr, i32) external {\nblock0(v0: ptr, v1: i32):\n    fill v0, v1, \
                 v1, align 1\n    return\n}\n",
                &[
                    "@f, inst0: operand 1 is i32, expected i8",
                    "@f, inst0: operand 2 is i32, expected i64",
                ],
            ),
            (
                "function @f(i32) -> i32 external {\nblock0(v0: i32):\n    return\n}\n",
                &["@f, inst0: the function returns i32, but the return gives nothing"],
            ),
        ],
    );
}

#[test]
fn edge_errors() {
    assert_rejects(&[
        (
            "function @f(i32) external {\nblock0(v0: i32):\n    jump block1(v0)\nblock1:\n    \
             return\n}\n",
            &["@f, inst0: the edge to block1 passes 1 arguments for 0 parameters"],
        ),
        (
            "function @f(i32) external {\nblock0(v0: i32):\n    jump block1(v0)\nblock1(v1: \
             i64):\n    return\n}\n",
            &["@f, inst0: argument 0 to block1 is i32, expected i64"],
        ),
        (
            "function @f(i1, i32, i32) external {\nblock0(v0: i1, v1: i32, v2: i32):\n    brif \
             v0, block1(v1), block1(v2)\nblock1(v3: i32):\n    return\n}\n",
            &["@f, inst0: edges to block1 pass different arguments"],
        ),
        (
            "function @f(i32) external {\nblock0(v0: i32):\n    switch v0, block1, [1: block1, 1: \
             block2]\nblock1:\n    return\nblock2:\n    return\n}\n",
            &["@f, inst0: the case 1 appears twice"],
        ),
        (
            "function @f(i32) external {\nblock0(v0: i32):\n    brif v0, block1, \
             block1\nblock1:\n    return\n}\n",
            &["@f, inst0: operand 0 is i32, expected i1"],
        ),
    ]);
}

#[test]
fn call_errors() {
    assert_rejects(&[
        (
            "function @g() external\n\nfunction @f(i32) external {\nblock0(v0: i32):\n    call \
             @g(v0)\n    return\n}\n",
            &["@f, inst0: the call passes 1 arguments for 0 parameters"],
        ),
        (
            "function @g(ptr, ...) external\n\nfunction @f(i32) external {\nblock0(v0: i32):\n    \
             call @g()\n    call @g(v0, v0)\n    return\n}\n",
            &[
                "@f, inst0: the call passes 0 arguments for at least 1 parameters",
                "@f, inst1: call argument 0 is i32, expected ptr",
            ],
        ),
        (
            "function @f(i32) external {\nblock0(v0: i32):\n    call_indirect v0() : ()\n    \
             return\n}\n",
            &["@f, inst0: operand 0 is i32, expected ptr"],
        ),
    ]);
}

#[test]
fn dominance_errors() {
    assert_rejects(
        &[
            (
                "function @f(i1) -> i32 external {\nblock0(v0: i1):\n    brif v0, block1, \
                 block2\nblock1:\n    v1 = iconst.i32 1\n    jump block2\nblock2:\n    return \
                 v1\n}\n",
                &["@f, inst3: the use of v1 is not dominated by its definition"],
            ),
            (
                "function @f() -> i32 external {\nblock0:\n    v1 = iadd.i32 v0, v0\n    v0 = \
                 iconst.i32 1\n    return v1\n}\n",
                &["@f, inst0: the use of v0 is not dominated by its definition"],
            ),
            (
                "function @f(i1) external {\nblock0(v0: i1):\n    brif v0, block1, \
                 block2\nblock1:\n    jump block2\nblock2:\n    v1 = iconst.i32 1\n    jump \
                 block3(v1)\nblock3(v2: i32):\n    return\nblock4:\n    v3 = iadd.i32 v2, v2\n    \
                 jump block3(v3)\n}\n",
                &[],
            ),
        ],
    );
}

#[test]
fn global_errors() {
    assert_rejects(&[
        (
            "global @g internal size 4, align 4 = bytes \"00\"\n",
            &["@g: the initializer has 1 bytes for a size of 4"],
        ),
        (
            "global @g internal size 8, align 8 = bytes \"0000000000000000\" relocs [4: @g]\n",
            &["@g: the relocation at offset 4 does not fit in the global"],
        ),
    ]);
}

/// Builds `@f` with the signature `(i32) -> i32`, lets `edit` insert
/// instructions after the entry block is current, and returns the
/// verifier's messages.
fn build_and_verify(edit: impl FnOnce(&mut FunctionBuilder<'_, '_>, Value)) -> Vec<String> {
    let arena = Bump::new();
    let mut module = Module::new(&arena);
    let signature = module.intern_signature(&[Type::I32], Some(Type::I32), false);
    let func = module.declare_function("f", signature, Linkage::External);
    let mut builder = FunctionBuilder::new(&mut module, func);
    let entry = builder.create_entry_block();
    builder.switch_to_block(entry);
    let param = builder.body().block_params(entry)[0];
    edit(&mut builder, param);
    builder.finish();
    let scratch = Bump::new();
    verify_function(&module, func, Profile::PreAbi, &scratch)
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn errors_only_broken_records_can_have() {
    assert_eq!(
        build_and_verify(|builder, _| builder.ret(Some(Value::new(99)))),
        ["@f, inst0: value 99 does not exist"]
    );
    assert_eq!(
        build_and_verify(|builder, param| {
            let address = builder.stack_addr(StackSlot::new(5));
            let global = builder.global_addr(GlobalId::new(1));
            let func = builder.func_addr(FuncId::new(7));
            _ = (address, global, func);
            builder.ret(Some(param));
        }),
        [
            "@f, inst0: stack slot 5 does not exist",
            "@f, inst1: global 1 does not exist",
            "@f, inst2: function 7 does not exist",
        ]
    );
    assert_eq!(
        build_and_verify(|builder, param| {
            let data = builder.constant(Opcode::Iconst, Type::I8, 0x1FF);
            _ = builder.insert(data);
            builder.ret(Some(param));
        }),
        ["@f, inst0: a constant does not fit in i8"]
    );
    assert_eq!(
        build_and_verify(|builder, param| {
            let constant = builder.iconst(Type::I32, 1);
            builder.body.values[constant].ty = Type::I64;
            builder.ret(Some(param));
        }),
        ["@f, inst0: the result is i64, expected i32"]
    );
    assert_eq!(
        build_and_verify(|builder, param| {
            builder.body.values[param].def = ValueDef::Param(Block::new(0), 7);
            builder.ret(Some(param));
        }),
        ["@f: v0 is not defined where the value table says"]
    );
    assert_eq!(
        build_and_verify(|builder, param| {
            builder.ret(Some(param));
            let inst = builder.body().block_insts(Block::new(0))[0];
            builder.body.blocks[Block::new(0)].insts.push(inst);
        }),
        [
            "@f, inst0: a terminator is followed by instructions",
            "@f, inst0: the instruction appears more than once",
        ]
    );
    assert_eq!(
        build_and_verify(|builder, param| {
            let sig = builder.intern_signature(&[], Some(Type::I32), false);
            let args = builder.value_list(&[]);
            let inst = builder.insert(InstData::CallIndirect { sig, args });
            assert!(builder.inst_result(inst).is_some());
            builder.ret(Some(param));
        }),
        ["@f, inst0: call_indirect has no callee"]
    );
    assert_eq!(
        build_and_verify(|builder, param| {
            let target = builder.create_block();
            let narrow = builder.iconst(Type::I8, 3);
            let constant = builder.wide_constant(0x1FF);
            let call = builder.block_call(target, &[]);
            let cases = builder.case_list(&[(constant, call)]);
            _ = builder.insert(InstData::Switch {
                value: narrow,
                default: call,
                cases,
            });
            builder.switch_to_block(target);
            builder.ret(Some(param));
        }),
        ["@f, inst1: a constant does not fit in i8"]
    );
}

#[test]
fn errors_are_structured() {
    let arena = Bump::new();
    let module = parse(
        &arena,
        "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = zext.i8 v0
    return          v0
}
",
    );
    let scratch = Bump::new();
    let errors = verify_module(&module, Profile::PreAbi, &scratch);
    let expected: &[VerifierError<'_>] = &[VerifierError {
        item:     "f",
        location: Location::Inst(Inst::new(0)),
        kind:     VerifierErrorKind::Conversion {
            opcode: Opcode::Zext,
            from:   Type::I32,
            to:     Type::I8,
        },
    }];
    assert_eq!(&errors[..], expected);
    assert_eq!(
        VerifierErrorKind::InvalidReference(EntityKind::Value, 3).to_string(),
        "value 3 does not exist"
    );
}
