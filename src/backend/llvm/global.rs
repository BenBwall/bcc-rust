//! Globals: declarations, and definitions with their bytes and address
//! constants.

use std::fmt;

use super::{
    EmitError,
    Emitter,
    syntax::{
        byte_string,
        symbol,
    },
};
use crate::{
    ir::{
        GlobalId,
        GlobalInit,
        Linkage,
        Relocation,
        Symbol,
    },
    util::bump::Bump,
};

impl<'m> Emitter<'_, 'm, '_> {
    /// Prints one global on its own line.
    ///
    /// A definition is `dso_local`, as clang marks definitions for the
    /// executables it links; an internal one is `internal`. A global without
    /// an initializer is an `external` declaration, since LLVM has no
    /// internal declarations. Contents are bytes, so the value type is
    /// `[N x i8]`, or the packed struct of [`Emitter::relocated`].
    pub(super) fn global(&mut self, global: GlobalId, scratch: &Bump) -> Result<(), EmitError<'m>> {
        let data = self.module.global(global);
        write!(self.out, "{} = ", symbol(data.name))?;
        let kind = if data.constant { "constant" } else { "global" };
        let Some(init) = data.init else {
            writeln!(
                self.out,
                "external {kind} [{} x i8], align {}",
                data.size,
                data.align.bytes()
            )?;
            return Ok(());
        };
        let linkage = match data.linkage {
            | Linkage::External => "dso_local",
            | Linkage::Internal => "internal",
        };
        write!(self.out, "{linkage} {kind} ")?;
        match init {
            | GlobalInit::Bytes { bytes, relocations } if !relocations.is_empty() =>
                self.relocated(data.name, bytes, relocations, scratch)?,
            | GlobalInit::Bytes { bytes, .. } if bytes.iter().any(|&byte| byte != 0) =>
                write!(self.out, "[{} x i8] {}", bytes.len(), byte_string(bytes))?,
            | GlobalInit::Zero | GlobalInit::Bytes { .. } =>
                write!(self.out, "[{} x i8] zeroinitializer", data.size)?,
        }
        writeln!(self.out, ", align {}", data.align.bytes())?;
        Ok(())
    }

    /// Prints contents with address constants as a packed struct that
    /// alternates the byte runs between relocations with the relocated
    /// pointers, so each pointer lands at its exact offset:
    ///
    /// ```text
    /// <{ [8 x i8], ptr, [4 x i8] }> <{ [8 x i8] c"...", ptr @s, [4 x i8] c"..." }>
    /// ```
    ///
    /// Empty runs are left out. The relocations are visited in offset order;
    /// overlapping ones are an error, since only one pointer fits.
    fn relocated(
        &mut self,
        name: &'m str,
        bytes: &[u8],
        relocations: &[Relocation],
        scratch: &Bump,
    ) -> Result<(), EmitError<'m>> {
        let sorted = scratch.alloc_slice_copy(relocations);
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
        let pieces = Pieces {
            bytes,
            relocations: sorted,
        };
        write!(self.out, "<{{ ")?;
        pieces.visit(|piece, separator| match piece {
            | Piece::Bytes(run) => write!(self.out, "{separator}[{} x i8]", run.len()),
            | Piece::Pointer(_) => write!(self.out, "{separator}ptr"),
        })?;
        write!(self.out, " }}> <{{ ")?;
        pieces.visit(|piece, separator| match piece {
            | Piece::Bytes(run) => write!(
                self.out,
                "{separator}[{} x i8] {}",
                run.len(),
                byte_string(run)
            ),
            | Piece::Pointer(relocation) => {
                let target = match relocation.symbol {
                    | Symbol::Function(func) => self.module.function(func).name,
                    | Symbol::Global(global) => self.module.global(global).name,
                };
                if relocation.addend == 0 {
                    write!(self.out, "{separator}ptr {}", symbol(target))
                } else {
                    write!(
                        self.out,
                        "{separator}ptr getelementptr (i8, ptr {}, i64 {})",
                        symbol(target),
                        relocation.addend
                    )
                }
            },
        })?;
        write!(self.out, " }}>")?;
        Ok(())
    }
}

/// A global's contents split at its relocations, which are sorted and do not
/// overlap.
struct Pieces<'a> {
    bytes:       &'a [u8],
    relocations: &'a [Relocation],
}

/// A run of plain bytes or one relocated pointer.
enum Piece<'a> {
    Bytes(&'a [u8]),
    Pointer(Relocation),
}

impl<'a> Pieces<'a> {
    /// Calls `visit` with each nonempty piece in order and the separator to
    /// print before it.
    fn visit(&self, mut visit: impl FnMut(Piece<'a>, &str) -> fmt::Result) -> fmt::Result {
        let mut separator = "";
        let mut start = 0;
        for &relocation in self.relocations {
            let offset = usize::try_from(relocation.offset).map_err(|_| fmt::Error)?;
            if offset > start {
                visit(Piece::Bytes(&self.bytes[start..offset]), separator)?;
                separator = ", ";
            }
            visit(Piece::Pointer(relocation), separator)?;
            separator = ", ";
            start = offset + 8;
        }
        if start < self.bytes.len() {
            visit(Piece::Bytes(&self.bytes[start..]), separator)?;
        }
        Ok(())
    }
}
