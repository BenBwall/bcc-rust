//! File-scope GNU basic assembly, carried into the module as text. Clang
//! concatenates every block, in source order, into the module's inline
//! assembly, which the assembler sees before any function or object; the
//! back ends keep that order. An invalid block, or one lowering cannot
//! represent faithfully, is reported with its provenance, never dropped.
//! C99: extension to §6.9, p. 140; PDF p. 152 (external declarations).
//! GNU extension: GCC manual, "Basic Asm", outside functions.
//! <https://gcc.gnu.org/onlinedocs/gcc/Basic-Asm.html>

use super::{
    super::errors::FileScopeAsmFault,
    ArenaVec,
    Construct,
    LoweringError,
    LoweringErrorKind,
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

impl<'a> Unit<'a, '_, '_> {
    /// Appends each file-scope `asm("...")` to the module in source order.
    pub(super) fn file_scope_asm(
        &mut self,
        syntax: &ParsedTranslationUnit<'_>,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) {
        for &root in syntax.external_declarations() {
            let ExternalDeclaration::Asm(asm) = root else {
                continue;
            };
            match self.asm_text(root) {
                | Ok(text) => self.module.add_module_asm(text),
                | Err(kind) => errors.push(LoweringError {
                    kind,
                    source: Some(asm.source_vectors),
                }),
            }
        }
    }

    /// The text of a file-scope `asm` root, or why it has none. Qualifiers,
    /// operands and wide or Unicode templates are invalid, reported in
    /// Clang's words; a template with NUL or bytes that are not UTF-8 is
    /// valid but cannot reach the assembler as text, and is unsupported.
    fn asm_text(&self, root: ExternalDeclaration<'_>) -> Result<&'a str, LoweringErrorKind> {
        let invalid = |fault| Err(LoweringErrorKind::InvalidFileScopeAsm(fault));
        let ExternalDeclaration::Asm(asm) = root else {
            return Err(LoweringErrorKind::MissingFact("a file-scope asm root"));
        };
        let qualifiers = asm.qualifiers;
        if qualifiers.volatile {
            return invalid(FileScopeAsmFault::Volatile);
        }
        if qualifiers.inline {
            return invalid(FileScopeAsmFault::Inline);
        }
        if qualifiers.goto {
            return invalid(FileScopeAsmFault::Goto);
        }
        if asm.recovered
            || asm.sections != 0
            || !asm.operands.is_empty()
            || !asm.clobbers.is_empty()
            || !asm.labels.is_empty()
        {
            return invalid(FileScopeAsmFault::Operands);
        }
        let id = match asm.template.map(|token| token.kind) {
            | Some(TokenType::String(StringTokenType::String(id))) => id,
            | Some(TokenType::String(StringTokenType::WideString(_))) =>
                return invalid(FileScopeAsmFault::WideTemplate),
            | Some(TokenType::String(StringTokenType::EncodedString(..))) =>
                return invalid(FileScopeAsmFault::UnicodeTemplate),
            // The parser reports a missing template before lowering runs;
            // refuse rather than drop one that slips through.
            | _ => return invalid(FileScopeAsmFault::Operands),
        };
        self.context
            .literal_text_in(self.tables, id, false)
            .filter(|text| !text.contains('\0'))
            .ok_or(LoweringErrorKind::Unsupported(Construct::FileScopeAssembly))
    }
}
