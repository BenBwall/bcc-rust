//! Instantiation (WebAssembly 2.0 §4.5.4): resolve imports, allocate the
//! memory, tables and globals, copy the active data and element segments,
//! and queue the start function and the entry point for the host to call.
//!
//! The stepper is the only host. It provides WASI preview 1's functions
//! (`wasi.rs`) and creates an imported memory or table itself; other imports
//! stay unresolved and trap only if they are called, so a program that
//! never reaches them still runs. An imported global cannot be provided.

use std::{
    fmt,
    rc::Rc,
};

use super::{
    Effect,
    Event,
    Machine,
    Status,
    memory::{
        MAX_PAGES,
        Memory,
    },
    tables::Table,
};
use crate::{
    instr::Instr,
    module::{
        ConstExpr,
        ExternKind,
        ImportKind,
        Module,
        SegmentMode,
    },
    value::Value,
    wasi::{
        Host,
        WasiFunction,
    },
};

/// Why a module could not be instantiated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstantiateError(pub String);

impl fmt::Display for InstantiateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Machine {
    /// Instantiate a validated `module`, ready to call `entry` (after the
    /// start function, if any).
    ///
    /// # Errors
    /// When an import cannot be provided, a memory is too large, or a
    /// segment does not fit.
    #[expect(clippy::too_many_lines, reason = "The steps of §4.5.4, in order.")]
    pub fn instantiate(
        module: Rc<Module>,
        entry: Option<u32>,
        host: Host,
    ) -> Result<Self, InstantiateError> {
        let mut globals = Vec::new();
        let mut tables = Vec::new();
        let mut memory = None;
        let mut resolved = Vec::new();
        for import in &module.imports {
            match import.kind {
                | ImportKind::Func(ty) => {
                    let ty = module.types.get(ty as usize);
                    resolved.push(
                        WasiFunction::resolve(&import.module, &import.name)
                            .filter(|_| crate::wasi::provides(&import.module, &import.name, ty)),
                    );
                },
                | ImportKind::Memory(limits) => memory = Some(new_memory(limits.min, limits.max)?),
                | ImportKind::Table(ty) => tables.push(Table {
                    elements: vec![Value::default_of(ty.element); ty.limits.min as usize],
                    max:      ty.limits.max,
                }),
                | ImportKind::Global(_) => {
                    return Err(InstantiateError(format!(
                        "the module imports the global `{}.{}`, which the stepper cannot provide",
                        import.module, import.name
                    )));
                },
            }
        }
        // Defined functions resolve to nothing.
        resolved.resize(module.funcs.len(), None);
        if let Some(limits) = module.memories.first() {
            memory = Some(new_memory(limits.min, limits.max)?);
        }
        for table in &module.tables {
            if table.limits.min > 1 << 20 {
                return Err(InstantiateError(
                    "a table is larger than the stepper allows".into(),
                ));
            }
            tables.push(Table {
                elements: vec![Value::default_of(table.element); table.limits.min as usize],
                max:      table.limits.max,
            });
        }
        for global in &module.globals {
            let value = evaluate(&global.init, &globals)?;
            globals.push(value);
        }
        let mut elements = Vec::new();
        for element in &module.elements {
            let items = element
                .items
                .iter()
                .map(|item| evaluate(item, &globals))
                .collect::<Result<Vec<_>, _>>()?;
            elements.push(items);
        }
        // Active segments are copied in order, then dropped (§4.5.4).
        for (index, element) in module.elements.iter().enumerate() {
            match &element.mode {
                | SegmentMode::Active {
                    index: table,
                    offset,
                } => {
                    let offset = offset32(offset, &globals)?;
                    let table = tables
                        .get_mut(*table as usize)
                        .ok_or_else(|| InstantiateError("an element segment's table".into()))?;
                    let items = std::mem::take(&mut elements[index]);
                    let end = offset as usize + items.len();
                    if end > table.elements.len() {
                        return Err(InstantiateError(format!(
                            "element segment {index} does not fit in its table"
                        )));
                    }
                    table.elements[offset as usize..end].copy_from_slice(&items);
                },
                | SegmentMode::Declarative => elements[index].clear(),
                | SegmentMode::Passive => {},
            }
        }
        let mut dropped_data = vec![false; module.data.len()];
        for (index, data) in module.data.iter().enumerate() {
            if let SegmentMode::Active { offset, .. } = &data.mode {
                let offset = offset32(offset, &globals)?;
                let memory = memory
                    .as_mut()
                    .ok_or_else(|| InstantiateError("a data segment needs a memory".into()))?;
                memory.write(offset, &data.bytes).map_err(|_| {
                    InstantiateError(format!(
                        "data segment {index} ({} bytes at {offset}) does not fit in memory",
                        data.bytes.len()
                    ))
                })?;
                dropped_data[index] = true;
            }
        }
        let stack_pointer = (0..globals.len() as u32)
            .find(|&index| module.global_name(index) == Some("__stack_pointer"));
        let mut pending: Vec<u32> = entry.into_iter().collect();
        pending.extend(module.start);
        Ok(Self {
            module,
            memory,
            globals,
            tables,
            elements,
            dropped_data,
            stack: Vec::new(),
            frames: Vec::new(),
            labels: Vec::new(),
            status: Status::Running,
            steps: 0,
            host,
            resolved,
            pending,
            queued_args: 0,
            stack_pointer,
            event: Event {
                effect: Effect::None,
                ..Event::default()
            },
        })
    }
}

