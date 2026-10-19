//! Checked C library builtins: the prototypes of the object-size-checking
//! aliases in `builtins/checked.rs`, whose `va_list` and `FILE *` parameters
//! depend on the target and on the program's own `<stdio.h>`. Calls are then
//! checked as ordinary prototyped calls, and lower to the libc `__*_chk`
//! entry point, which checks the destination size it is passed.
//!
//! GNU extension: GCC "Object Size Checking".
//! <https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html>
//! C99: §7.15p3, p. 249; PDF p. 261 (`va_list`); §6.7.5.3p7, p. 119;
//! PDF p. 130 (array parameter adjustment); §7.19.1p2, p. 262; PDF p. 274
//! (`FILE`); §6.5.2.2p7, p. 72; PDF p. 84 (prototyped arguments).

use super::Identifier;
use crate::{
    builtins::{
        BuiltinDescriptor,
        SignatureType,
    },
    translation_phases::semantic_analysis::{
        Analyzer,
        BindingKind,
        SemanticErrorKind,
        TypeId,
        TypeKind,
        scopes::Namespace,
    },
};

impl Analyzer<'_, '_, '_> {
    /// Whether `descriptor`'s prototype can be formed where `name` is first
    /// used: a `FILE *` parameter needs the program's file-scope `FILE`
    /// typedef. An unavailable builtin is diagnosed here and stays
    /// unmodeled.
    pub(in crate::translation_phases::semantic_analysis) fn builtin_prototype_available(
        &mut self,
        descriptor: &BuiltinDescriptor,
        name: Identifier,
    ) -> bool {
        let needs_file = descriptor
            .signature
            .parameters
            .contains(&SignatureType::File);
        if !needs_file || self.stdio_file().is_some() {
            return true;
        }
        self.error(
            SemanticErrorKind::BuiltinRequiresHeader,
            name.source_vectors,
            Some(name.name),
            None,
        );
        false
    }

    /// The adjusted parameter type of the target's `__builtin_va_list`: a
    /// pointer to its element when it is an array, else the list itself.
    pub(super) fn va_list_parameter(&mut self) -> TypeId {
        let list = self.builtin_va_list();
        match self.types.nodes[list.index] {
            | TypeKind::Array(element, _) => self.types.intern(TypeKind::Pointer(element)),
            | _ => list,
        }
    }

    /// `FILE *` for the file-scope `FILE` typedef; with none, a pointer to an
    /// unanalyzed type, which [`Self::builtin_prototype_available`] has
    /// already diagnosed.
    pub(super) fn file_parameter(&mut self) -> TypeId {
        let file = self.stdio_file().unwrap_or_else(|| self.types.unknown());
        self.types.intern(TypeKind::Pointer(file))
    }

    /// The type the file-scope ordinary identifier `FILE` names as a typedef.
    /// A block-scope shadow cannot change the C library prototype.
    fn stdio_file(&mut self) -> Option<TypeId> {
        let name = self.context.string_cache.intern("FILE");
        let entry = self.lookup_at(Namespace::Ordinary, name, 0)?;
        let binding = self.bindings[entry.binding];
        (binding.kind == BindingKind::Typedef).then_some(binding.ty)
    }
}
