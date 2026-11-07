//! Reduces semantic analysis's retained types to the debug tables' own
//! ([`DebugTypeData`]), for `-g` and `-g2`.
//!
//! A type is described the first time something names it, so an unused one
//! takes no space, as `-feliminate-unused-debug-types` makes GCC's default and
//! Clang's. Types are shared and memoized by their `TypeId`, and a structure
//! enters the table before its members, so one that points at itself ends.
//! The recursion follows the nesting of the source's declarators, which
//! `--max-nesting-depth` bounds, on the compilation stack.
//!
//! A retained type has seen through its typedefs: `my_t x` has `int`'s
//! `TypeId`. What the source wrote survives only in the declaration's
//! specifiers, so [`Unit::note_typedef_use`] records which typedef a binding's
//! specifiers name, and describing the binding's type substitutes the typedef
//! where its type appears, under any pointers and arrays of the declarator. A
//! typedef of a typedef, and a member of a record, name the final type.
//!
//! C99: §6.2.5, pp. 33-37; PDF pp. 45-49 (types); §6.7.7, pp. 123-124; PDF
//! pp. 135-136 (typedef names).

use super::module_items::Unit;
use crate::{
    ir::{
        DebugEncoding,
        DebugEnumerator,
        DebugFile,
        DebugMember,
        DebugQualifier,
        DebugRecord,
        DebugType,
        DebugTypeData,
    },
    target::Scalar,
    translation_phases::{
        parsing::declaration_syntax::{
            DeclarationSpecifiers,
            TypeQualifiers,
            TypeSpecifiers,
        },
        semantic_analysis::{
            ArrayBound,
            TagKind,
            TypeId,
            TypeKind,
        },
    },
    util::bump::ArenaVec,
};

impl<'ir> Unit<'_, '_, 'ir> {
    /// Records that `binding`'s declaration names a typedef in `specifiers`,
    /// if it does: of the typedefs with that name, the innermost whose type
    /// the binding's type holds.
    pub(super) fn note_typedef_use(
        &mut self,
        specifiers: &DeclarationSpecifiers<'_>,
        binding: usize,
    ) {
        let ty = self.sema.bindings[binding].ty;
        let typedef = self.typedef_named(specifiers, ty);
        if let Some(typedef) = typedef {
            _ = self.debug_typedefs.insert(binding, typedef);
        }
    }

    /// The description of `binding`'s type, with the typedef its declaration
    /// names.
    pub(super) fn debug_binding_type(&mut self, binding: usize) -> Option<DebugType> {
        let ty = self.sema.bindings[binding].ty;
        let typedef = self.debug_typedefs.get(&binding).copied();
        self.debug_type_naming(ty, typedef)
    }

    /// The description of `ty`, with `typedef` where its type appears.
    pub(super) fn debug_type_naming(
        &mut self,
        ty: TypeId,
        typedef: Option<usize>,
    ) -> Option<DebugType> {
        let Some(typedef) = typedef.filter(|&typedef| {
            let named = self.sema.bindings[typedef].ty;
            self.type_holds(ty, named)
        }) else {
            return self.debug_type(ty);
        };
        let named = self.sema.bindings[typedef].ty;
        if ty.index == named.index {
            let node = Some(self.debug_typedef(typedef));
            return self.qualified(node, ty.qualifiers.difference(named.qualifiers));
        }
        let qualifiers = ty.qualifiers;
        let node = match self.sema.types.kind(ty) {
            | TypeKind::Pointer(next) => {
                let pointee = self.debug_type_naming(next, Some(typedef));
                Some(self.add_debug_type(DebugTypeData::Pointer { pointee }))
            },
            | TypeKind::Atomic(next) => {
                let ty = self.debug_type_naming(next, Some(typedef));
                Some(self.add_debug_type(DebugTypeData::Qualified {
                    qualifier: DebugQualifier::Atomic,
                    ty,
                }))
            },
            | TypeKind::Array(next, bound) => {
                let element = self.debug_type_naming(next, Some(typedef));
                Some(self.array_type(ty, element, bound))
            },
            | TypeKind::Function {
                result,
                parameters,
                variadic,
                prototype,
            } => {
                let result = self.debug_type_naming(result, Some(typedef));
                Some(self.function_type(result, parameters, &[], (variadic, prototype)))
            },
            | _ => self.debug_type(ty.unqualified()),
        };
        self.qualified(node, qualifiers)
    }

