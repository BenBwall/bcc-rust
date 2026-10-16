//! Global declarations and packed initializers with byte-exact relocations.
use super::{
    EmitError,
    Emitter,
    api::{
        Session,
        Ty,
    },
};
use crate::{
    ir::{
        Entity,
        Global,
        GlobalId,
        GlobalInit,
        Relocation,
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
            | Piece::Pointer(_) => api.ty(Type::Ptr),
        });
    })?;
    Ok(api.structure(&fields, true))
}

impl<'m> Emitter<'_, '_, 'm, '_> {
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
                        | Piece::Pointer(relocation) => {
                            let symbol = match relocation.symbol {
                                | Symbol::Function(func) => self.functions[func.index()],
                                | Symbol::Global(global) => self.globals[global.index()],
                            };
                            if relocation.addend == 0 {
                                symbol
                            } else {
                                api.const_gep(symbol, relocation.addend)
                            }
                        },
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
        if pair[0].offset + 8 > pair[1].offset {
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
        start = offset + 8;
    }
    if start < bytes.len() {
        visit(Piece::Bytes(&bytes[start..]));
    }
    Ok(())
}
