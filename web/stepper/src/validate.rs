//! Validation (WebAssembly 2.0 §3), following the algorithm of the spec's
//! appendix (§7.3): each function body is checked against a stack of
//! operand types and a stack of control frames, so execution never meets an
//! operand of the wrong type, a branch to a missing label or an unknown
//! index. The module's other parts (constant expressions, segments,
//! exports, the start function) are checked too.
//!
//! One rule is relaxed: `ref.func` may name any function, not only those an
//! element segment or export declares.

use crate::{
    instr::{
        Instr,
        numeric_signature,
        trunc_sat_signature,
    },
    module::{
        BlockType,
        ConstExpr,
        ExternKind,
        FuncKind,
        FuncType,
        GlobalType,
        ImportKind,
        Module,
        SegmentMode,
        TableType,
        ValType,
    },
};

/// Check `module`.
///
/// # Errors
/// The first rule the module breaks, naming the function and instruction.
pub fn validate(module: &Module) -> Result<(), String> {
    let context = Context::new(module);
    context.check_module()?;
    for (index, func) in module.funcs.iter().enumerate() {
        let FuncKind::Defined(body) = &func.kind else {
            continue;
        };
        let Some(ty) = module.types.get(func.ty as usize) else {
            return Err(format!("function {index} has an unknown type"));
        };
        let mut locals = ty.params.clone();
        locals.extend(&body.locals);
        let mut checker = Checker {
            context: &context,
            locals,
            results: ty.results.clone(),
            values: Vec::new(),
            controls: Vec::new(),
        };
        checker.push_control(Kind::Function, Vec::new(), ty.results.clone());
        for (pc, instr) in body.code.iter().enumerate() {
            checker.instruction(instr).map_err(|message| {
                format!(
                    "{} (in {}, at byte {:#x})",
                    message,
                    module.func_name(index as u32),
                    body.offsets[pc]
                )
            })?;
        }
        if !checker.controls.is_empty() {
            return Err(format!(
                "{} does not end every block it opens",
                module.func_name(index as u32)
            ));
        }
    }
    Ok(())
}

/// The index spaces a function body refers to.
struct Context<'m> {
    module:   &'m Module,
    globals:  Vec<GlobalType>,
    tables:   Vec<TableType>,
    memories: usize,
}

impl<'m> Context<'m> {
    fn new(module: &'m Module) -> Self {
        let mut globals = Vec::new();
        let mut tables = Vec::new();
        let mut memories = module.memories.len();
        for import in &module.imports {
            match import.kind {
                | ImportKind::Global(ty) => globals.push(ty),
                | ImportKind::Table(ty) => tables.push(ty),
                | ImportKind::Memory(_) => memories += 1,
                | ImportKind::Func(_) => {},
            }
        }
        globals.extend(module.globals.iter().map(|global| global.ty));
        tables.extend(module.tables.iter().copied());
        Self {
            module,
            globals,
            tables,
            memories,
        }
    }