    /// The description of the function type `ty`, the type of a definition
    /// whose result and parameters name the typedefs `result_typedef` and
    /// `parameter_typedefs`.
    pub(super) fn debug_function_type(
        &mut self,
        ty: TypeId,
        result_typedef: Option<usize>,
        parameter_typedefs: &[Option<usize>],
    ) -> Option<DebugType> {
        let TypeKind::Function {
            result,
            parameters,
            variadic,
            prototype,
        } = self.sema.types.kind(ty)
        else {
            return self.debug_type(ty);
        };
        let result = self.debug_type_naming(result, result_typedef);
        Some(self.function_type(
            result,
            parameters,
            parameter_typedefs,
            (variadic, prototype),
        ))
    }

    /// The typedef `specifiers` name that `ty` holds, the innermost if several.
    pub(super) fn typedef_named(
        &self,
        specifiers: &DeclarationSpecifiers<'_>,
        ty: TypeId,
    ) -> Option<usize> {
        let TypeSpecifiers::TypedefName(name) = specifiers.type_specifiers else {
            return None;
        };
        let candidates = self.debug_typedef_names.get(&name.name)?;
        candidates
            .iter()
            .rev()
            .copied()
            .find(|&candidate| self.type_holds(ty, self.sema.bindings[candidate].ty))
    }

    /// Whether `ty` is `target` under pointers, arrays and function results.
    fn type_holds(&self, mut ty: TypeId, target: TypeId) -> bool {
        loop {
            if ty.index == target.index {
                return true;
            }
            ty = match self.sema.types.kind(ty) {
                | TypeKind::Pointer(next)
                | TypeKind::Atomic(next)
                | TypeKind::Array(next, _)
                | TypeKind::Function { result: next, .. } => next,
                | _ => return false,
            };
        }
    }

    fn debug_typedef(&mut self, typedef: usize) -> DebugType {
        if let Some(&node) = self.debug_typedef_nodes.get(&typedef) {
            return node;
        }
        let data = self.sema.bindings[typedef];
        let ty = self.debug_type(data.ty.unqualified());
        let ty = self.qualified(ty, data.ty.qualifiers);
        let vector = self.context.file_vector(data.name.source_vectors);
        let (line, file) = (vector.line, self.debug_file(vector.source_file_index));
        let name = self.debug_name(self.context.string_cache.at(data.name.name));
        let node = self.add_debug_type(DebugTypeData::Typedef {
            name,
            ty,
            file,
            line,
        });
        _ = self.debug_typedef_nodes.insert(typedef, node);
        node
    }

    /// The description of `ty`, or `None` for `void` and for what no
    /// description fits.
    pub(super) fn debug_type(&mut self, ty: TypeId) -> Option<DebugType> {
        let key = ty.with_alignment(0);
        if let Some(&node) = self.debug_types.get(&key) {
            return Some(node);
        }
        let node = if key.qualifiers.is_empty() {
            self.new_debug_type(key)
        } else {
            let unqualified = self.debug_type(key.unqualified());
            self.qualified(unqualified, key.qualifiers)
        };
        if let Some(node) = node {
            _ = self.debug_types.insert(key, node);
        }
        node
    }

