//! Scoped memory facts for GVN. Read `MemoryFacts::number` first, then the
//! rollback protocol and `AliasCheck`. All storage comes from draft scratch.
//!
//! A generation is the length of the current path's write log, like
//! `EarlyCSE`'s memory counter with an alias refinement: a fact from an older
//! generation survives only if every intervening write is provably disjoint. A
//! full barrier is a write of unknown extent. At a join or loop header GVN logs
//! the writes of every block on a path from the immediate dominator instead,
//! or a full barrier when that region is too large to scan.
//! The scoped table restores overwritten facts as well as removing new ones,
//! and truncates the log on leaving a subtree, so siblings see the same input.
//! This is an IR transformation; no C type-based alias assumptions are used.

use rustc_hash::FxBuildHasher;

use super::{
    draft::{
        Constant,
        Draft,
    },
    report::OptimizationReport,
};
use crate::{
    ir::{
        Block,
        Entity,
        GlobalId,
        Inst,
        InstData,
        InstFlags,
        Module,
        Opcode,
        StackSlot,
        Type,
        Value,
        ValueDef,
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
    },
};

/// Available loads and stored values on the current dominator path.
pub(super) struct MemoryFacts<'s, 'm, 'ir> {
    table:   ArenaMap<'s, Key, Available>,
    undo:    ArenaVec<'s, (Key, Option<Available>)>,
    writes:  ArenaVec<'s, Option<Location>>,
    aliases: AliasCheck<'s, 'm, 'ir>,
}

impl<'s, 'm, 'ir> MemoryFacts<'s, 'm, 'ir> {
    /// Reuses one non-volatile load, or records the instruction's memory
    /// effect. Every removed load consumes exactly one `allow()` rewrite.
    pub(super) fn number(
        &mut self,
        draft: &mut Draft<'_, '_>,
        inst: Inst,
        report: &mut OptimizationReport,
    ) -> bool {
        let data = *draft.inst(inst);
        self.log(memory_write(draft, data));
        if data.flags().contains(InstFlags::VOLATILE) {
            return false;
        }
        match data {
            | InstData::Load { addr, ty, .. } => {
                let key = Key {
                    addr: draft.resolve(addr),
                    ty,
                };
                let result = draft.inst_result(inst).expect("a load defines a value");
                let location = location(draft, key.addr, ty);
                let available = self.table.get(&key).copied().filter(|fact| {
                    self.writes[fact.generation..].iter().all(|write| {
                        write.is_some_and(|write| !self.aliases.may_alias(location, write))
                    })
                });
                if let Some(fact) = available
                    && report.allow()
                {
                    draft.replace(result, fact.value);
                    draft.remove_inst(inst);
                    self.remember(key, draft.resolve(fact.value));
                    return true;
                }
                self.remember(key, result);
            },
            | InstData::Store {
                args: [value, addr],
                ty,
                ..
            } => self.remember(
                Key {
                    addr: draft.resolve(addr),
                    ty,
                },
                draft.resolve(value),
            ),
            | _ => {},
        }
        false
    }

    pub(super) fn scope(&self) -> MemoryScope {
        MemoryScope {
            undo:       self.undo.len(),
            generation: self.writes.len(),
        }
    }

    pub(super) fn restore(&mut self, scope: MemoryScope) {
        while self.undo.len() > scope.undo {
            let (key, previous) = self.undo.pop().expect("the scope has an undo entry");
            if let Some(previous) = previous {
                _ = self.table.insert(key, previous);
            } else {
                _ = self.table.remove(&key);
            }
        }
        self.writes.truncate(scope.generation);
    }

    pub(super) fn barrier(&mut self) {
        self.writes.push(None);
    }

    fn log(&mut self, write: MemoryWrite) {
        match write {
            | MemoryWrite::None => {},
            | MemoryWrite::At(location) => self.writes.push(Some(location)),
            | MemoryWrite::Unknown => self.barrier(),
        }
    }