/// The function the host calls to run the program: WASI's `_start`, or C's
/// `main`. Clang names a `main(void)` `__original_main`, exports it as
/// `__main_void` (which wasi-libc's `_start` calls), and adds a `main(argc,
/// argv)` wrapper with no source lines, so `__main_void` comes first.
pub fn entry_point(module: &Module) -> Option<u32> {
    ["_start", "__main_void", "main", "__original_main"]
        .iter()
        .find_map(|name| module.export(name, ExternKind::Func))
}

fn new_memory(min: u32, max: Option<u32>) -> Result<Memory, InstantiateError> {
    if min > MAX_PAGES {
        return Err(InstantiateError(format!(
            "the module asks for {min} pages of memory ({} MiB); the stepper allows at most \
             {MAX_PAGES} (16 MiB)",
            u64::from(min) * 64 / 1024
        )));
    }
    Ok(Memory::new(min, max))
}

/// Evaluate a constant expression (§4.4.11) over the globals so far.
fn evaluate(expr: &ConstExpr, globals: &[Value]) -> Result<Value, InstantiateError> {
    let mut stack = Vec::new();
    for instr in &expr.code {
        let value = match instr {
            | Instr::I32Const(value) => Value::I32(*value),
            | Instr::I64Const(value) => Value::I64(*value),
            | Instr::F32Const(bits) => Value::F32(*bits),
            | Instr::F64Const(bits) => Value::F64(*bits),
            | Instr::RefNull(ty) => Value::default_of(*ty),
            | Instr::RefFunc(func) => Value::FuncRef(Some(*func)),
            | Instr::GlobalGet(index) => *globals
                .get(*index as usize)
                .ok_or_else(|| InstantiateError("a constant reads an unknown global".into()))?,
            | Instr::Numeric(op) => {
                let b = stack.pop();
                let a = stack.pop();
                match (a, b) {
                    | (Some(a), Some(b)) => super::numeric::apply(*op, &[a, b])
                        .map_err(|trap| InstantiateError(trap.message))?,
                    | _ => return Err(InstantiateError("a malformed constant".into())),
                }
            },
            | Instr::End => break,
            | _ => return Err(InstantiateError("a non-constant initializer".into())),
        };
        stack.push(value);
    }
    stack
        .pop()
        .ok_or_else(|| InstantiateError("an empty constant expression".into()))
}

fn offset32(expr: &ConstExpr, globals: &[Value]) -> Result<u32, InstantiateError> {
    match evaluate(expr, globals)? {
        | Value::I32(offset) => Ok(offset as u32),
        | _ => Err(InstantiateError("a segment offset must be an i32".into())),
    }
}