    fn new_debug_type(&mut self, ty: TypeId) -> Option<DebugType> {
        match self.sema.types.kind(ty) {
            | TypeKind::Unknown | TypeKind::Unmodeled(_) => None,
            | TypeKind::Nullptr =>
                Some(self.add_debug_type(DebugTypeData::Pointer { pointee: None })),
            | TypeKind::Scalar(scalar) => self.scalar_type(scalar),
            | TypeKind::Pointer(next) => {
                let pointee = self.debug_type(next);
                Some(self.add_debug_type(DebugTypeData::Pointer { pointee }))
            },
            | TypeKind::Array(next, bound) => {
                let element = self.debug_type(next);
                Some(self.array_type(ty, element, bound))
            },
            | TypeKind::Function {
                result,
                parameters,
                variadic,
                prototype,
            } => {
                let result = self.debug_type(result);
                Some(self.function_type(result, parameters, &[], (variadic, prototype)))
            },
            | TypeKind::Tag(tag) => Some(self.tag_type(ty, tag)),
            | TypeKind::Atomic(next) => {
                let ty = self.debug_type(next);
                Some(self.add_debug_type(DebugTypeData::Qualified {
                    qualifier: DebugQualifier::Atomic,
                    ty,
                }))
            },
            | TypeKind::BitInt { width, signed } => {
                let prefix = if signed { "" } else { "unsigned " };
                let name = crate::diagnostics::format_in!(self.tables, "{prefix}_BitInt({width})");
                let name = self.debug_name(name);
                let (size_bits, _) = self.size_and_alignment(ty);
                Some(self.add_debug_type(DebugTypeData::Base {
                    name,
                    size_bits,
                    encoding: if signed {
                        DebugEncoding::Signed
                    } else {
                        DebugEncoding::Unsigned
                    },
                }))
            },
            | TypeKind::Vector { element, count, .. } => {
                let element = self.debug_type(element);
                let (size_bits, align_bits) = self.size_and_alignment(ty);
                Some(self.add_debug_type(DebugTypeData::Vector {
                    element,
                    count,
                    size_bits,
                    align_bits,
                }))
            },
        }
    }

    fn scalar_type(&mut self, scalar: Scalar) -> Option<DebugType> {
        if scalar == Scalar::Void {
            return None;
        }
        let layout = self.sema.types.target;
        let spelling = scalar.spelling();
        let encoding = match scalar {
            | Scalar::Bool => DebugEncoding::Boolean,
            | Scalar::Char if layout.char_signed => DebugEncoding::SignedChar,
            | Scalar::SignedChar => DebugEncoding::SignedChar,
            | Scalar::Char | Scalar::UnsignedChar => DebugEncoding::UnsignedChar,
            | _ if spelling.contains("_Complex") => DebugEncoding::Complex,
            | _ => match layout.integer(scalar) {
                | Some((_, true)) => DebugEncoding::Signed,
                | Some((_, false)) => DebugEncoding::Unsigned,
                | None => DebugEncoding::Float,
            },
        };
        let size_bits = layout.scalar(scalar).map_or(0, |layout| layout.size * 8);
        let name = self.debug_name(spelling);
        Some(self.add_debug_type(DebugTypeData::Base {
            name,
            size_bits,
            encoding,
        }))
    }

    fn array_type(
        &mut self,
        ty: TypeId,
        element: Option<DebugType>,
        bound: ArrayBound,
    ) -> DebugType {
        let count = match bound {
            | ArrayBound::Constant(count) => Some(count),
            | ArrayBound::Incomplete | ArrayBound::Variable(_) | ArrayBound::Star => None,
        };
        let (size_bits, align_bits) = self.size_and_alignment(ty);
        self.add_debug_type(DebugTypeData::Array {
            element,
            count,
            size_bits,
            align_bits,
        })
    }

    fn function_type(
        &mut self,
        result: Option<DebugType>,
        parameters: &[TypeId],
        typedefs: &[Option<usize>],
        (variadic, prototyped): (bool, bool),
    ) -> DebugType {
        let mut described = ArenaVec::with_capacity_in(parameters.len(), self.tables);
        for (index, &parameter) in parameters.iter().enumerate() {
            let typedef = typedefs.get(index).copied().flatten();
            described.push(self.debug_type_naming(parameter, typedef));
        }
        let parameters = self.module.debug_info_mut().1.alloc_slice_copy(&described);
        self.add_debug_type(DebugTypeData::Function {
            result,
            parameters,
            variadic,
            prototyped,
        })
    }

