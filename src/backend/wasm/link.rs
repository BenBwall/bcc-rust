//! Linking: the program's IR modules and the runtime's become one
//! WebAssembly module.
//!
//! [`link`] first builds the table of external definitions (a strong one
//! replaces a weak one; two strong ones are an error), then walks
//! everything reachable from the roots, resolving each reference by name:
//! to a definition, to a function the linker provides (WASI imports, the
//! memory intrinsics, the constructor runners and the `main` adapter), to
//! null for a weak symbol nothing defines, or, for the freestanding target,
//! to an import from the module `env`. The roots are `_start` for WASI and
//! every external function of the program otherwise, with the constructors
//! and `used` symbols. Only what is reached gets an index or an address.
//!
//! Linear memory starts with a 1 KiB guard below the stack, so that a null
//! pointer never addresses an object, then the shadow stack (growing down
//! from its top, so an overflow runs into the guard and then wraps to an
//! out-of-bounds address), then the data, then the heap from
//! `__heap_base` on. The function table's entry 0 is null, so a call
//! through a null function pointer traps.
//!
//! Names the linker provides:
//! - `__bcc_wasi_NAME` (a declared function): the import `NAME` from
//!   `wasi_snapshot_preview1`;
//! - `__bcc_wasm_memory_size`, `__bcc_wasm_memory_grow`: `memory.size` and
//!   `memory.grow` of memory 0, in pages;
//! - `__heap_base`, `__data_end` (declared objects): the first address after
//!   the data, as `wasm-ld` defines them;
//! - `__wasm_call_ctors`, `__wasm_call_dtors`: run the constructors in
//!   ascending priority, and the destructors in descending priority;
//! - `__bcc_main`: `int (int argc, char **argv)`, calling the program's `main`
//!   with as many of those as it takes.

use super::{
    WasmError,
    codegen::{
        Routine,
        registers_scalar,
        wasm_signature,
    },
    instr::{
        Instr,
        ValType,
        op,
    },
    low32,
    module::{
        Export,
        ExportKind,
        FuncType,
        Function,
        Import,
        Segment,
        WasmModule,
    },
    u32_of,
};
use crate::{
    ir::{
        Entity,
        FuncId,
        GlobalId,
        GlobalInit,
        InitKind,
        InstData,
        Linkage,
        Module,
        RelocationTarget,
        SigId,
        Signature,
        Symbol,
        Type,
    },
    target::Target,
    util::bump::{
        ArenaMap,
        ArenaString,
        ArenaVec,
        Bump,
    },
};

/// The bytes below the stack that no object occupies.
const GUARD: u32 = 1024;
/// The shadow stack's size.
pub(crate) const STACK_SIZE: u32 = 1 << 20;
const PAGE: u32 = 1 << 16;

/// What a direct call reaches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Callee {
    Function(u32),
    MemorySize,
    MemoryGrow,
    /// A weak function nothing defines.
    Null,
}

/// A function the linker writes itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub(crate) enum Synthetic {
    Main,
    Constructors,
    Destructors,
}

/// What a reference resolves to before indices are assigned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Resolved {
    Defined(u32, FuncId),
    Import(u32),
    Synthetic(Synthetic),
    MemorySize,
    MemoryGrow,
    Null,
}

/// The linked module's layout, which the code generator consults.
pub(crate) struct Linked<'a, 'ir> {
    units:             &'a [&'a Module<'ir>],
    /// Each unit's resolution of each function, once indices are known.
    callees:           ArenaVec<'a, ArenaVec<'a, Callee>>,
    /// Each unit's address of each global.
    addresses:         ArenaVec<'a, ArenaVec<'a, u32>>,
    /// Each unit's type index of each signature, `u32::MAX` if unused.
    sig_types:         ArenaVec<'a, ArenaVec<'a, u32>>,
    /// Each function index's table index, 0 when its address is not taken.
    table_index:       ArenaVec<'a, u32>,
    /// Each unit's name of each defined function, unique in the module.
    names:             ArenaVec<'a, ArenaVec<'a, &'a str>>,
    routines:          ArenaVec<'a, u32>,
    /// The defined functions in index order, after the imports: an input
    /// function, or a synthetic one.
    pub(crate) order:  ArenaVec<'a, Defined>,
    /// The types of the synthetic `main` adapter and constructor runners.
    synthetic_types:   [u32; 3],
    pub(crate) module: WasmModule<'a>,
}

