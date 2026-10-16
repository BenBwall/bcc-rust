//! Library prototypes for implicit declarations and the existing GCC aliases,
//! before ordinary call checking and conversions. Aliases lower to C symbols.
//! Unlisted intrinsics retain their opaque types and unsupported diagnostics.
//!
//! GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html>.
//! C99: §7.1.4, p. 166; PDF p. 178 (library functions);
//! §6.5.2.2 paragraphs 2 and 7, pp. 71-72; PDF pp. 83-84 (prototypes).

use super::Identifier;
use crate::translation_phases::semantic_analysis::{
    Analyzer,
    Scalar,
    TypeId,
};

impl Analyzer<'_, '_, '_> {
    /// Clang-compatible implicit library declarations use the library type.
    /// This extension avoids incompatible calls under C99 §6.5.2.2 paragraphs
    /// 6 and 9, pp. 71-72; PDF pp. 83-84. C99 §7.1.4 paragraph 2,
    /// p. 167; PDF p. 179 permits declarations without the associated header.
    pub(in crate::translation_phases::semantic_analysis) fn implicit_library_type(
        &mut self,
        name: Identifier,
    ) -> Option<TypeId> {
        // Freestanding ordinary names retain C89 implicit int. Reserved
        // __builtin_ aliases keep their explicit GCC contract in either mode.
        // C99: §5.1.2.1p1, p. 11; PDF p. 23.
        if !self.context.configuration.hosted() {
            return None;
        }
        let descriptor = crate::builtins::ordinary(self.context.string_cache.at(name.name))?;
        let signature = descriptor.signature;
        let variadic = signature.variadic;
        let ty = self.descriptor_type(descriptor);
        let mut spelling = crate::util::bump::ArenaString::new_in(self.scratch);
        spelling.push_str(self.library_type_spelling(signature.result));
        spelling.push_str(" (");
        for (index, code) in signature.parameters.iter().copied().enumerate() {
            if index != 0 {
                spelling.push_str(", ");
            }
            spelling.push_str(self.library_type_spelling(code));
        }
        if variadic {
            spelling.push_str(", ...");
        } else if signature.parameters.is_empty() {
            spelling.push_str("void");
        }
        spelling.push(')');
        let spelling = self.context.string_cache.intern(spelling.as_str());
        if !self.context.withholds(
            crate::translation_phases::ErrorSeverity::Warning,
            false,
            name.source_vectors,
        ) {
            self.context.append_pending_errors([
                crate::translation_phases::TranslationError::Semantic(
                    super::super::SemanticError {
                        extension_severity: None,
                        kind:
                            super::super::SemanticErrorKind::ImplicitLibraryFunction(spelling),
                        source_vectors:     name.source_vectors,
                        name:               Some(name.name),
                        previous:           None,
                    },
                ),
            ]);
        }
        Some(ty)
    }

    fn library_type_spelling(&self, pattern: crate::builtins::SignatureType) -> &'static str {
        use crate::builtins::SignatureType as T;
        match pattern {
            | T::Scalar(scalar) => scalar.spelling(),
            | T::Size => self.types.target.size_t.spelling(),
            | T::Pointer(Scalar::Char, false) => "char *",
            | T::Pointer(Scalar::Char, true) => "const char *",
            | T::Pointer(_, false) => "void *",
            | T::Pointer(_, true) => "const void *",
            | T::Unsigned(_) => "unsigned long long",
            | T::VaList => "va_list",
            | T::File => "FILE *",
        }
    }
}