    fn tag_type(&mut self, ty: TypeId, tag: usize) -> DebugType {
        let data = self.sema.types.tags[tag];
        let name = data
            .name
            .map(|name| self.debug_name(self.context.string_cache.at(name)));
        let (size_bits, align_bits) = self.size_and_alignment(ty);
        let complete = data.complete.get() && data.layout.get().is_some();
        let (file, line) = self.tag_position(data.members.get());
        if data.kind == TagKind::Enum {
            let compatible = data.compatible.get();
            let base = complete.then(|| self.scalar_type(compatible)).flatten();
            let enumerators = self.enumerators_of(tag);
            return self.add_debug_type(DebugTypeData::Enumeration {
                name,
                file,
                line,
                size_bits,
                align_bits,
                base,
                enumerators,
            });
        }
        let union = data.kind == TagKind::Union;
        let incomplete = DebugRecord {
            union,
            name,
            file,
            line,
            size_bits,
            align_bits,
            complete: false,
            members: &[],
        };
        // In the memo before its members, one of which may point back at it.
        let node = self.add_debug_type(DebugTypeData::Record(incomplete));
        _ = self.debug_types.insert(ty, node);
        if !complete {
            return node;
        }
        let mut members = ArenaVec::new_in(self.tables);
        for member in data.members.get() {
            // An unnamed bit-field only holds space. C99 §6.7.2.1p12, p. 102;
            // PDF p. 114.
            if member.invalid || (member.name.is_none() && !member.anonymous) {
                continue;
            }
            let name = member
                .name
                .map(|name| self.debug_name(self.context.string_cache.at(name.name)));
            let ty = self.debug_type(member.ty);
            let line = if member.source_vectors.length() == 0 {
                0
            } else {
                self.context.file_vector(member.source_vectors).line
            };
            members.push(DebugMember {
                name,
                ty,
                line,
                offset_bits: member.offset * 8 + u64::from(member.bit_offset),
                bit_field: member.width,
            });
        }
        let members = self.module.debug_info_mut().1.alloc_slice_copy(&members);
        self.module.debug_info_mut().0.set_type(
            node,
            DebugTypeData::Record(DebugRecord {
                complete: true,
                members,
                ..incomplete
            }),
        );
        node
    }

    /// Where a tag is declared, as near as what is retained says: its first
    /// member's line, or the main file for a type without one.
    fn tag_position(
        &mut self,
        members: &[crate::translation_phases::semantic_analysis::Member],
    ) -> (DebugFile, u32) {
        let first = members
            .iter()
            .find(|member| member.source_vectors.length() != 0);
        match first {
            | Some(member) => {
                let vector = self.context.file_vector(member.source_vectors);
                let line = vector.line;
                (self.debug_file(vector.source_file_index), line)
            },
            | None => (self.debug_file(super::debug_locations::MAIN_FILE_INDEX), 0),
        }
    }

    fn enumerators_of(&mut self, tag: usize) -> &'ir [DebugEnumerator<'ir>] {
        let mut enumerators = ArenaVec::new_in(self.tables);
        for enumerator in self.sema.enumerators.iter().filter(|item| item.tag == tag) {
            // DWARF's constant holds what the compatible type does; a full
            // width unsigned value wraps as Clang's does.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "An enumerator holds at most 64 bits, which semantic analysis checked."
            )]
            let value = enumerator.value as i64;
            enumerators.push(DebugEnumerator {
                name: self.debug_name(self.context.string_cache.at(enumerator.name)),
                value,
            });
        }
        self.module
            .debug_info_mut()
            .1
            .alloc_slice_copy(&enumerators)
    }

    fn size_and_alignment(&self, ty: TypeId) -> (u64, u32) {
        let types = &self.sema.types;
        let size = types.layout(ty).map_or(0, |layout| layout.size * 8);
        let align = types
            .natural_alignment(ty)
            .map_or(0, |align| u32::try_from(align * 8).unwrap_or(0));
        (size, align)
    }

    fn qualified(
        &mut self,
        mut node: Option<DebugType>,
        qualifiers: TypeQualifiers,
    ) -> Option<DebugType> {
        // As GCC and Clang nest them when printed: `const volatile int`.
        for (flag, qualifier) in [
            (TypeQualifiers::RESTRICT, DebugQualifier::Restrict),
            (TypeQualifiers::VOLATILE, DebugQualifier::Volatile),
            (TypeQualifiers::CONST, DebugQualifier::Const),
        ] {
            if qualifiers.contains(flag) {
                node = Some(self.add_debug_type(DebugTypeData::Qualified {
                    qualifier,
                    ty: node,
                }));
            }
        }
        node
    }

    fn add_debug_type(&mut self, data: DebugTypeData<'ir>) -> DebugType {
        self.module.debug_info_mut().0.add_type(data)
    }

    /// `name` in the module's arena.
    fn debug_name(&mut self, name: &str) -> &'ir str {
        self.module.debug_info_mut().1.alloc_str(name)
    }
}
