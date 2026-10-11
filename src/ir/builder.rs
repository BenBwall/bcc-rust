//! Building a function body one instruction at a time.
//!
//! A [`FunctionBuilder`] owns the body while it grows and borrows the module
//! for signatures, so a call's result type is known as the call is inserted.
//! [`FunctionBuilder::finish`] installs the body in the module. The builder
//! does not check what it is given; the verifier does.

use super::{
    FuncId,
    Module,
    entities::{
        Block,
        ConstId,
        Entity,
        GlobalId,
        Inst,
        SigId,
        StackSlot,
        Value,
    },
    function::{
        BlockData,
        Body,
        StackSlotData,
        ValueData,
        ValueDef,
    },
    instructions::{
        Align,
        Bits64,
        BlockCall,
        CaseList,
        FloatCC,
        InstData,
        InstFlags,
        IntCC,
        MemFlags,
        Opcode,
        ValueList,
    },
    types::Type,
};

/// Builds the body of one function; see the module docs.
#[derive(Debug)]
pub(crate) struct FunctionBuilder<'m, 'ir> {
    module:          &'m mut Module<'ir>,
    func:            FuncId,
    pub(super) body: Body<'ir>,
    current:         Option<Block>,
}

impl FunctionBuilder<'_, '_> {
    /// Inserts an instruction at the end of the current block, creating its
    /// result value if it has one. Panics if no block is current.
    pub(crate) fn insert(&mut self, data: InstData) -> Inst {
        let result = self.module.result_type(&data).map(|_| self.reserve_value());
        self.insert_with_result(data, result)
    }

    /// Inserts an instruction whose result value already exists, as the
    /// textual parser does to keep the text's value numbers.
    pub(super) fn insert_with_result(&mut self, data: InstData, result: Option<Value>) -> Inst {
        let block = self
            .current
            .expect("switch to a block before inserting instructions");
        let inst = self.body.insts.push(data);
        _ = self.body.results.push(result);
        if let Some(value) = result {
            self.body.values[value] = ValueData {
                ty:  self.module.result_type(&data).unwrap_or(Type::Ptr),
                def: ValueDef::Result(inst),
            };
        }
        self.body.blocks[block].insts.push(inst);
        inst
    }

    /// Like [`FunctionBuilder::insert`], returning the result value. Panics
    /// if the instruction has none.
    fn insert_value(&mut self, data: InstData) -> Value {
        let inst = self.insert(data);
        self.body.results[inst].expect("the instruction defines a value")
    }

    pub(crate) fn create_block(&mut self) -> Block {
        self.body
            .blocks
            .push(BlockData::new_in(self.module.arena()))
    }

    /// Creates a block with one parameter per parameter of the function's
    /// signature; it is the entry block if it is the first block.
    pub(crate) fn create_entry_block(&mut self) -> Block {
        let block = self.create_block();
        let signature = self.module.function_signature(self.func);
        for &ty in signature.params {
            _ = self.append_block_param(block, ty);
        }
        block
    }

    pub(crate) fn append_block_param(&mut self, block: Block, ty: Type) -> Value {
        let value = self.reserve_value();
        self.define_block_param(block, value, ty);
        value
    }

    /// Makes `block` the block that instructions are appended to.
    pub(crate) fn switch_to_block(&mut self, block: Block) {
        self.current = Some(block);
    }

    pub(crate) fn current_block(&self) -> Option<Block> {
        self.current
    }

    pub(crate) fn create_stack_slot(&mut self, size: u32, align: Align) -> StackSlot {
        self.body.stack_slots.push(StackSlotData { size, align })
    }

    // Pool entries

    pub(crate) fn value_list(&mut self, values: &[Value]) -> ValueList {
        let start = self.pool_start();
        self.body.pool.push(Value::new(values.len()));
        self.body.pool.extend_from_slice(values);
        ValueList(start)
    }

    pub(crate) fn block_call(&mut self, block: Block, args: &[Value]) -> BlockCall {
        let start = self.pool_start();
        self.body.pool.push(Value::new(args.len()));
        self.body.pool.push(Value::new(block.index()));
        self.body.pool.extend_from_slice(args);
        BlockCall(start)
    }