    /// Enters a join or loop header: logs every write of `region`, the
    /// blocks on some path from its immediate dominator to it, or a full
    /// barrier when the region is unknown. A fact from the dominator then
    /// survives exactly when no write on any incoming path may alias it.
    pub(super) fn join(&mut self, draft: &Draft<'_, '_>, region: Option<&[Block]>) {
        let Some(region) = region else {
            self.barrier();
            return;
        };
        for &block in region {
            for &inst in &draft.block(block).insts {
                if !draft.is_removed(inst) {
                    self.log(memory_write(draft, *draft.inst(inst)));
                }
            }
        }
    }

    fn remember(&mut self, key: Key, value: Value) {
        let previous = self.table.insert(
            key,
            Available {
                value,
                generation: self.writes.len(),
            },
        );
        self.undo.push((key, previous));
    }

    pub(super) fn new(draft: &Draft<'_, 's>, module: &'m Module<'ir>) -> Self {
        Self {
            table:   ArenaMap::with_hasher_in(FxBuildHasher, draft.scratch),
            undo:    ArenaVec::new_in(draft.scratch),
            writes:  ArenaVec::new_in(draft.scratch),
            aliases: AliasCheck::compute(draft, module),
        }
    }
}

/// What one instruction may write, as GVN sees it.
#[derive(Clone, Copy)]
enum MemoryWrite {
    /// Nothing; a non-volatile load or a pure instruction.
    None,
    /// Exactly the bytes of one store.
    At(Location),
    /// Anything: a full barrier.
    Unknown,
}

/// The only classifier of memory writes for GVN: both numbering a block and
/// scanning a join's region log what it returns, so an instruction that
/// writes cannot be missed by one of them. A volatile access, a call, a range
/// operation, an atomic (including relaxed loads and fences), or a `va_*`
/// operation, breakpoint, or cycle-counter observation is a full barrier.
/// A new instruction that
/// may write memory or synchronize must be added here.
fn memory_write(draft: &Draft<'_, '_>, data: InstData) -> MemoryWrite {
    if data.flags().contains(InstFlags::VOLATILE) {
        return MemoryWrite::Unknown;
    }
    match data {
        | InstData::Store {
            args: [_, addr],
            ty,
            ..
        } => MemoryWrite::At(location(draft, draft.resolve(addr), ty)),
        | InstData::Nullary {
            opcode: Opcode::DebugTrap | Opcode::ReadTimestamp,
            ..
        }
        | InstData::Atomic { .. }
        | InstData::Call { .. }
        | InstData::CallIndirect { .. }
        | InstData::VaArgAggregate { .. }
        | InstData::MemoryRange { .. }
        | InstData::Unary {
            opcode: Opcode::VaStart | Opcode::VaArg | Opcode::VaEnd | Opcode::StackRestore,
            ..
        }
        | InstData::Binary {
            opcode: Opcode::VaCopy,
            ..
        } => MemoryWrite::Unknown,
        | _ => MemoryWrite::None,
    }
}

/// Both the fact table and the memory counter are scoped, not function-wide.
#[derive(Clone, Copy, Debug)]
pub(super) struct MemoryScope {
    undo:       usize,
    generation: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    addr: Value,
    ty:   Type,
}

#[derive(Clone, Copy)]
struct Available {
    value:      Value,
    generation: usize,
}

/// A conservative escape analysis and byte-range alias oracle. Passing a
/// derived slot address through any unsupported operation (including block
/// parameters) counts as escape. Only address derivation and direct memory
/// accesses are whitelisted; this also catches stored, returned, call and
/// integer-converted addresses, even when the escape occurs later in the CFG.
///
/// An alias global names its target's storage, so global identities compare
/// after following the alias (GNU extension: GCC manual, "Common Variable
/// Attributes", alias).
struct AliasCheck<'s, 'm, 'ir> {
    escaped: ArenaVec<'s, bool>,
    module:  &'m Module<'ir>,
}

