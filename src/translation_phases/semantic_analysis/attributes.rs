//! Phase-7 GNU representation attributes. Attribute names are read only at
//! list depth; arguments use the arena-backed iterative integer reducer.
//! C99: §6.7, p. 97; PDF p. 109; §6.7.2.1p10, p. 102; PDF p. 114.
//! GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html>.

use super::{
    Analyzer,
    ArenaVec,
    AttributeSpecifier,
    Declarator,
    DirectDeclarator,
    SemanticErrorKind,
    SpecifierExtension,
    SpecifierExtensionKind,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};

/// Representation facts independent of a declaration's underlying C type.
#[derive(Clone, Copy, Default)]
pub(super) struct LayoutAttributes {
    pub(super) packed:    bool,
    pub(super) alignment: u32,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// GCC member attributes may follow a complete declarator, including
    /// grouped declarators. Walk those attribute lists without recursion.
    pub(super) fn packed_declarator(&mut self, declarator: Declarator<'tu>) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(declarator);
        while let Some(declarator) = work.pop() {
            for direct in declarator.kind {
                match direct {
                    | DirectDeclarator::Attributes(a) if self.layout_attributes(a).packed =>
                        return true,
                    | DirectDeclarator::Parenthesized(p) => work.push(p.declarator),
                    | _ => {},
                }
            }
        }
        false
    }

    /// GCC packed is member/record layout; aligned is a power-of-two byte
    /// alignment, defaulting to the x86-64 ABI's maximum scalar alignment.
    pub(super) fn layout_attributes(&mut self, a: &AttributeSpecifier<'tu>) -> LayoutAttributes {
        let mut result = LayoutAttributes::default();
        let mut invalid = false;
        for (index, name) in super::vectors::attribute_names(a, self.context) {
            if name == "packed" {
                result.packed = true;
            } else if name == "aligned" {
                let tokens = a.tokens.as_slice();
                let argument = tokens
                    .get(index + 1)
                    .is_some_and(|token| self.context.string_cache.at(token.contents) == "(");
                let value = if argument {
                    self.vector_attribute_integer(&tokens[index + 2..])
                } else {
                    Some(16)
                };
                if let Some(value) =
                    value.filter(|value| value.is_power_of_two() && *value <= 1 << 31)
                {
                    result.alignment = result.alignment.max(u32::try_from(value).unwrap_or(0));
                } else {
                    invalid = true;
                }
            }
        }
        if invalid {
            self.error(
                SemanticErrorKind::InvalidAlignmentAttribute,
                a.source_vectors,
                None,
                None,
            );
        }
        result
    }

    /// Combine separate attribute lists without interpreting argument names
    /// as attributes. GCC Common Attributes: packed and aligned compose.
    pub(super) fn layout_attribute_chain(
        &mut self,
        mut chain: Option<&'tu SpecifierExtension<'tu>>,
    ) -> LayoutAttributes {
        let mut result = LayoutAttributes::default();
        while let Some(item) = chain {
            if let SpecifierExtensionKind::Attributes(a) = item.kind {
                let attributes = self.layout_attributes(a);
                result.packed |= attributes.packed;
                result.alignment = result.alignment.max(attributes.alignment);
            }
            chain = item.next;
        }
        result
    }

    /// GCC mode selects an integer or real scalar representation, preserving
    /// signedness. Vector/complex modes require their separate subsystems.
    /// C99: §6.2.5p4-11, pp. 33-34; PDF pp. 45-46 (extended scalar types).
    pub(super) fn mode_attribute(&mut self, mut ty: TypeId, a: &AttributeSpecifier<'tu>) -> TypeId {
        let mut invalid = false;
        for (index, name) in super::vectors::attribute_names(a, self.context) {
            if name != "mode" {
                continue;
            }
            let mode = a.tokens.as_slice().get(index + 2).map(|token| {
                self.context
                    .string_cache
                    .at(token.contents)
                    .trim_matches('_')
            });
            let scalar = match self.types.nodes[ty.index] {
                | TypeKind::Scalar(scalar) => Some(scalar),
                | _ => None,
            };
            let integer = self.integer_type(ty);
            let mode_bits = match mode {
                | Some("QI" | "byte") => Some(8),
                | Some("HI") => Some(16),
                | Some("SI") => Some(32),
                | Some("DI" | "word" | "pointer") => Some(64),
                | Some("TI") => Some(128),
                | _ => None,
            };
            let selected = if let (Some(bits), Some((_, signed))) = (mode_bits, integer) {
                if scalar == Some(Scalar::Bool) && bits == 8 {
                    Some(Scalar::Bool)
                } else if scalar == Some(Scalar::Bool) {
                    None
                } else {
                    Some(match (bits, signed) {
                        | (8, true) => Scalar::SignedChar,
                        | (8, false) => Scalar::UnsignedChar,
                        | (16, true) => Scalar::Short,
                        | (16, false) => Scalar::UnsignedShort,
                        | (32, true) => Scalar::Int,
                        | (32, false) => Scalar::UnsignedInt,
                        | (64, true) => self.types.target.intmax_t,
                        | (64, false) => self.types.target.uintmax_t,
                        | (_, true) => Scalar::Int128,
                        | (_, false) => Scalar::UnsignedInt128,
                    })
                }
            } else if matches!(
                scalar,
                Some(Scalar::Float | Scalar::Double | Scalar::LongDouble)
            ) {
                match mode {
                    | Some("SF") => Some(Scalar::Float),
                    | Some("DF") => Some(Scalar::Double),
                    | Some("XF") => Some(Scalar::LongDouble),
                    | _ => None,
                }
            } else {
                None
            };
            let Some(selected) = selected else {
                invalid = true;
                break;
            };
            ty = self
                .types
                .scalar(selected)
                .qualified(ty.qualifiers)
                .with_alignment(ty.alignment);
        }
        if invalid {
            self.error(
                SemanticErrorKind::InvalidModeAttribute,
                a.source_vectors,
                None,
                None,
            );
            return self.types.unknown();
        }
        ty
    }