    pub(crate) fn case_list(&mut self, cases: &[(ConstId, BlockCall)]) -> CaseList {
        let start = self.pool_start();
        self.body.pool.push(Value::new(cases.len()));
        for &(constant, call) in cases {
            self.body.pool.push(Value::new(constant.index()));
            self.body.pool.push(Value::new(call.0 as usize));
        }
        CaseList(start)
    }

    /// Adds bits to the function's constant pool.
    pub(crate) fn wide_constant(&mut self, bits: u128) -> ConstId {
        self.body.constants.push(bits)
    }

    fn pool_start(&self) -> u32 {
        u32::try_from(self.body.pool.len()).expect("the value pool exceeds u32 indices")
    }

    // Arithmetic and logic

    /// An integer or floating binary operation; the type is `a`'s.
    pub(crate) fn binary(&mut self, opcode: Opcode, flags: InstFlags, a: Value, b: Value) -> Value {
        debug_assert!(
            opcode.is_int_binary() || opcode.is_float_binary(),
            "{opcode} is not a binary operation"
        );
        let ty = self.body.values[a].ty;
        self.insert_value(InstData::Binary {
            opcode,
            ty,
            flags,
            args: [a, b],
        })
    }

    /// `fneg` or `freeze`; the type is the operand's.
    pub(crate) fn unary(&mut self, opcode: Opcode, value: Value) -> Value {
        debug_assert!(
            matches!(opcode, Opcode::Fneg | Opcode::Freeze),
            "{opcode} is not a unary operation"
        );
        let ty = self.body.values[value].ty;
        self.insert_value(InstData::Unary {
            opcode,
            ty,
            arg: value,
        })
    }

    /// A conversion of `value` to `to`.
    pub(crate) fn convert(&mut self, opcode: Opcode, to: Type, value: Value) -> Value {
        debug_assert!(opcode.is_conversion(), "{opcode} is not a conversion");
        self.insert_value(InstData::Unary {
            opcode,
            ty: to,
            arg: value,
        })
    }

    pub(crate) fn icmp(&mut self, cond: IntCC, a: Value, b: Value) -> Value {
        let ty = self.body.values[a].ty;
        self.insert_value(InstData::IntCompare {
            cond,
            ty,
            args: [a, b],
        })
    }

    pub(crate) fn fcmp(&mut self, cond: FloatCC, a: Value, b: Value) -> Value {
        let ty = self.body.values[a].ty;
        self.insert_value(InstData::FloatCompare {
            cond,
            ty,
            args: [a, b],
        })
    }

    /// `cond ? a : b`; the type is `a`'s.
    pub(crate) fn select(&mut self, cond: Value, a: Value, b: Value) -> Value {
        let ty = self.body.values[a].ty;
        self.insert_value(InstData::Select {
            ty,
            args: [cond, a, b],
        })
    }

    // Constants

    /// An integer constant, truncated to the type's width.
    pub(crate) fn iconst(&mut self, ty: Type, value: i128) -> Value {
        let data = self.constant(Opcode::Iconst, ty, value as u128 & ty.mask());
        self.insert_value(data)
    }

    /// A floating constant given by its bits.
    pub(crate) fn fconst(&mut self, ty: Type, bits: u128) -> Value {
        let data = self.constant(Opcode::Fconst, ty, bits);
        self.insert_value(data)
    }

    /// The record of an `iconst` or `fconst`, putting bits wider than 64 in
    /// the constant pool.
    pub(super) fn constant(&mut self, opcode: Opcode, ty: Type, bits: u128) -> InstData {
        match u64::try_from(bits) {
            | Ok(narrow) if ty.bits() <= 64 => InstData::Const {
                opcode,
                ty,
                bits: Bits64::new(narrow),
            },
            | _ => InstData::WideConst {
                opcode,
                ty,
                constant: self.wide_constant(bits),
            },
        }
    }

    pub(crate) fn poison(&mut self, ty: Type) -> Value {
        self.insert_value(InstData::Nullary {
            opcode: Opcode::Poison,
            ty,
        })
    }