/// A defined function of the output.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Defined {
    Input(u32, FuncId),
    Synthetic(Synthetic),
}

impl<'a, 'ir> Linked<'a, 'ir> {
    pub(crate) fn module(&self, unit: usize) -> &'a Module<'ir> {
        self.units[unit]
    }

    pub(crate) fn callee(&self, unit: usize, func: FuncId) -> Callee {
        self.callees[unit][func.index()]
    }

    pub(crate) fn global_address(&self, unit: usize, global: GlobalId) -> u32 {
        self.addresses[unit][global.index()]
    }

    /// The table index a function pointer to `func` holds; 0 for null.
    pub(crate) fn function_address(&self, unit: usize, func: FuncId) -> u32 {
        match self.callee(unit, func) {
            | Callee::Function(index) => self.table_index[index as usize],
            | _ => 0,
        }
    }

    pub(crate) fn type_of(&self, unit: usize, sig: SigId) -> u32 {
        self.sig_types[unit][sig.index()]
    }

    pub(crate) fn function_type(&self, unit: usize, sig: SigId) -> FuncType<'a> {
        self.module.types[self.type_of(unit, sig) as usize]
    }

    pub(crate) fn symbol_name(&self, unit: usize, func: FuncId) -> &'a str {
        self.names[unit][func.index()]
    }

    pub(crate) fn routine(&self, routine: Routine) -> u32 {
        self.routines[routine as usize]
    }
}

/// The state of linking.
struct Linker<'a, 'ir> {
    units:           &'a [&'a Module<'ir>],
    /// How many leading units are the program's.
    program:         usize,
    target:          Target,
    arena:           &'a Bump,
    symbols:         ArenaMap<'a, &'a str, (u32, Symbol)>,
    resolved:        ArenaVec<'a, ArenaVec<'a, Option<Resolved>>>,
    /// Each unit's global, resolved to the unit and global that defines it,
    /// or `None` for a null weak global; `Some(None)` marks a linker
    /// symbol at the end of the data.
    globals:         ArenaVec<'a, ArenaVec<'a, Option<GlobalRef>>>,
    reached:         ArenaVec<'a, ArenaVec<'a, bool>>,
    reached_globals: ArenaVec<'a, ArenaVec<'a, bool>>,
    work:            ArenaVec<'a, Work>,
    imports:         ArenaVec<'a, Import<'a>>,
    import_ids:      ArenaMap<'a, (&'a str, &'a str), u32>,
    synthetic:       [bool; 3],
    taken:           ArenaVec<'a, Resolved>,
    routines:        ArenaVec<'a, Option<Resolved>>,
    types:           ArenaVec<'a, FuncType<'a>>,
    type_ids:        ArenaMap<'a, FuncType<'a>, u32>,
    undefined:       ArenaVec<'a, &'a str>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GlobalRef {
    Defined(u32, GlobalId),
    Null,
    HeapBase,
}

#[derive(Clone, Copy)]
enum Work {
    Function(u32, FuncId),
    Global(u32, GlobalId),
}

/// Links `units`, of which the first `program` are the program's and the
/// rest the runtime's, for `target`.
pub(crate) fn link<'a, 'ir>(
    units: &'a [&'a Module<'ir>],
    program: usize,
    target: Target,
    arena: &'a Bump,
) -> Result<Linked<'a, 'ir>, WasmError<'a>> {
    let mut linker = Linker {
        units,
        program,
        target,
        arena,
        symbols: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        resolved: ArenaVec::new_in(arena),
        globals: ArenaVec::new_in(arena),
        reached: ArenaVec::new_in(arena),
        reached_globals: ArenaVec::new_in(arena),
        work: ArenaVec::new_in(arena),
        imports: ArenaVec::new_in(arena),
        import_ids: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        synthetic: [false; 3],
        taken: ArenaVec::new_in(arena),
        routines: ArenaVec::new_in(arena),
        types: ArenaVec::new_in(arena),
        type_ids: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        undefined: ArenaVec::new_in(arena),
    };
    linker.routines.resize(Routine::ALL.len(), None);
    for module in units {
        let mut resolved = ArenaVec::new_in(arena);
        resolved.resize(module.functions().count(), None);
        linker.resolved.push(resolved);
        let mut reached = ArenaVec::new_in(arena);
        reached.resize(module.functions().count(), false);
        linker.reached.push(reached);
        let mut globals = ArenaVec::new_in(arena);
        globals.resize(module.globals().count(), None);
        linker.globals.push(globals);
        let mut reached = ArenaVec::new_in(arena);
        reached.resize(module.globals().count(), false);
        linker.reached_globals.push(reached);
    }
    linker.define_symbols()?;
    linker.roots()?;
    while let Some(work) = linker.work.pop() {
        match work {
            | Work::Function(unit, func) => linker.scan_function(unit, func),
            | Work::Global(unit, global) => linker.scan_global(unit, global),
        }
    }
    if !linker.undefined.is_empty() {
        linker.undefined.sort_unstable();
        linker.undefined.dedup();
        let mut list = ArenaString::new_in(arena);
        for (index, name) in linker.undefined.iter().enumerate() {
            if index > 0 {
                list.push_str(", ");
            }
            list.push_str(name);
        }
        return Err(WasmError::Undefined(list.into_str()));
    }
    linker.finish()
}

