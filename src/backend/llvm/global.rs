//! Global declarations and packed initializers with byte-exact relocations:
//! symbol and label addresses as pointers, label differences as integers.
use super::{
    EmitError,
    Emitter,
    api::{
        Session,
        Ty,
        Val,
    },
};
use crate::{
    ir::{
        BlockLabel,
        Entity,
        FuncId,
        Global,
        GlobalId,
        GlobalInit,
        Relocation,
        RelocationTarget,
        Symbol,
        Type,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

pub(super) fn global_type<'id, 'm>(
    api: &Session<'id>,
    global: &'m Global<'_>,
    arena: &Bump,
) -> Result<Ty<'id>, EmitError<'m>> {
    let Some(GlobalInit::Bytes { bytes, relocations }) = global.init else {
        return Ok(api.array(api.ty(Type::I8), global.size));
    };
    if relocations.is_empty() {
        return Ok(api.array(api.ty(Type::I8), global.size));
    }
    let mut fields = ArenaVec::new_in(arena);
    pieces(global.name, bytes, relocations, arena, |piece| {
        fields.push(match piece {
            | Piece::Bytes(bytes) => api.array(api.ty(Type::I8), bytes.len() as u64),
            | Piece::Pointer(Relocation {
                target: RelocationTarget::LabelDifference { width, .. },
                ..
            }) => api.ty(width),
            | Piece::Pointer(_) => api.ty(Type::Ptr),
        });
    })?;
    Ok(api.structure(&fields, true))
}

impl<'id, 'm> Emitter<'_, 'id, 'm, '_> {
    pub(super) fn global(&self, id: GlobalId) -> Result<(), EmitError<'m>> {
        let global = self.module.global(id);
        let Some(init) = global.init else {
            return Ok(());
        };
        let api = self.api;
        let value = match init {
            | GlobalInit::Bytes { bytes, relocations } if !relocations.is_empty() => {
                let mut values = ArenaVec::new_in(self.arena);
                pieces(global.name, bytes, relocations, self.arena, |piece| {
                    values.push(match piece {
                        | Piece::Bytes(bytes) => api.bytes(bytes),
                        | Piece::Pointer(relocation) => self.relocation(relocation),
                    });
                })?;
                api.const_structure(&values)
            },
            | GlobalInit::Bytes { bytes, .. } => api.bytes(bytes),
            | GlobalInit::Zero => api.null(api.array(api.ty(Type::I8), global.size)),
        };
        api.initializer(self.globals[id.index()], value);
        Ok(())
    }

    /// The constant a relocation's bytes hold: an address plus its addend,
    /// or a label difference as Clang emits it,
    /// `trunc(sub(ptrtoint(blockaddress), ptrtoint(blockaddress)))`.
    fn relocation(&self, relocation: Relocation) -> Val<'id> {
        let api = self.api;
        let label = |func: FuncId, label: BlockLabel| {
            api.block_address(
                self.functions[func.index()],
                self.labels[func.index()][label.index()],
            )
        };
        let address = match relocation.target {
            | RelocationTarget::Symbol(Symbol::Function(func)) => self.functions[func.index()],
            | RelocationTarget::Symbol(Symbol::Global(global)) => self.globals[global.index()],
            | RelocationTarget::Label {
                func,
                label: target,
            } => label(func, target),
            | RelocationTarget::LabelDifference {
                func,
                label: target,
                base,
                width,
            } =>
                return api.label_difference(
                    label(func, target),
                    label(func, base),
                    relocation.addend,
                    width,
                ),
        };
        if relocation.addend == 0 {
            address
        } else {
            api.const_gep(address, relocation.addend)
        }
    }
}

enum Piece<'a> {
    Bytes(&'a [u8]),
    Pointer(Relocation),
}

fn pieces<'m>(
    name: &'m str,
    bytes: &[u8],
    relocations: &[Relocation],
    arena: &Bump,
    mut visit: impl FnMut(Piece<'_>),
) -> Result<(), EmitError<'m>> {
    let sorted = arena.alloc_slice_copy(relocations);
    sorted.sort_unstable_by_key(|relocation| relocation.offset);
    for pair in sorted.windows(2) {
        if pair[0].offset + pair[0].size(8) > pair[1].offset {
            return Err(EmitError::OverlappingRelocations {
                global: name,
                first:  pair[0].offset,
                second: pair[1].offset,
            });
        }
    }
    let mut start = 0;
    for &relocation in sorted.iter() {
        let offset = usize::try_from(relocation.offset).map_err(|_| std::fmt::Error)?;
        if offset > start {
            visit(Piece::Bytes(&bytes[start..offset]));
        }
        visit(Piece::Pointer(relocation));
        start = offset + usize::try_from(relocation.size(8)).map_err(|_| std::fmt::Error)?;
    }
    if start < bytes.len() {
        visit(Piece::Bytes(&bytes[start..]));
    }
    Ok(())
}