    /// Apply storage/type alignment after the complete declarator. Alignment
    /// on a function declaration does not change its return type.
    pub(super) fn align_declared_type(&mut self, ty: TypeId, alignment: u32) -> TypeId {
        if matches!(self.types.nodes[ty.index], TypeKind::Function { .. }) {
            return ty;
        }
        ty.with_alignment(ty.alignment.max(alignment))
    }

    /// GCC aligned cannot weaken a variable's requirement. A typedef may
    /// lower it, and declarations using that typedef inherit its alignment.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html>
    pub(super) fn minimum_declared_alignment(
        &self,
        ty: TypeId,
        base: TypeId,
        can_weaken: bool,
    ) -> TypeId {
        if can_weaken || ty.alignment == 0 {
            return ty;
        }
        let minimum = if matches!(
            self.types.nodes[ty.index],
            TypeKind::Scalar(_) | TypeKind::Tag(_)
        ) && base.alignment != 0
        {
            self.types.layout(base)
        } else {
            self.types.layout(ty.with_alignment(0))
        };
        if let Some(minimum) = minimum {
            u32::try_from(minimum.align).map_or(ty.with_alignment(0), |alignment| {
                ty.with_alignment(ty.alignment.max(alignment))
            })
        } else {
            ty
        }
    }

    /// C11 §6.7.5p2-6 (p. 127; PDF p. 145): alignas zero is
    /// ineffective; other operands specify valid alignment without weakening
    /// the natural requirement. Operands were typed by the explicit work stack.
    pub(super) fn standard_alignment(
        &mut self,
        mut ty: TypeId,
        mut chain: Option<&'tu SpecifierExtension<'tu>>,
    ) -> TypeId {
        while let Some(item) = chain {
            if let SpecifierExtensionKind::Alignment(operand) = item.kind {
                if matches!(self.types.nodes[ty.index], TypeKind::Function { .. }) {
                    self.error(
                        SemanticErrorKind::InvalidStandardAlignment,
                        item.source_vectors,
                        None,
                        None,
                    );
                    chain = item.next;
                    continue;
                }
                let value = match operand {
                    | super::SyntaxOperand::Expression(expression) => self
                        .expression_info(expression)
                        .integer
                        .filter(|_| self.expression_info(expression).ice)
                        .and_then(super::Integer::to_u64),
                    | super::SyntaxOperand::Type(_) => self
                        .types
                        .layout(self.operand_type(operand))
                        .map(|layout| layout.align),
                };
                if value != Some(0) {
                    if let Some(value) = value.filter(|value| {
                        value.is_power_of_two()
                            && *value <= 1 << 31
                            && self
                                .types
                                .layout(ty)
                                .is_none_or(|layout| *value >= layout.align)
                    }) {
                        ty = ty.with_alignment(ty.alignment.max(u32::try_from(value).unwrap_or(0)));
                    } else {
                        self.error(
                            SemanticErrorKind::InvalidStandardAlignment,
                            item.source_vectors,
                            None,
                            None,
                        );
                    }
                }
            }
            chain = item.next;
        }
        ty
    }

    /// C11 §6.7.5p2, p. 127; PDF p. 145: alignment is an object
    /// declaration specifier, forbidden on typedefs, register objects,
    /// parameters, bit-fields and functions.
    pub(super) fn reject_standard_alignment(
        &mut self,
        mut chain: Option<&'tu SpecifierExtension<'tu>>,
    ) {
        while let Some(item) = chain {
            if matches!(item.kind, SpecifierExtensionKind::Alignment(_)) {
                self.error(
                    SemanticErrorKind::InvalidStandardAlignment,
                    item.source_vectors,
                    None,
                    None,
                );
            }
            chain = item.next;
        }
    }
}
