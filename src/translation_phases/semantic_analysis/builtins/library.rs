//! Prototypes for a small set of GCC library builtins, before ordinary call
//! checking and conversions. Their names lower to the corresponding C symbols.
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
        let mut codes = signature.bytes();
        let result = self.library_type(codes.next()?);
        let mut parameters = ArenaVec::new_in(self.types.tu);
        for code in codes {
            parameters.push(self.library_type(code));
        }
        Some(self.types.intern(TypeKind::Function {
            result,
            parameters: parameters.leak(),
            prototype: true,
            variadic,
        }))
    }

    fn library_type(&mut self, code: u8) -> TypeId {
        let scalar = match code {
            | b'i' => Scalar::Int,
            | b'z' => self.types.target.size_t,
            | b'd' => Scalar::Double,
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

/// Result first, then parameters: v=void, i=int, `z=size_t`, d=double,
/// p/P=void*/const void*, s/S=char*/const char*.
fn library_builtin(name: &str) -> Option<(&'static str, &'static str, bool)> {
    Some(match name {
        | "__builtin_abort" => ("abort", "v", false),
        | "__builtin_exit" => ("exit", "vi", false),
        | "__builtin_malloc" => ("malloc", "pz", false),
        | "__builtin_calloc" => ("calloc", "pzz", false),
        | "__builtin_free" => ("free", "vp", false),
        | "__builtin_memcpy" => ("memcpy", "ppPz", false),
        | "__builtin_memmove" => ("memmove", "ppPz", false),
        | "__builtin_memset" => ("memset", "ppiz", false),
        | "__builtin_memcmp" => ("memcmp", "iPPz", false),
        | "__builtin_memchr" => ("memchr", "pPiz", false),
        | "__builtin_strcmp" => ("strcmp", "iSS", false),
        | "__builtin_strncmp" => ("strncmp", "iSSz", false),
        | "__builtin_strcpy" => ("strcpy", "ssS", false),
        | "__builtin_strncpy" => ("strncpy", "ssSz", false),
        | "__builtin_strchr" => ("strchr", "sSi", false),
        | "__builtin_strrchr" => ("strrchr", "sSi", false),
        | "__builtin_strlen" => ("strlen", "zS", false),
        | "__builtin_printf" => ("printf", "iS", true),
        | "__builtin_sprintf" => ("sprintf", "isS", true),
        | "__builtin_puts" => ("puts", "iS", false),
        | "__builtin_putchar" => ("putchar", "ii", false),
        | "__builtin_abs" => ("abs", "ii", false),
        | "__builtin_fabs" => ("fabs", "dd", false),
        | "__builtin_floor" => ("floor", "dd", false),
        | _ => return None,
    })
}