    /// The null pointer.
    pub(crate) fn null(&mut self) -> Value {
        self.insert_value(InstData::Nullary {
            opcode: Opcode::Null,
            ty:     Type::Ptr,
        })
    }

    // Addresses and memory

    pub(crate) fn stack_addr(&mut self, slot: StackSlot) -> Value {
        self.insert_value(InstData::StackAddr { slot })
    }

    pub(crate) fn global_addr(&mut self, global: GlobalId) -> Value {
        self.insert_value(InstData::GlobalAddr { global })
    }

    pub(crate) fn func_addr(&mut self, func: FuncId) -> Value {
        self.insert_value(InstData::FuncAddr { func })
    }

    /// `base` plus the `i64` byte `offset`.
    pub(crate) fn ptr_add(&mut self, flags: InstFlags, base: Value, offset: Value) -> Value {
        self.insert_value(InstData::Binary {
            opcode: Opcode::PtrAdd,
            ty: Type::Ptr,
            flags,
            args: [base, offset],
        })
    }

    pub(crate) fn load(&mut self, ty: Type, addr: Value, mem: MemFlags) -> Value {
        self.insert_value(InstData::Load {
            ty,
            flags: volatile(mem),
            align: mem.align,
            tag: mem.tag,
            addr,
        })
    }

    /// Stores `value` at `addr`; the type is the value's.
    pub(crate) fn store(&mut self, value: Value, addr: Value, mem: MemFlags) {
        let ty = self.body.values[value].ty;
        _ = self.insert(InstData::Store {
            ty,
            flags: volatile(mem),
            align: mem.align,
            tag: mem.tag,
            args: [value, addr],
        });
    }

    /// Copies `size` bytes (an `i64`) from `src` to `dst`, as `memcpy` or,
    /// with `may_overlap`, `memmove`.
    pub(crate) fn copy(&mut self, flags: InstFlags, [dst, src, size]: [Value; 3], align: Align) {
        _ = self.insert(InstData::MemoryRange {
            opcode: Opcode::Copy,
            flags,
            align,
            args: [dst, src, size],
        });
    }

    /// Sets `size` bytes (an `i64`) at `dst` to `byte` (an `i8`), as `memset`.
    pub(crate) fn fill(&mut self, flags: InstFlags, [dst, byte, size]: [Value; 3], align: Align) {
        _ = self.insert(InstData::MemoryRange {
            opcode: Opcode::Fill,
            flags,
            align,
            args: [dst, byte, size],
        });
    }

    // Calls and variable arguments

    /// A direct call; [`FunctionBuilder::inst_result`] gives its result.
    pub(crate) fn call(&mut self, func: FuncId, args: &[Value]) -> Inst {
        let args = self.value_list(args);
        self.insert(InstData::Call { func, args })
    }

    /// A call through the pointer `callee`.
    pub(crate) fn call_indirect(&mut self, sig: SigId, callee: Value, args: &[Value]) -> Inst {
        let start = self.pool_start();
        self.body.pool.push(Value::new(args.len() + 1));
        self.body.pool.push(callee);
        self.body.pool.extend_from_slice(args);
        self.insert(InstData::CallIndirect {
            sig,
            args: ValueList(start),
        })
    }

    pub(crate) fn inst_result(&self, inst: Inst) -> Option<Value> {
        self.body.results[inst]
    }

    /// Initializes the `va_list` at `list` for the current function's extra
    /// arguments.
    pub(crate) fn va_start(&mut self, list: Value) {
        _ = self.insert(InstData::Unary {
            opcode: Opcode::VaStart,
            ty:     Type::Ptr,
            arg:    list,
        });
    }

    /// The next extra argument, of type `ty`, from the `va_list` at `list`.
    pub(crate) fn va_arg(&mut self, ty: Type, list: Value) -> Value {
        self.insert_value(InstData::Unary {
            opcode: Opcode::VaArg,
            ty,
            arg: list,
        })
    }

    pub(crate) fn va_copy(&mut self, dst: Value, src: Value) {
        _ = self.insert(InstData::Binary {
            opcode: Opcode::VaCopy,
            ty:     Type::Ptr,
            flags:  InstFlags::empty(),
            args:   [dst, src],
        });
    }

