//! File-scope GNU basic assembly, carried into the module as text. Clang
//! concatenates every block, in source order, into the module's inline
//! assembly, which the assembler sees before any function or object; the
//! back ends keep that order. A block lowering cannot represent faithfully
//! is refused with its provenance, never dropped.
//! C99: extension to §6.9, p. 140; PDF p. 152 (external declarations).
//! GNU extension: GCC manual, "Basic Asm", outside functions.
//! <https://gcc.gnu.org/onlinedocs/gcc/Basic-Asm.html>

use super::{
    ArenaVec,
    Construct,
    LoweringError,
    Unit,
};
use crate::translation_phases::{
    parsing::{
        ExternalDeclaration,
        ParsedTranslationUnit,
    },
    preprocessing::{
        StringTokenType,
        TokenType,
    },
};

impl Unit<'_, '_, '_> {
    /// Appends each file-scope `asm("...")` to the module in source order.
    /// Only a plain narrow template without NUL is accepted: Clang rejects
    /// qualifiers, operands and wide templates at file scope, and the text
    /// reaches the assembler as UTF-8 lines.
    pub(super) fn file_scope_asm(
        &mut self,
        syntax: &ParsedTranslationUnit<'_>,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) {
        for root in syntax.external_declarations() {
            let ExternalDeclaration::Asm(asm) = *root else {
                continue;
            };
            let plain = !asm.recovered
                && asm.sections == 0
                && !asm.qualifiers.volatile
                && !asm.qualifiers.inline
                && !asm.qualifiers.goto
                && asm.operands.is_empty()
                && asm.clobbers.is_empty()
                && asm.labels.is_empty();
            let text = match asm.template.map(|token| token.kind) {
                | Some(TokenType::String(StringTokenType::String(id))) if plain => self
                    .context
                    .literal_text_in(self.tables, id, false)
                    .filter(|text| !text.contains('\0')),
                | _ => None,
            };
            match text {
                | Some(text) => self.module.add_module_asm(text),
                | None => errors.push(LoweringError::unsupported(
                    Construct::FileScopeAssembly,
                    asm.source_vectors,
                )),
            }
        }
    }
}