    fn check_module(&self) -> Result<(), String> {
        let module = self.module;
        for func in &module.funcs {
            if module.types.get(func.ty as usize).is_none() {
                return Err("a function refers to an unknown type".into());
            }
        }
        let limits = module
            .memories
            .iter()
            .copied()
            .chain(
                module
                    .imports
                    .iter()
                    .filter_map(|import| match import.kind {
                        | ImportKind::Memory(limits) => Some(limits),
                        | _ => None,
                    }),
            );
        for limits in limits {
            if limits.min > 65_536
                || limits
                    .max
                    .is_some_and(|max| max > 65_536 || max < limits.min)
            {
                return Err("a memory's limits are out of range".into());
            }
        }
        let imported_globals = self.globals.len() - module.globals.len();
        for (index, global) in module.globals.iter().enumerate() {
            // A global's initializer may read only the globals before it.
            self.constant(&global.init, global.ty.value, imported_globals + index)
                .map_err(|message| format!("global {}: {message}", imported_globals + index))?;
        }
        for (index, element) in module.elements.iter().enumerate() {
            for item in &element.items {
                self.constant(item, element.ty, self.globals.len())
                    .map_err(|message| format!("element segment {index}: {message}"))?;
            }
            if let SegmentMode::Active {
                index: table,
                offset,
            } = &element.mode
            {
                let Some(table) = self.tables.get(*table as usize) else {
                    return Err(format!("element segment {index} names an unknown table"));
                };
                if table.element != element.ty {
                    return Err(format!("element segment {index} has the wrong type"));
                }
                self.constant(offset, ValType::I32, self.globals.len())
                    .map_err(|message| format!("element segment {index}: {message}"))?;
            }
        }
        for (index, data) in module.data.iter().enumerate() {
            if let SegmentMode::Active {
                index: memory,
                offset,
            } = &data.mode
            {
                if *memory as usize >= self.memories {
                    return Err(format!("data segment {index} names an unknown memory"));
                }
                self.constant(offset, ValType::I32, self.globals.len())
                    .map_err(|message| format!("data segment {index}: {message}"))?;
            }
        }
        for export in &module.exports {
            let known = match export.kind {
                | ExternKind::Func => (export.index as usize) < module.funcs.len(),
                | ExternKind::Table => (export.index as usize) < self.tables.len(),
                | ExternKind::Memory => (export.index as usize) < self.memories,
                | ExternKind::Global => (export.index as usize) < self.globals.len(),
            };
            if !known {
                return Err(format!("export \"{}\" names an unknown item", export.name));
            }
        }
        if let Some(start) = module.start {
            match module.func_type(start) {
                | Some(ty) if ty.params.is_empty() && ty.results.is_empty() => {},
                | Some(_) => return Err("the start function must take and return nothing".into()),
                | None => return Err("the start function does not exist".into()),
            }
        }
        Ok(())
    }

    /// Check a constant expression of type `ty` that may read the first
    /// `globals` globals (immutable ones only).
    fn constant(&self, expr: &ConstExpr, ty: ValType, globals: usize) -> Result<(), String> {
        let mut stack = Vec::new();
        for instr in &expr.code {
            match instr {
                | Instr::I32Const(_) => stack.push(ValType::I32),
                | Instr::I64Const(_) => stack.push(ValType::I64),
                | Instr::F32Const(_) => stack.push(ValType::F32),
                | Instr::F64Const(_) => stack.push(ValType::F64),
                | Instr::RefNull(ty) => stack.push(*ty),
                | Instr::RefFunc(func) => {
                    if *func as usize >= self.module.funcs.len() {
                        return Err("ref.func names an unknown function".into());
                    }
                    stack.push(ValType::FuncRef);
                },
                | Instr::GlobalGet(global) => {
                    let Some(global) = self
                        .globals
                        .get(*global as usize)
                        .filter(|_| (*global as usize) < globals)
                    else {
                        return Err("a constant reads an unknown global".into());
                    };
                    if global.mutable {
                        return Err("a constant reads a mutable global".into());
                    }
                    stack.push(global.value);
                },
                // Extended constant expressions.
                | Instr::Numeric(op @ (0x6A..=0x6C | 0x7C..=0x7E)) => {
                    let operand = if *op < 0x79 {
                        ValType::I32
                    } else {
                        ValType::I64
                    };
                    if stack.pop() != Some(operand) || stack.pop() != Some(operand) {
                        return Err("a constant's arithmetic has the wrong operands".into());
                    }
                    stack.push(operand);
                },
                | Instr::End => break,
                | _ => return Err("a constant expression uses a non-constant instruction".into()),
            }
        }
        if stack == [ty] {
            Ok(())
        } else {
            Err(format!(
                "a constant expression must produce one {}",
                ty.name()
            ))
        }
    }

    fn global(&self, index: u32) -> Result<GlobalType, String> {
        self.globals
            .get(index as usize)
            .copied()
            .ok_or_else(|| format!("global {index} does not exist"))
    }

    fn table(&self, index: u32) -> Result<TableType, String> {
        self.tables
            .get(index as usize)
            .copied()
            .ok_or_else(|| format!("table {index} does not exist"))
    }

    fn func_type(&self, index: u32) -> Result<&FuncType, String> {
        self.module
            .func_type(index)
            .ok_or_else(|| format!("function {index} does not exist"))
    }

    fn type_(&self, index: u32) -> Result<&FuncType, String> {
        self.module
            .types
            .get(index as usize)
            .ok_or_else(|| format!("type {index} does not exist"))
    }

