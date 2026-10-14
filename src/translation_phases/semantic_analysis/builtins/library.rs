//! Library prototypes for implicit declarations and the existing GCC aliases,
//! before ordinary call checking and conversions. Aliases lower to C symbols.
//! Unlisted intrinsics retain their opaque types and unsupported diagnostics.
//!
//! GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html>.
//! C99: §7.1.4, p. 166; PDF p. 178 (library functions);
//! §6.5.2.2 paragraphs 2 and 7, pp. 71-72; PDF pp. 83-84 (prototypes).

use super::Identifier;
use crate::{
    translation_phases::{
        parsing::declaration_syntax::TypeQualifiers,
        semantic_analysis::{
            Analyzer,
            Scalar,
            TypeId,
            TypeKind,
        },
    },
    util::bump::ArenaVec,
};

impl Analyzer<'_, '_, '_> {
    /// Reserved library aliases have the complete prototype of the library
    /// function, including the target's `size_t` and const-qualified pointees.
    /// GNU extension: GCC "Library Builtins".
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84.
    pub(in crate::translation_phases::semantic_analysis) fn library_builtin_type(
        &mut self,
        name: Identifier,
    ) -> Option<TypeId> {
        let (_, signature, variadic) = library_builtin(self.context.string_cache.at(name.name))?;
        Some(self.library_prototype(signature, variadic))
    }

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
        let (_, signature, variadic) = library_function(self.context.string_cache.at(name.name))?;
        let ty = self.library_prototype(signature, variadic);
        let mut spelling = crate::util::bump::ArenaString::new_in(self.scratch);
        spelling.push_str(self.library_type_spelling(signature.as_bytes()[0]));
        spelling.push_str(" (");
        for (index, code) in signature.bytes().skip(1).enumerate() {
            if index != 0 {
                spelling.push_str(", ");
            }
            spelling.push_str(self.library_type_spelling(code));
        }
        if variadic {
            spelling.push_str(", ...");
        } else if signature.len() == 1 {
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

    fn library_prototype(&mut self, signature: &str, variadic: bool) -> TypeId {
        let mut codes = signature.bytes();
        let result = self.library_type(codes.next().unwrap_or(b'v'));
        let mut parameters = ArenaVec::new_in(self.types.tu);
        for code in codes {
            parameters.push(self.library_type(code));
        }
        self.types.intern(TypeKind::Function {
            result,
            parameters: parameters.leak(),
            prototype: true,
            variadic,
        })
    }

    fn library_type_spelling(&self, code: u8) -> &'static str {
        match code {
            | b'i' => "int",
            | b'z' => self.types.target.size_t.spelling(),
            | b'd' => "double",
            | b'f' => "float",
            | b'l' => "long double",
            | b'p' => "void *",
            | b'P' => "const void *",
            | b's' => "char *",
            | b'S' => "const char *",
            | _ => "void",
        }
    }

    fn library_type(&mut self, code: u8) -> TypeId {
        let scalar = match code {
            | b'i' => Scalar::Int,
            | b'z' => self.types.target.size_t,
            | b'd' => Scalar::Double,
            | b'f' => Scalar::Float,
            | b'l' => Scalar::LongDouble,
            | b's' | b'S' => Scalar::Char,
            | _ => Scalar::Void,
        };
        let mut ty = self.types.scalar(scalar);
        if matches!(code, b'P' | b'S') {
            ty = ty.qualified(TypeQualifiers::CONST);
        }
        if matches!(code, b'p' | b'P' | b's' | b'S') {
            ty = self.types.intern(TypeKind::Pointer(ty));
        }
        ty
    }
}

/// The runtime symbol for a supported __builtin_ library alias.
/// GNU extension: GCC "Library Builtins" guarantees identical type, address,
/// and meaning for these aliases. C99: §7.1.4, p. 166; PDF p. 178.
pub(crate) fn library_builtin_name(name: &str) -> Option<&'static str> {
    library_builtin(name).map(|(name, _, _)| name)
}

/// Result first, then parameters: v=void, i=int, `z=size_t`, f=float,
/// d=double, l=long double,
/// p/P=void*/const void*, s/S=char*/const char*.
fn library_builtin(name: &str) -> Option<(&'static str, &'static str, bool)> {
    library_function(name.strip_prefix("__builtin_")?)
}

/// Standard library prototypes shared by aliases and implicit declarations.
/// C99: §7.1.4, p. 166; PDF p. 178.
fn library_function(name: &str) -> Option<(&'static str, &'static str, bool)> {
    Some(match name {
        | "abort" => ("abort", "v", false),
        | "exit" => ("exit", "vi", false),
        | "malloc" => ("malloc", "pz", false),
        | "calloc" => ("calloc", "pzz", false),
        | "free" => ("free", "vp", false),
        | "memcpy" => ("memcpy", "ppPz", false),
        | "memmove" => ("memmove", "ppPz", false),
        | "memset" => ("memset", "ppiz", false),
        | "memcmp" => ("memcmp", "iPPz", false),
        | "memchr" => ("memchr", "pPiz", false),
        | "strcmp" => ("strcmp", "iSS", false),
        | "strncmp" => ("strncmp", "iSSz", false),
        | "strcpy" => ("strcpy", "ssS", false),
        | "strncpy" => ("strncpy", "ssSz", false),
        | "strchr" => ("strchr", "sSi", false),
        | "strrchr" => ("strrchr", "sSi", false),
        | "strlen" => ("strlen", "zS", false),
        | "printf" => ("printf", "iS", true),
        | "sprintf" => ("sprintf", "isS", true),
        | "snprintf" => ("snprintf", "iszS", true),
        | "puts" => ("puts", "iS", false),
        | "putchar" => ("putchar", "ii", false),
        | "abs" => ("abs", "ii", false),
        | "fabs" => ("fabs", "dd", false),
        | "fabsf" => ("fabsf", "ff", false),
        | "fabsl" => ("fabsl", "ll", false),
        | "floor" => ("floor", "dd", false),
        | _ => return None,
    })
}