impl<'a, 'ir> Linker<'a, 'ir> {
    /// The table of external and weak definitions.
    fn define_symbols(&mut self) -> Result<(), WasmError<'a>> {
        for (unit, module) in self.units.iter().enumerate() {
            let definitions = module
                .functions()
                .filter(|(_, function)| {
                    function.body.is_some() || function.attributes.alias.is_some()
                })
                .map(|(func, function)| (function.name, function.linkage, Symbol::Function(func)))
                .chain(
                    module
                        .globals()
                        .filter(|(_, global)| {
                            global.init.is_some() || global.attributes.alias.is_some()
                        })
                        .map(|(id, global)| (global.name, global.linkage, Symbol::Global(id))),
                );
            for (name, linkage, symbol) in definitions {
                if linkage == Linkage::Internal {
                    continue;
                }
                let name: &'a str = self.arena.alloc_str(name);
                let weak = linkage != Linkage::External;
                match self.symbols.get(name) {
                    | None => {
                        _ = self.symbols.insert(name, (u32_of(unit), symbol));
                    },
                    | Some(&(other, other_symbol)) => {
                        let other_weak = self.linkage(other, other_symbol) != Linkage::External;
                        if !weak && !other_weak {
                            return Err(WasmError::Duplicate(name));
                        }
                        if other_weak && !weak {
                            _ = self.symbols.insert(name, (u32_of(unit), symbol));
                        }
                    },
                }
            }
        }
        Ok(())
    }

    fn linkage(&self, unit: u32, symbol: Symbol) -> Linkage {
        let module = self.units[unit as usize];
        match symbol {
            | Symbol::Function(func) => module.function(func).linkage,
            | Symbol::Global(global) => module.global(global).linkage,
        }
    }

    fn roots(&mut self) -> Result<(), WasmError<'a>> {
        if self.target == Target::Wasm32Wasi {
            match self.symbols.get("_start").copied() {
                | Some((unit, Symbol::Function(func))) => {
                    _ = self.reference_function(unit, func, false);
                },
                | _ => return Err(WasmError::Undefined("_start")),
            }
        } else {
            for unit in 0..self.program {
                let module = self.units[unit];
                for (func, function) in module.functions() {
                    let defined = function.body.is_some() || function.attributes.alias.is_some();
                    if defined && function.linkage != Linkage::Internal {
                        _ = self.reference_function(u32_of(unit), func, false);
                    }
                }
                if module
                    .init_functions()
                    .iter()
                    .any(|init| init.kind == InitKind::Constructor)
                {
                    self.synthetic[Synthetic::Constructors as usize] = true;
                }
            }
        }
        for unit in 0..self.program {
            let module = self.units[unit];
            for init in module.init_functions() {
                _ = self.reference_function(u32_of(unit), init.func, false);
            }
            for (func, function) in module.functions() {
                if function.attributes.used && function.body.is_some() {
                    _ = self.reference_function(u32_of(unit), func, false);
                }
            }
            for (global, data) in module.globals() {
                if data.attributes.used && data.init.is_some() {
                    _ = self.reference_global(u32_of(unit), global);
                }
            }
        }
        Ok(())
    }

    /// Resolves a reference to `func` of `unit` and queues what it reaches.
    fn reference_function(&mut self, unit: u32, func: FuncId, address: bool) -> Resolved {
        let resolved = match self.resolved[unit as usize][func.index()] {
            | Some(resolved) => resolved,
            | None => {
                let resolved = self.resolve_function(unit, func);
                self.resolved[unit as usize][func.index()] = Some(resolved);
                resolved
            },
        };
        match resolved {
            | Resolved::Defined(def_unit, def_func) => {
                if !self.reached[def_unit as usize][def_func.index()] {
                    self.reached[def_unit as usize][def_func.index()] = true;
                    self.work.push(Work::Function(def_unit, def_func));
                }
            },
            | Resolved::Synthetic(kind) =>
                if !self.synthetic[kind as usize] {
                    self.synthetic[kind as usize] = true;
                    self.reach_synthetic(kind);
                },
            | _ => {},
        }
        if address && !self.taken.contains(&resolved) {
            self.taken.push(resolved);
        }
        resolved
    }

    /// What `func` of `unit` names.
    fn resolve_function(&mut self, unit: u32, func: FuncId) -> Resolved {
        let module = self.units[unit as usize];
        let function = module.function(func);
        if let Some(Symbol::Function(target)) = function.attributes.alias {
            return self.resolve_function(unit, target);
        }
        if function.body.is_some() {
            if function.linkage != Linkage::Internal
                && let Some(&(other, Symbol::Function(other_func))) =
                    self.symbols.get(function.name)
                && (other, other_func) != (unit, func)
            {
                return self.resolve_function(other, other_func);
            }
            return Resolved::Defined(unit, func);
        }
        if let Some(&(other, symbol)) = self.symbols.get(function.name) {
            return match symbol {
                | Symbol::Function(other_func) => self.resolve_function(other, other_func),
                | Symbol::Global(_) => {
                    self.undefined.push(self.arena.alloc_str(function.name));
                    Resolved::Null
                },
            };
        }
        let name = function.name;
        if let Some(field) = name.strip_prefix("__bcc_wasi_") {
            let ty = self.intern(wasm_signature(module.function_signature(func), self.arena));
            return Resolved::Import(self.import("wasi_snapshot_preview1", field, name, ty));
        }
        match name {
            | "__bcc_wasm_memory_size" => return Resolved::MemorySize,
            | "__bcc_wasm_memory_grow" => return Resolved::MemoryGrow,
            | "__bcc_main" => return Resolved::Synthetic(Synthetic::Main),
            | "__wasm_call_ctors" => return Resolved::Synthetic(Synthetic::Constructors),
            | "__wasm_call_dtors" => return Resolved::Synthetic(Synthetic::Destructors),
            | _ => {},
        }
        if function.linkage == Linkage::Weak {
            return Resolved::Null;
        }
        if self.target == Target::Wasm32Unknown {
            let ty = self.intern(wasm_signature(module.function_signature(func), self.arena));
            return Resolved::Import(self.import("env", name, name, ty));
        }
        self.undefined.push(self.arena.alloc_str(name));
        Resolved::Null
    }

    /// The index of the import `module`.`field`, added on first use.
    fn import(&mut self, module: &str, field: &str, name: &str, ty: u32) -> u32 {
        let module: &'a str = self.arena.alloc_str(module);
        let field: &'a str = self.arena.alloc_str(field);
        if let Some(&index) = self.import_ids.get(&(module, field)) {
            return index;
        }
        let index = u32_of(self.imports.len());
        self.imports.push(Import {
            module,
            field,
            name: self.arena.alloc_str(name),
            ty,
        });
        _ = self.import_ids.insert((module, field), index);
        index
    }

    fn reach_synthetic(&mut self, kind: Synthetic) {
        match kind {
            | Synthetic::Main => match self.symbols.get("main").copied() {
                | Some((unit, Symbol::Function(func))) => {
                    _ = self.reference_function(unit, func, false);
                },
                | _ => self.undefined.push("main"),
            },
            | Synthetic::Constructors | Synthetic::Destructors =>
                for unit in 0..self.units.len() {
                    for init in self.units[unit].init_functions() {
                        _ = self.reference_function(u32_of(unit), init.func, false);
                    }
                },
        }
    }

    fn reference_global(&mut self, unit: u32, global: GlobalId) -> GlobalRef {
        let resolved = match self.globals[unit as usize][global.index()] {
            | Some(resolved) => resolved,
            | None => {
                let resolved = self.resolve_global(unit, global);
                self.globals[unit as usize][global.index()] = Some(resolved);
                resolved
            },
        };
        if let GlobalRef::Defined(def_unit, def_global) = resolved
            && !self.reached_globals[def_unit as usize][def_global.index()]
        {
            self.reached_globals[def_unit as usize][def_global.index()] = true;
            self.work.push(Work::Global(def_unit, def_global));
        }
        resolved
    }

    fn resolve_global(&mut self, unit: u32, global: GlobalId) -> GlobalRef {
        let module = self.units[unit as usize];
        let data = module.global(global);
        if let Some(Symbol::Global(target)) = data.attributes.alias {
            return self.resolve_global(unit, target);
        }
        if data.init.is_some() {
            if data.linkage != Linkage::Internal
                && let Some(&(other, Symbol::Global(other_global))) = self.symbols.get(data.name)
                && (other, other_global) != (unit, global)
            {
                return self.resolve_global(other, other_global);
            }
            return GlobalRef::Defined(unit, global);
        }
        if let Some(&(other, Symbol::Global(other_global))) = self.symbols.get(data.name) {
            return self.resolve_global(other, other_global);
        }
        if matches!(data.name, "__heap_base" | "__data_end") {
            return GlobalRef::HeapBase;
        }
        if data.linkage != Linkage::Weak {
            self.undefined.push(self.arena.alloc_str(data.name));
        }
        GlobalRef::Null
    }

    /// Queues everything `func`'s body reaches.
    fn scan_function(&mut self, unit: u32, func: FuncId) {
        let module = self.units[unit as usize];
        let function = module.function(func);
        _ = self.signature_type(unit, function.signature);
        let Some(body) = &function.body else { return };
        for block in body.blocks() {
            for &inst in body.block_insts(block) {
                let data = body.inst(inst);
                match *data {
                    | InstData::Call { func, .. } => {
                        _ = self.reference_function(unit, func, false);
                    },
                    | InstData::FuncAddr { func } => {
                        _ = self.reference_function(unit, func, true);
                    },
                    | InstData::GlobalAddr { global } => {
                        _ = self.reference_global(unit, global);
                    },
                    | InstData::CallIndirect { sig, .. } => {
                        _ = self.signature_type(unit, sig);
                    },
                    | _ => {},
                }
                super::codegen::routines_of(
                    data,
                    |value| body.value_type(value),
                    |routine| {
                        self.reference_routine(routine);
                    },
                );
            }
        }
    }

    fn reference_routine(&mut self, routine: Routine) {
        if self.routines[routine as usize].is_some() {
            return;
        }
        let name = routine.name();
        let resolved = match self.symbols.get(name).copied() {
            | Some((unit, Symbol::Function(func))) => self.reference_function(unit, func, false),
            | _ if self.target == Target::Wasm32Unknown => {
                let (_, params, result) = routine.signature();
                let signature = plain_signature(params, result);
                let ty = self.intern(wasm_signature(signature, self.arena));
                Resolved::Import(self.import("env", name, name, ty))
            },
            | _ => {
                self.undefined.push(name);
                Resolved::Null
            },
        };
        self.routines[routine as usize] = Some(resolved);
    }

    fn scan_global(&mut self, unit: u32, global: GlobalId) {
        let module = self.units[unit as usize];
        let Some(GlobalInit::Bytes { relocations, .. }) = module.global(global).init else {
            return;
        };
        for relocation in relocations {
            match relocation.target {
                | RelocationTarget::Symbol(Symbol::Function(func)) => {
                    _ = self.reference_function(unit, func, true);
                },
                | RelocationTarget::Symbol(Symbol::Global(target)) => {
                    _ = self.reference_global(unit, target);
                },
                | _ => {},
            }
        }
    }

    /// The type index of `sig` of `unit`.
    fn signature_type(&mut self, unit: u32, sig: SigId) -> u32 {
        let signature = self.units[unit as usize].signature(sig);
        self.intern(wasm_signature(signature, self.arena))
    }

    fn intern(&mut self, ty: FuncType<'a>) -> u32 {
        if let Some(&index) = self.type_ids.get(&ty) {
            return index;
        }
        let index = u32_of(self.types.len());
        self.types.push(ty);
        _ = self.type_ids.insert(ty, index);
        index
    }

    /// Assigns indices and addresses, and builds the module around the
    /// functions the code generator will fill in.
    fn finish(mut self) -> Result<Linked<'a, 'ir>, WasmError<'a>> {
        let arena = self.arena;
        let imports = u32_of(self.imports.len());
        // Defined functions: the program's, then the runtime's, then the
        // synthetic ones.
        let mut order = ArenaVec::new_in(arena);
        let mut index_of = ArenaVec::new_in(arena);
        for (unit, reached) in self.reached.iter().enumerate() {
            let mut indices = ArenaVec::with_capacity_in(reached.len(), arena);
            for (func, &is_reached) in reached.iter().enumerate() {
                if is_reached {
                    indices.push(imports + u32_of(order.len()));
                    order.push(Defined::Input(u32_of(unit), FuncId::new(func)));
                } else {
                    indices.push(u32::MAX);
                }
            }
            index_of.push(indices);
        }
        let mut synthetic_index = [u32::MAX; 3];
        for kind in [
            Synthetic::Main,
            Synthetic::Constructors,
            Synthetic::Destructors,
        ] {
            if self.synthetic[kind as usize] {
                synthetic_index[kind as usize] = imports + u32_of(order.len());
                order.push(Defined::Synthetic(kind));
            }
        }
        let callee_of = |resolved: Resolved| -> Callee {
            match resolved {
                | Resolved::Defined(unit, func) =>
                    Callee::Function(index_of[unit as usize][func.index()]),
                | Resolved::Import(import) => Callee::Function(import),
                | Resolved::Synthetic(kind) => Callee::Function(synthetic_index[kind as usize]),
                | Resolved::MemorySize => Callee::MemorySize,
                | Resolved::MemoryGrow => Callee::MemoryGrow,
                | Resolved::Null => Callee::Null,
            }
        };
        let mut callees = ArenaVec::with_capacity_in(self.units.len(), arena);
        for resolved in &self.resolved {
            let mut list = ArenaVec::with_capacity_in(resolved.len(), arena);
            list.extend(
                resolved
                    .iter()
                    .map(|resolved| resolved.map_or(Callee::Null, callee_of)),
            );
            callees.push(list);
        }
        let mut routines = ArenaVec::with_capacity_in(Routine::ALL.len(), arena);
        routines.extend(
            self.routines
                .iter()
                .map(|resolved| match resolved.map(callee_of) {
                    | Some(Callee::Function(index)) => index,
                    | _ => u32::MAX,
                }),
        );
        // The table: every function whose address is taken.
        let total = imports + u32_of(order.len());
        let mut table_index = ArenaVec::with_capacity_in(total as usize, arena);
        table_index.resize(total as usize, 0_u32);
        let mut taken: ArenaVec<'_, u32> = ArenaVec::new_in(arena);
        taken.extend(
            self.taken
                .iter()
                .filter_map(|&resolved| match callee_of(resolved) {
                    | Callee::Function(index) => Some(index),
                    | _ => None,
                }),
        );
        taken.sort_unstable();
        taken.dedup();
        for (position, &function) in taken.iter().enumerate() {
            table_index[function as usize] = u32_of(position) + 1;
        }
        // Memory: the guard, the stack, the data, the heap.
        let stack_top = GUARD + STACK_SIZE;
        let mut next = u64::from(stack_top);
        let mut placed = ArenaVec::with_capacity_in(self.units.len(), arena);
        for (unit, reached) in self.reached_globals.iter().enumerate() {
            let module = self.units[unit];
            let mut addresses = ArenaVec::with_capacity_in(reached.len(), arena);
            for (global, &is_reached) in reached.iter().enumerate() {
                if is_reached {
                    let data = module.global(GlobalId::new(global));
                    next = next.next_multiple_of(data.align.bytes());
                    addresses.push(low32(u128::from(next)));
                    next += data.size;
                } else {
                    addresses.push(0);
                }
            }
            placed.push(addresses);
        }
        if next > u64::from(u32::MAX) - u64::from(PAGE) {
            return Err(WasmError::Unsupported {
                function:  None,
                construct: "more than 4 GiB of static data",
            });
        }
        let heap_base = low32(u128::from(next.next_multiple_of(16)));
        let mut addresses = ArenaVec::with_capacity_in(self.units.len(), arena);
        for (unit, globals) in self.globals.iter().enumerate() {
            let mut list = ArenaVec::with_capacity_in(globals.len(), arena);
            list.extend(globals.iter().map(|resolved| match resolved {
                | Some(GlobalRef::Defined(def_unit, def_global)) =>
                    placed[*def_unit as usize][def_global.index()],
                | Some(GlobalRef::HeapBase) => heap_base,
                | _ => 0,
            }));
            // A global reached only through its own definition.
            for (global, address) in list.iter_mut().enumerate() {
                if self.reached_globals[unit][global] {
                    *address = placed[unit][global];
                }
            }
            addresses.push(list);
        }
        // Signature types per unit.
        let mut sig_types = ArenaVec::with_capacity_in(self.units.len(), arena);
        for unit in 0..self.units.len() {
            let module = self.units[unit];
            let mut list = ArenaVec::with_capacity_in(module.signature_count(), arena);
            for index in 0..module.signature_count() {
                let ty = wasm_signature(module.signature(SigId::new(index)), arena);
                list.push(self.intern(ty));
            }
            sig_types.push(list);
        }
        // Unique function names.
        let mut seen: ArenaMap<'_, &str, u32> =
            ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
        for import in &self.imports {
            _ = seen.insert(import.name, 1);
        }
        let mut names = ArenaVec::with_capacity_in(self.units.len(), arena);
        for (unit, module) in self.units.iter().enumerate() {
            let mut list = ArenaVec::with_capacity_in(module.functions().count(), arena);
            for (func, function) in module.functions() {
                if !self.reached[unit][func.index()] {
                    list.push("");
                    continue;
                }
                let name: &'a str = arena.alloc_str(function.name);
                let count = seen.entry(name).or_insert(0);
                *count += 1;
                list.push(if *count == 1 {
                    name
                } else {
                    let mut unique = ArenaString::new_in(arena);
                    _ = core::fmt::Write::write_fmt(
                        &mut unique,
                        format_args!("{name}.{}", *count - 1),
                    );
                    unique.into_str()
                });
            }
            names.push(list);
        }
        // Data segments, with every address constant patched in.
        let mut data = ArenaVec::new_in(arena);
        for (unit, reached) in self.reached_globals.iter().enumerate() {
            let module = self.units[unit];
            for (global, &is_reached) in reached.iter().enumerate() {
                if !is_reached {
                    continue;
                }
                let Some(GlobalInit::Bytes { bytes, relocations }) =
                    module.global(GlobalId::new(global)).init
                else {
                    continue;
                };
                let patched = arena.alloc_slice_copy(bytes);
                for relocation in relocations {
                    let offset = usize::try_from(relocation.offset)
                        .expect("a relocation is inside its global");
                    let (value, size): (u64, usize) = match relocation.target {
                        | RelocationTarget::Symbol(Symbol::Function(func)) => {
                            let callee = callees[unit][func.index()];
                            let value = match callee {
                                | Callee::Function(index) => u64::from(table_index[index as usize]),
                                | _ => 0,
                            };
                            (value.wrapping_add(relocation.addend as u64), 4)
                        },
                        | RelocationTarget::Symbol(Symbol::Global(target)) => (
                            u64::from(addresses[unit][target.index()])
                                .wrapping_add(relocation.addend as u64),
                            4,
                        ),
                        | RelocationTarget::Label { label, .. } => (
                            u64::from(label.as_u32() + 1).wrapping_add(relocation.addend as u64),
                            4,
                        ),
                        | RelocationTarget::LabelDifference {
                            label, base, width, ..
                        } => (
                            u64::from(label.as_u32()).wrapping_sub(u64::from(base.as_u32())),
                            width.bytes() as usize,
                        ),
                    };
                    let end = (offset + size).min(patched.len());
                    patched[offset..end].copy_from_slice(&value.to_le_bytes()[..end - offset]);
                }
                if patched.iter().all(|&byte| byte == 0) {
                    continue;
                }
                data.push(Segment {
                    offset: placed[unit][global],
                    bytes:  patched,
                });
            }
        }
        // Exports.
        let mut exports = ArenaVec::new_in(arena);
        exports.push(Export {
            name:  "memory",
            kind:  ExportKind::Memory,
            index: 0,
        });
        let mut start = None;
        if self.target == Target::Wasm32Wasi {
            if let Some(&(unit, Symbol::Function(func))) = self.symbols.get("_start")
                && let Some(Callee::Function(index)) =
                    callees[unit as usize].get(func.index()).copied()
            {
                exports.push(Export {
                    name: "_start",
                    kind: ExportKind::Function,
                    index,
                });
            }
        } else {
            for unit in 0..self.program {
                let module = self.units[unit];
                for (func, function) in module.functions() {
                    let defined = function.body.is_some() || function.attributes.alias.is_some();
                    if defined
                        && function.linkage != Linkage::Internal
                        && let Callee::Function(index) = callees[unit][func.index()]
                        && !exports
                            .iter()
                            .any(|export: &Export<'_>| export.name == function.name)
                    {
                        exports.push(Export {
                            name: arena.alloc_str(function.name),
                            kind: ExportKind::Function,
                            index,
                        });
                    }
                }
            }
            if synthetic_index[Synthetic::Constructors as usize] != u32::MAX {
                start = Some(synthetic_index[Synthetic::Constructors as usize]);
            }
        }
        let void = self.intern(FuncType {
            params:  &[],
            results: &[],
        });
        let main_type = self.intern(FuncType {
            params:  &[ValType::I32, ValType::I32],
            results: &[ValType::I32],
        });
        let mut table = ArenaVec::with_capacity_in(taken.len(), arena);
        table.extend(taken.iter().copied());
        let memory_pages = low32(u128::from(
            (u64::from(heap_base) + u64::from(PAGE)).div_ceil(u64::from(PAGE)),
        ));
        let linked = Linked {
            units: self.units,
            callees,
            addresses,
            sig_types,
            table_index,
            names,
            routines,
            order,
            synthetic_types: [main_type, void, void],
            module: WasmModule {
                types: self.types,
                imports: self.imports,
                functions: ArenaVec::new_in(arena),
                table,
                memory_pages,
                stack_pointer: stack_top,
                exports,
                start,
                data,
            },
        };
        Ok(linked)
    }
}