    pub(crate) fn va_end(&mut self, list: Value) {
        _ = self.insert(InstData::Unary {
            opcode: Opcode::VaEnd,
            ty:     Type::Ptr,
            arg:    list,
        });
    }

    // Terminators

    pub(crate) fn jump(&mut self, block: Block, args: &[Value]) {
        let dest = self.block_call(block, args);
        _ = self.insert(InstData::Jump { dest });
    }

    /// Branches to `then` if `cond` is 1, else to `otherwise`.
    pub(crate) fn brif(
        &mut self,
        cond: Value,
        (then, then_args): (Block, &[Value]),
        (otherwise, otherwise_args): (Block, &[Value]),
    ) {
        let dests = [
            self.block_call(then, then_args),
            self.block_call(otherwise, otherwise_args),
        ];
        _ = self.insert(InstData::Brif { cond, dests });
    }

    /// Branches on `value` to the edge of the case equal to it, else to the
    /// default. Case values are truncated to `value`'s type.
    pub(crate) fn switch(
        &mut self,
        value: Value,
        (default, default_args): (Block, &[Value]),
        cases: &[(i128, Block, &[Value])],
    ) {
        let mask = self.body.values[value].ty.mask();
        let default = self.block_call(default, default_args);
        // The edges go first, one after another, so the list that follows
        // can name each by its position.
        let mut call = self.pool_start();
        for &(_, block, args) in cases {
            _ = self.block_call(block, args);
        }
        let start = self.pool_start();
        self.body.pool.push(Value::new(cases.len()));
        for &(case, _, args) in cases {
            let constant = self.wide_constant(case as u128 & mask);
            self.body.pool.push(Value::new(constant.index()));
            self.body.pool.push(Value::new(call as usize));
            call += u32::try_from(2 + args.len()).expect("the value pool exceeds u32 indices");
        }
        _ = self.insert(InstData::Switch {
            value,
            default,
            cases: CaseList(start),
        });
    }

    pub(crate) fn ret(&mut self, value: Option<Value>) {
        _ = self.insert(InstData::Return { value });
    }

    pub(crate) fn unreachable(&mut self) {
        _ = self.insert(InstData::Unreachable);
    }
}

fn volatile(mem: MemFlags) -> InstFlags {
    if mem.volatile {
        InstFlags::VOLATILE
    } else {
        InstFlags::empty()
    }
}

impl<'m, 'ir> FunctionBuilder<'m, 'ir> {
    /// Starts an empty body for `func`, replacing any body it has once
    /// finished.
    pub(crate) fn new(module: &'m mut Module<'ir>, func: FuncId) -> Self {
        let body = Body::new_in(module.arena());
        Self {
            module,
            func,
            body,
            current: None,
        }
    }

    /// Installs the body in the module.
    pub(crate) fn finish(self) {
        self.module.function_mut(self.func).body = Some(self.body);
    }

    pub(crate) fn module(&self) -> &Module<'ir> {
        self.module
    }

    /// Interns a signature, as for `call_indirect`.
    pub(crate) fn intern_signature(
        &mut self,
        params: &[Type],
        result: Option<Type>,
        variadic: bool,
    ) -> SigId {
        self.module.intern_signature(params, result, variadic)
    }

    pub(crate) fn body(&self) -> &Body<'ir> {
        &self.body
    }

    /// A value whose definition comes later, through
    /// [`FunctionBuilder::insert_with_result`] or
    /// [`FunctionBuilder::define_block_param`].
    pub(super) fn reserve_value(&mut self) -> Value {
        self.body.values.push(ValueData {
            ty:  Type::Ptr,
            def: ValueDef::Param(Block::new(0), u32::MAX),
        })
    }

    /// Appends a reserved value to `block`'s parameters.
    pub(super) fn define_block_param(&mut self, block: Block, value: Value, ty: Type) {
        let params = &mut self.body.blocks[block].params;
        let index = u32::try_from(params.len()).expect("too many block parameters");
        params.push(value);
        self.body.values[value] = ValueData {
            ty,
            def: ValueDef::Param(block, index),
        };
    }
}