impl<'s, 'm, 'ir> AliasCheck<'s, 'm, 'ir> {
    fn may_alias(&self, a: Location, b: Location) -> bool {
        let (a, b) = (self.canonical(a), self.canonical(b));
        match (a.base, b.base) {
            | (Base::Stack(a), Base::Stack(b)) if a != b => return false,
            | (Base::Global(a), Base::Global(b)) if a != b => return false,
            | (Base::Stack(_), Base::Global(_)) | (Base::Global(_), Base::Stack(_)) =>
                return false,
            | (Base::Stack(slot), Base::Other(_)) | (Base::Other(_), Base::Stack(slot))
                if !self.escaped[slot.index()] =>
                return false,
            | _ => {},
        }
        if a.base == b.base
            && let (Some(a_start), Some(b_start)) = (a.offset, b.offset)
            && let (Some(a_end), Some(b_end)) = (
                a_start.checked_add(i64::from(a.size)),
                b_start.checked_add(i64::from(b.size)),
            )
        {
            return a_start < b_end && b_start < a_end;
        }
        true
    }

    /// The location with an alias global replaced by its target.
    fn canonical(&self, mut location: Location) -> Location {
        if let Base::Global(global) = location.base {
            location.base = Base::Global(self.module.global_definition(global));
        }
        location
    }

    fn compute(draft: &Draft<'_, 's>, module: &'m Module<'ir>) -> Self {
        let mut escaped = ArenaVec::new_in(draft.scratch);
        escaped.resize(draft.stack_slot_count(), false);
        let mut escape = |value| {
            if draft.value_type(value) == Type::Ptr
                && let Base::Stack(slot) = location(draft, value, Type::Ptr).base
            {
                escaped[slot.index()] = true;
            }
        };
        for block in &draft.blocks {
            if !block.alive {
                continue;
            }
            for &inst in &block.insts {
                if draft.is_removed(inst) {
                    continue;
                }
                match *draft.inst(inst) {
                    | InstData::Load { .. }
                    | InstData::Binary {
                        opcode: Opcode::PtrAdd,
                        ..
                    } => {},
                    | InstData::Store {
                        args: [value, _], ..
                    } => escape(draft.resolve(value)),
                    | _ => draft.for_each_operand(inst, &mut escape),
                }
            }
            block
                .term
                .for_each_control_operand(|value| escape(draft.resolve(value)));
            for edge in block.term.edges() {
                for &value in &edge.args {
                    escape(draft.resolve(value));
                }
            }
        }
        Self { escaped, module }
    }
}

#[derive(Clone, Copy)]
struct Location {
    base:   Base,
    offset: Option<i64>,
    size:   u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Base {
    Stack(StackSlot),
    Global(GlobalId),
    Other(Value),
}

/// Follows pointer derivations iteratively. Variable or overflowing offsets
/// retain the object identity but prevent range proofs. Unsupported pointer
/// producers are opaque; their operand uses make any source slots escape.
fn location(draft: &Draft<'_, '_>, addr: Value, ty: Type) -> Location {
    let mut value = draft.resolve(addr);
    let mut offset = Some(0_i64);
    let base = loop {
        let ValueDef::Result(inst) = draft.value_def(value) else {
            break Base::Other(value);
        };
        match *draft.inst(inst) {
            | InstData::StackAddr { slot } => break Base::Stack(slot),
            | InstData::GlobalAddr { global } => break Base::Global(global),
            | InstData::Binary {
                opcode: Opcode::PtrAdd,
                args: [base, delta],
                ..
            } => {
                offset = match (offset, draft.constant(delta)) {
                    | (Some(offset), Some(Constant::Int(Type::I64, bits))) =>
                        u64::try_from(bits).ok().and_then(|bits| {
                            offset.checked_add(i64::from_ne_bytes(bits.to_ne_bytes()))
                        }),
                    | _ => None,
                };
                value = draft.resolve(base);
            },
            | _ => break Base::Other(value),
        }
    };
    Location {
        base,
        offset,
        size: ty.bytes(),
    }
}
