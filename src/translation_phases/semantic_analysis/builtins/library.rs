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
    TypeKind,
    TypeQualifiers,
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
        let spelling = self.context.string_cache.at(name.name);
        if crate::builtins::extension_name(spelling) && !self.context.configuration.gnu_extensions()
        {
            return None;
        }
        // `-fno-builtin` and `-fno-builtin-NAME` make the name an ordinary
        // identifier, as in GCC and Clang: an undeclared call is an implicit
        // declaration again.
        match self.context.library_builtins {
            | crate::configuration::LibraryBuiltins::None => return None,
            | crate::configuration::LibraryBuiltins::Except(names) if names.contains(&spelling) =>
                return None,
            | _ => {},
        }
        let Some(descriptor) = crate::builtins::ordinary(spelling) else {
            let sized = sized_atomic(spelling)?;
            return self.sized_atomic_type(sized);
        };
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

    /// The type GCC gives the size-specific forms of its atomic builtins,
    /// `__atomic_store_16` and the like ([`sized_atomic`]), which are
    /// libatomic's entry points. Calls take it as an ordinary declaration
    /// of the library routine, which bcc calls, where GCC expands the
    /// narrower ones inline. Only GCC knows them; elsewhere a call is an
    /// implicit declaration. GNU extension: GCC manual, "`__atomic`
    /// Builtins"; libatomic's ABI.
    fn sized_atomic_type(&mut self, (shape, unit): (AtomicShape, Scalar)) -> Option<TypeId> {
        if !self
            .context
            .configuration
            .accepts(crate::configuration::Feature::GccOnlyBuiltins)
        {
            return None;
        }
        let unit = self.types.scalar(unit);
        let order = self.types.scalar(Scalar::Int);
        let void = self.types.scalar(Scalar::Void);
        // GCC's `volatile void *` operand, `const volatile void *` for a
        // load, and the plain `void *` of the expected value.
        let volatile = self
            .types
            .intern(TypeKind::Pointer(void.qualified(TypeQualifiers::VOLATILE)));
        let source = self.types.intern(TypeKind::Pointer(
            void.qualified(TypeQualifiers::CONST | TypeQualifiers::VOLATILE),
        ));
        let pointer = self.types.intern(TypeKind::Pointer(void));
        let truth = self.types.scalar(Scalar::Bool);
        let (result, parameters): (TypeId, &[TypeId]) = match shape {
            | AtomicShape::Load => (unit, &[source, order]),
            | AtomicShape::Store => (void, &[volatile, unit, order]),
            | AtomicShape::Exchange => (unit, &[volatile, unit, order]),
            | AtomicShape::CompareExchange =>
                (truth, &[volatile, pointer, unit, truth, order, order]),
        };
        let parameters = self.types.tu.alloc_slice_copy(parameters);
        Some(self.types.intern(TypeKind::Function {
            result,
            parameters,
            prototype: true,
            variadic: false,
        }))
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
            | T::Vector(..) | T::VectorPointer(..) | T::VaListPointer | T::Opaque =>
                unreachable!("ordinary library prototypes use C library types"),
        }
    }
}

/// The parameters and result of a size-specific atomic routine.
#[derive(Clone, Copy)]
enum AtomicShape {
    /// `I (const volatile void *, int)`.
    Load,
    /// `void (volatile void *, I, int)`.
    Store,
    /// `I (volatile void *, I, int)`: an exchange or a read-modify-write.
    Exchange,
    /// `bool (volatile void *, void *, I, bool, int, int)`.
    CompareExchange,
}

/// The shape of the libatomic routine `spelling` names, and its `I`: the
/// unsigned integer of the size in bytes the name ends with.
fn sized_atomic(spelling: &str) -> Option<(AtomicShape, Scalar)> {
    let (operation, size) = spelling.strip_prefix("__atomic_")?.rsplit_once('_')?;
    let unit = match size {
        | "1" => Scalar::UnsignedChar,
        | "2" => Scalar::UnsignedShort,
        | "4" => Scalar::UnsignedInt,
        | "8" => Scalar::UnsignedLongLong,
        | "16" => Scalar::UnsignedInt128,
        | _ => return None,
    };
    let shape = match operation {
        | "load" => AtomicShape::Load,
        | "store" => AtomicShape::Store,
        | "compare_exchange" => AtomicShape::CompareExchange,
        | "exchange" | "fetch_add" | "fetch_sub" | "fetch_and" | "fetch_or" | "fetch_xor"
        | "fetch_nand" | "add_fetch" | "sub_fetch" | "and_fetch" | "or_fetch" | "xor_fetch"
        | "nand_fetch" => AtomicShape::Exchange,
        | _ => return None,
    };
    Some((shape, unit))
}