    fn memory(&self) -> Result<(), String> {
        if self.memories == 0 {
            Err("the module has no memory".into())
        } else {
            Ok(())
        }
    }

    fn block_type(&self, ty: BlockType) -> Result<(Vec<ValType>, Vec<ValType>), String> {
        Ok(match ty {
            | BlockType::Empty => (Vec::new(), Vec::new()),
            | BlockType::Value(value) => (Vec::new(), vec![value]),
            | BlockType::Type(index) => {
                let ty = self.type_(index)?;
                (ty.params.clone(), ty.results.clone())
            },
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Function,
    Block,
    Loop,
    If,
    Else,
}

struct Control {
    kind:        Kind,
    start:       Vec<ValType>,
    end:         Vec<ValType>,
    height:      usize,
    unreachable: bool,
}

/// An operand's type, or `None` for an unknown type below an unconditional
/// branch.
type Operand = Option<ValType>;

struct Checker<'c, 'm> {
    context:  &'c Context<'m>,
    locals:   Vec<ValType>,
    results:  Vec<ValType>,
    values:   Vec<Operand>,
    controls: Vec<Control>,
}

impl Checker<'_, '_> {
    #[expect(clippy::too_many_lines, reason = "One rule per instruction family.")]
    fn instruction(&mut self, instr: &Instr) -> Result<(), String> {
        use ValType::{
            F32,
            F64,
            FuncRef,
            I32,
            I64,
        };
        match instr {
            | Instr::Unreachable => self.unreachable(),
            | Instr::Nop => {},
            | Instr::Block { ty, .. } | Instr::Loop { ty, .. } => {
                let (params, results) = self.context.block_type(*ty)?;
                self.pop_all(&params)?;
                let kind = if matches!(instr, Instr::Loop { .. }) {
                    Kind::Loop
                } else {
                    Kind::Block
                };
                self.push_control(kind, params, results);
            },
            | Instr::If { ty, .. } => {
                self.pop_expect(I32)?;
                let (params, results) = self.context.block_type(*ty)?;
                self.pop_all(&params)?;
                self.push_control(Kind::If, params, results);
            },
            | Instr::Else { .. } => {
                let control = self.pop_control()?;
                if control.kind != Kind::If {
                    return Err("else without a matching if".into());
                }
                self.push_control(Kind::Else, control.start, control.end);
            },
            | Instr::End => {
                let control = self.pop_control()?;
                // An if without else must leave its parameters as results.
                if control.kind == Kind::If && control.start != control.end {
                    return Err("an if without an else must not change the stack's types".into());
                }
                self.push_all(&control.end);
            },
            | Instr::Br(depth) => {
                let types = self.label_types(*depth)?;
                self.pop_all(&types)?;
                self.unreachable();
            },
            | Instr::BrIf(depth) => {
                self.pop_expect(I32)?;
                let types = self.label_types(*depth)?;
                self.pop_all(&types)?;
                self.push_all(&types);
            },
            | Instr::BrTable { targets, default } => {
                self.pop_expect(I32)?;
                let arity = self.label_types(*default)?.len();
                for target in targets.iter().copied().chain([*default]) {
                    let types = self.label_types(target)?;
                    if types.len() != arity {
                        return Err("br_table's labels take different numbers of values".into());
                    }
                    // Each label's types must match the same operands.
                    let popped = self.pop_all(&types)?;
                    self.values.extend(popped);
                }
                let types = self.label_types(*default)?;
                self.pop_all(&types)?;
                self.unreachable();
            },
            | Instr::Return => {
                let results = self.results.clone();
                self.pop_all(&results)?;
                self.unreachable();
            },
            | Instr::Call(func) => {
                let ty = self.context.func_type(*func)?.clone();
                self.pop_all(&ty.params)?;
                self.push_all(&ty.results);
            },
            | Instr::CallIndirect { ty, table } => {
                if self.context.table(*table)?.element != FuncRef {
                    return Err("call_indirect needs a funcref table".into());
                }
                self.pop_expect(I32)?;
                let ty = self.context.type_(*ty)?.clone();
                self.pop_all(&ty.params)?;
                self.push_all(&ty.results);
            },
            | Instr::Drop => {
                self.pop()?;
            },
            | Instr::Select(None) => {
                self.pop_expect(I32)?;
                let first = self.pop()?;
                let second = self.pop()?;
                if first.is_some_and(|ty| !ty.is_numeric())
                    || second.is_some_and(|ty| !ty.is_numeric())
                {
                    return Err("select without a type needs numeric operands".into());
                }
                match (first, second) {
                    | (Some(a), Some(b)) if a != b =>
                        return Err("select's operands have different types".into()),
                    | (Some(ty), _) | (_, Some(ty)) => self.push(Some(ty)),
                    | (None, None) => self.push(None),
                }
            },
            | Instr::Select(Some(ty)) => {
                self.pop_expect(I32)?;
                self.pop_expect(*ty)?;
                self.pop_expect(*ty)?;
                self.push(Some(*ty));
            },
            | Instr::LocalGet(index) => {
                let ty = self.local(*index)?;
                self.push(Some(ty));
            },
            | Instr::LocalSet(index) => {
                let ty = self.local(*index)?;
                self.pop_expect(ty)?;
            },
            | Instr::LocalTee(index) => {
                let ty = self.local(*index)?;
                self.pop_expect(ty)?;
                self.push(Some(ty));
            },
            | Instr::GlobalGet(index) => {
                let ty = self.context.global(*index)?;
                self.push(Some(ty.value));
            },
            | Instr::GlobalSet(index) => {
                let ty = self.context.global(*index)?;
                if !ty.mutable {
                    return Err(format!("global {index} is immutable"));
                }
                self.pop_expect(ty.value)?;
            },
            | Instr::TableGet(table) => {
                let ty = self.context.table(*table)?;
                self.pop_expect(I32)?;
                self.push(Some(ty.element));
            },
            | Instr::TableSet(table) => {
                let ty = self.context.table(*table)?;
                self.pop_expect(ty.element)?;
                self.pop_expect(I32)?;
            },
            | Instr::Load(load, memarg) => {
                self.context.memory()?;
                if 1_u64 << memarg.align.min(63) > u64::from(load.size()) {
                    return Err("a load's alignment is larger than its width".into());
                }
                self.pop_expect(I32)?;
                self.push(Some(load.ty()));
            },
            | Instr::Store(store, memarg) => {
                self.context.memory()?;
                if 1_u64 << memarg.align.min(63) > u64::from(store.size()) {
                    return Err("a store's alignment is larger than its width".into());
                }
                self.pop_expect(store.ty())?;
                self.pop_expect(I32)?;
            },
            | Instr::MemorySize => {
                self.context.memory()?;
                self.push(Some(I32));
            },
            | Instr::MemoryGrow => {
                self.context.memory()?;
                self.pop_expect(I32)?;
                self.push(Some(I32));
            },
            | Instr::MemoryInit(segment) => {
                self.context.memory()?;
                self.data(*segment)?;
                self.pop_all(&[I32, I32, I32])?;
            },
            | Instr::DataDrop(segment) => self.data(*segment)?,
            | Instr::MemoryCopy | Instr::MemoryFill => {
                self.context.memory()?;
                self.pop_all(&[I32, I32, I32])?;
            },
            | Instr::TableInit { elem, table } => {
                let ty = self.context.table(*table)?;
                if self.element(*elem)? != ty.element {
                    return Err("table.init's segment and table have different types".into());
                }
                self.pop_all(&[I32, I32, I32])?;
            },
            | Instr::ElemDrop(elem) => {
                self.element(*elem)?;
            },
            | Instr::TableCopy { dst, src } => {
                if self.context.table(*dst)?.element != self.context.table(*src)?.element {
                    return Err("table.copy's tables have different types".into());
                }
                self.pop_all(&[I32, I32, I32])?;
            },
            | Instr::TableGrow(table) => {
                let ty = self.context.table(*table)?;
                self.pop_all(&[ty.element, I32])?;
                self.push(Some(I32));
            },
            | Instr::TableSize(table) => {
                self.context.table(*table)?;
                self.push(Some(I32));
            },
            | Instr::TableFill(table) => {
                let ty = self.context.table(*table)?;
                self.pop_all(&[I32, ty.element, I32])?;
            },
            | Instr::I32Const(_) => self.push(Some(I32)),
            | Instr::I64Const(_) => self.push(Some(I64)),
            | Instr::F32Const(_) => self.push(Some(F32)),
            | Instr::F64Const(_) => self.push(Some(F64)),
            | Instr::RefNull(ty) => self.push(Some(*ty)),
            | Instr::RefIsNull => {
                let operand = self.pop()?;
                if operand.is_some_and(|ty| !ty.is_reference()) {
                    return Err("ref.is_null needs a reference".into());
                }
                self.push(Some(I32));
            },
            | Instr::RefFunc(func) => {
                self.context.func_type(*func)?;
                self.push(Some(FuncRef));
            },
            | Instr::Numeric(op) => {
                let (operands, result) =
                    numeric_signature(*op).ok_or_else(|| format!("unknown opcode {op:#04x}"))?;
                self.pop_all(operands)?;
                self.push(Some(result));
            },
            | Instr::TruncSat(op) => {
                let (operand, result) =
                    trunc_sat_signature(*op).ok_or("unknown saturating truncation")?;
                self.pop_expect(operand)?;
                self.push(Some(result));
            },
        }
        Ok(())
    }

    fn local(&self, index: u32) -> Result<ValType, String> {
        self.locals
            .get(index as usize)
            .copied()
            .ok_or_else(|| format!("local {index} does not exist"))
    }

    fn data(&self, segment: u32) -> Result<(), String> {
        if (segment as usize) < self.context.module.data.len() {
            Ok(())
        } else {
            Err(format!("data segment {segment} does not exist"))
        }
    }

    fn element(&self, segment: u32) -> Result<ValType, String> {
        self.context
            .module
            .elements
            .get(segment as usize)
            .map(|element| element.ty)
            .ok_or_else(|| format!("element segment {segment} does not exist"))
    }

    fn push(&mut self, ty: Operand) {
        self.values.push(ty);
    }

    fn push_all(&mut self, types: &[ValType]) {
        self.values.extend(types.iter().map(|&ty| Some(ty)));
    }

    fn pop(&mut self) -> Result<Operand, String> {
        let control = self
            .controls
            .last()
            .ok_or("an instruction after the final end")?;
        if self.values.len() == control.height {
            if control.unreachable {
                return Ok(None);
            }
            return Err("an instruction needs an operand the stack does not have".into());
        }
        Ok(self.values.pop().flatten())
    }

    fn pop_expect(&mut self, expected: ValType) -> Result<Operand, String> {
        let actual = self.pop()?;
        match actual {
            | Some(actual) if actual != expected => Err(format!(
                "expected an {} operand but found an {}",
                expected.name(),
                actual.name()
            )),
            | _ => Ok(actual.or(Some(expected))),
        }
    }

    /// Pop operands of `types`, last first; returns them in stack order.
    fn pop_all(&mut self, types: &[ValType]) -> Result<Vec<Operand>, String> {
        let mut popped = Vec::with_capacity(types.len());
        for &ty in types.iter().rev() {
            popped.push(self.pop_expect(ty)?);
        }
        popped.reverse();
        Ok(popped)
    }

    fn push_control(&mut self, kind: Kind, start: Vec<ValType>, end: Vec<ValType>) {
        let height = self.values.len();
        self.push_all(&start);
        self.controls.push(Control {
            kind,
            start,
            end,
            height,
            unreachable: false,
        });
    }

    fn pop_control(&mut self) -> Result<Control, String> {
        let end = self
            .controls
            .last()
            .map(|control| control.end.clone())
            .ok_or("end without an open block")?;
        self.pop_all(&end)?;
        let control = self.controls.pop().ok_or("end without an open block")?;
        if self.values.len() != control.height {
            return Err("a block leaves extra values on the stack".into());
        }
        Ok(control)
    }

    fn label_types(&self, depth: u32) -> Result<Vec<ValType>, String> {
        let index = self
            .controls
            .len()
            .checked_sub(1 + depth as usize)
            .ok_or_else(|| format!("a branch to label {depth}, which does not exist"))?;
        let control = &self.controls[index];
        Ok(if control.kind == Kind::Loop {
            control.start.clone()
        } else {
            control.end.clone()
        })
    }

    fn unreachable(&mut self) {
        if let Some(control) = self.controls.last_mut() {
            self.values.truncate(control.height);
            control.unreachable = true;
        }
    }
}