/// A signature of plain scalars, as a runtime routine's.
fn plain_signature(params: &[Type], result: Option<Type>) -> Signature<'_> {
    Signature {
        params,
        result,
        variadic: false,
        variadic_fixed: None,
        aggregates: &[],
        extensions: &[],
        result_extension: None,
        noundef: &[],
        result_noundef: false,
        variadic_noundef: false,
    }
}

impl<'a> Linked<'a, '_> {
    /// The body of a function the linker writes itself.
    pub(crate) fn synthetic(&self, kind: Synthetic, arena: &'a Bump) -> Function<'a> {
        let mut code = ArenaVec::new_in(arena);
        let (name, ty, names): (&str, u32, &[&str]) = match kind {
            | Synthetic::Main => ("__bcc_main", self.synthetic_types[0], &["argc", "argv"]),
            | Synthetic::Constructors => ("__wasm_call_ctors", self.synthetic_types[1], &[]),
            | Synthetic::Destructors => ("__wasm_call_dtors", self.synthetic_types[2], &[]),
        };
        match kind {
            | Synthetic::Main =>
                if let Some((unit, func)) = self.find_main() {
                    let module = self.units[unit];
                    let signature = module.function_signature(func);
                    let params = wasm_signature(signature, arena).params.len();
                    for position in 0..params.min(3) {
                        if position < 2 {
                            code.push(Instr::LocalGet(u32_of(position)));
                        } else {
                            code.push(Instr::I32Const(0));
                        }
                    }
                    if let Callee::Function(index) = self.callee(unit, func) {
                        code.push(Instr::Call(index));
                    }
                    let returns = registers_scalar(signature).or(signature.result);
                    if returns.is_none() {
                        code.push(Instr::I32Const(0));
                    } else if returns == Some(Type::I64) {
                        code.push(Instr::Num(op::I32_WRAP_I64));
                    }
                } else {
                    code.push(Instr::Unreachable);
                },
            | Synthetic::Constructors | Synthetic::Destructors => {
                let wanted = if kind == Synthetic::Constructors {
                    InitKind::Constructor
                } else {
                    InitKind::Destructor
                };
                let mut inits = ArenaVec::new_in(arena);
                for (unit, module) in self.units.iter().enumerate() {
                    for init in module.init_functions() {
                        if init.kind == wanted
                            && let Callee::Function(index) = self.callee(unit, init.func)
                        {
                            inits.push((init.priority, index));
                        }
                    }
                }
                inits.sort_by_key(|&(priority, _)| priority);
                if kind == Synthetic::Destructors {
                    inits.reverse();
                }
                for &(_, index) in &inits {
                    code.push(Instr::Call(index));
                }
            },
        }
        Function {
            name,
            ty,
            locals: &[],
            local_names: arena.alloc_slice_copy(names),
            code: arena.alloc_slice_copy(&code),
            labels: &[],
        }
    }

    fn find_main(&self) -> Option<(usize, FuncId)> {
        for (unit, module) in self.units.iter().enumerate() {
            if let Some(Symbol::Function(func)) = module.symbol("main")
                && module.function(func).body.is_some()
            {
                return Some((unit, func));
            }
        }
        None
    }
}
