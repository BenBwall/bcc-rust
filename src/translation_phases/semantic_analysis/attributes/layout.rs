//! Phase-7 GNU representation attributes. Attribute names are read only at
//! list depth; alignment operands use the semantic constant evaluator after
//! their expression syntax has been visited by the explicit work stack.
//! C99: §6.7, p. 97; PDF p. 109; §6.7.2.1p10, p. 102; PDF p. 114.
//! GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html>.

use super::super::{
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
pub(in crate::translation_phases::semantic_analysis) struct LayoutAttributes {
    pub(in crate::translation_phases::semantic_analysis) packed:    bool,
    pub(in crate::translation_phases::semantic_analysis) alignment: u32,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// GCC member attributes may follow a complete declarator, including
    /// grouped declarators. Walk those attribute lists without recursion.
    pub(in crate::translation_phases::semantic_analysis) fn packed_declarator(
        &mut self,
        declarator: Declarator<'tu>,
    ) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(declarator);
        while let Some(declarator) = work.pop() {
            for direct in declarator.kind {
                match direct {
                    | DirectDeclarator::Attributes(a)
                        if super::known_names(a, self.context)
                            .any(|(_, name)| name == "packed") =>
                        return true,
                    | DirectDeclarator::Parenthesized(p) => work.push(p.declarator),
                    | _ => {},
                }
            }
        }
        false
    }

    /// Alignment written on this member, separate from its typedef's type
    /// alignment. Operands have already been checked during type construction;
    /// this retention step must not issue their diagnostics a second time.
    /// GCC Common Variable Attributes, packed; C11: §6.7.5p2-6,
    /// p. 127; PDF p. 145 (explicit _Alignas).
    pub(in crate::translation_phases::semantic_analysis) fn explicit_member_alignment(
        &self,
        extensions: Option<&'tu SpecifierExtension<'tu>>,
        attributes: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) -> u32 {
        let mut alignment = self
            .written_alignment(extensions)
            .max(self.written_alignment(attributes));
        let mut work = ArenaVec::new_in(self.scratch);
        if let Some(declarator) = declarator {
            work.push(declarator);
        }
        while let Some(declarator) = work.pop() {
            for direct in declarator.kind {
                match direct {
                    | DirectDeclarator::Attributes(a) =>
                        alignment = alignment.max(self.written_attribute_alignment(a)),
                    | DirectDeclarator::Parenthesized(p) => work.push(p.declarator),
                    | _ => {},
                }
            }
        }
        alignment
    }

    fn written_alignment(&self, mut chain: Option<&'tu SpecifierExtension<'tu>>) -> u32 {
        let mut alignment = 0;
        while let Some(item) = chain {
            let requested = match item.kind {
                | SpecifierExtensionKind::Attributes(a) => self.written_attribute_alignment(a),
                | SpecifierExtensionKind::Alignment(operand) => {
                    let value = match operand {
                        | super::super::SyntaxOperand::Expression(e) => self
                            .expression_info(e)
                            .integer
                            .and_then(super::super::Integer::to_u64),
                        | super::super::SyntaxOperand::Type(_) =>
                            self.types.alignment(self.operand_type(operand)),
                    };
                    value
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(0)
                },
                | _ => 0,
            };
            alignment = alignment.max(requested);
            chain = item.next;
        }
        alignment
    }

    fn written_attribute_alignment(&self, a: &AttributeSpecifier<'tu>) -> u32 {
        super::known_names(a, self.context)
            .filter(|(_, name)| *name == "aligned")
            .filter_map(|(index, _)| self.requested_attribute_alignment(a, index))
            .filter_map(|value| u32::try_from(value).ok())
            .max()
            .unwrap_or(0)
    }

    fn requested_attribute_alignment(
        &self,
        a: &AttributeSpecifier<'tu>,
        index: usize,
    ) -> Option<u64> {
        if a.tokens
            .get(index + 1)
            .is_some_and(|token| self.context.string_cache.at(token.contents) == "(")
        {
            self.attribute_alignment(a, index)
        } else {
            Some(16)
        }
    }

    /// Schedule parsed alignment operands before consuming representation
    /// facts. C99: §6.6p6, p. 95; PDF p. 107 (GNU ICE operand extension).
    pub(in crate::translation_phases::semantic_analysis) fn attribute_operands(
        &mut self,
        attribute: &AttributeSpecifier<'tu>,
    ) {
        for argument in attribute.arguments.iter().rev() {
            self.work
                .push(super::super::Work::Expression(argument.expression));
        }
    }

    /// Attribute chains share the ordinary iterative expression/type-name
    /// traversal, including sizeof, alignof, enum constants and arithmetic.
    pub(in crate::translation_phases::semantic_analysis) fn attribute_chain_operands(
        &mut self,
        mut chain: Option<&'tu SpecifierExtension<'tu>>,
    ) {
        while let Some(item) = chain {
            if let SpecifierExtensionKind::Attributes(attribute) = item.kind {
                self.attribute_operands(attribute);
            }
            chain = item.next;
        }
    }

    /// GNU aligned requires an integer constant expression; the caller also
    /// checks the positive power-of-two and target-representability rules.
    /// C99: §6.6p6, p. 95; PDF p. 107.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html> (aligned).
    pub(in crate::translation_phases::semantic_analysis) fn attribute_alignment(
        &self,
        attribute: &AttributeSpecifier<'tu>,
        index: usize,
    ) -> Option<u64> {
        let argument = attribute
            .arguments
            .iter()
            .find(|argument| argument.index == index)?;
        let info = self.expression_info(argument.expression);
        info.integer
            .filter(|_| info.ice)
            .and_then(super::super::Integer::to_u64)
    }

    /// GCC packed is member/record layout; aligned is a power-of-two byte
    /// alignment, defaulting to the x86-64 ABI's maximum scalar alignment.
    pub(in crate::translation_phases::semantic_analysis) fn layout_attributes(
        &mut self,
        a: &AttributeSpecifier<'tu>,
    ) -> LayoutAttributes {
        let mut result = LayoutAttributes::default();
        let mut invalid = false;
        for (index, name) in super::known_names(a, self.context) {
            if name == "packed" {
                result.packed = true;
            } else if name == "aligned" {
                let value = self.requested_attribute_alignment(a, index);
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
    pub(in crate::translation_phases::semantic_analysis) fn layout_attribute_chain(
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

    /// GCC mode selects an integer representation, preserving signedness, or
    /// a real or complex floating representation. Vector modes
    /// belong to the vector subsystem.
    /// C99: §6.2.5p4-11, pp. 33-34; PDF pp. 45-46 (extended scalar types).
    pub(in crate::translation_phases::semantic_analysis) fn mode_attribute(
        &mut self,
        mut ty: TypeId,
        a: &AttributeSpecifier<'tu>,
    ) -> TypeId {
        let mut invalid = false;
        for (index, name) in super::known_names(a, self.context) {
            if name != "mode" {
                continue;
            }
            let mode = a.tokens.get(index + 2).map(|token| {
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
            } else if mode_bits.is_none() {
                scalar.and_then(|scalar| self.floating_mode(scalar, mode?))
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

    /// The floating scalar a GNU real (`SF`, `DF`, `XF`, `TF`, `KF`) or
    /// complex (`SC`, `DC`, `XC`, `TC`, `KC`) mode selects for `base`, as
    /// Clang resolves it for the target. A real mode applies to any floating
    /// base, real or complex, and a complex mode only to a complex base. `X`
    /// names the x87 extended format, `T` the 128-bit format (binary128 when
    /// `long double` is not one), and `K` binary128.
    /// GCC internals, "Machine Modes";
    /// <https://gcc.gnu.org/onlinedocs/gccint/Machine-Modes.html>.
    /// C99: complex types §6.2.5p11, p. 34; PDF p. 46.
    fn floating_mode(&self, base: Scalar, mode: &str) -> Option<Scalar> {
        use Scalar as S;
        let complex = match mode.as_bytes() {
            | [_, b'F'] => false,
            | [_, b'C'] => true,
            | _ => return None,
        };
        let base_complex = matches!(
            base,
            S::ComplexFloat | S::ComplexDouble | S::ComplexLongDouble | S::ComplexFloat128
        );
        let base_real = matches!(base, S::Float | S::Double | S::LongDouble | S::Float128);
        if !(base_complex || (base_real && !complex)) {
            return None;
        }
        let target = self.context.configuration.target();
        let x87 = !target.layout().long_double_is_double();
        let real = match &mode[..1] {
            | "S" => S::Float,
            | "D" => S::Double,
            | "X" if x87 => S::LongDouble,
            | "T" | "K" if target.float128_type() => S::Float128,
            | _ => return None,
        };
        Some(match (complex, real) {
            | (false, real) => real,
            | (true, S::Float) => S::ComplexFloat,
            | (true, S::Double) => S::ComplexDouble,
            | (true, S::LongDouble) => S::ComplexLongDouble,
            | (true, _) => S::ComplexFloat128,
        })
    }

    /// Apply storage/type alignment after the complete declarator. Alignment
    /// on a function declaration does not change its return type.
    pub(in crate::translation_phases::semantic_analysis) fn align_declared_type(
        &mut self,
        ty: TypeId,
        alignment: u32,
    ) -> TypeId {
        if matches!(self.types.nodes[ty.index], TypeKind::Function { .. }) {
            return ty;
        }
        ty.with_alignment(ty.alignment.max(alignment))
    }

    /// GCC aligned cannot weaken a variable's requirement. A typedef may
    /// lower it, and declarations using that typedef inherit its alignment.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html>
    pub(in crate::translation_phases::semantic_analysis) fn minimum_declared_alignment(
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
    pub(in crate::translation_phases::semantic_analysis) fn standard_alignment(
        &mut self,
        ty: TypeId,
        mut chain: Option<&'tu SpecifierExtension<'tu>>,
    ) -> TypeId {
        // C11 §6.7.5p4,p6: compare the combined request with the original
        // requirement, not with the alignment strengthened by an earlier
        // specifier. The effective request is independent of source order.
        // Array alignment depends on the element type even when a VLA has
        // no compile-time size; typedefs retain that same requirement.
        let natural = self.types.alignment(ty);
        let mut requested = 0;
        let mut source = None;
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
                    | super::super::SyntaxOperand::Expression(expression) => self
                        .expression_info(expression)
                        .integer
                        .filter(|_| self.expression_info(expression).ice)
                        .and_then(super::super::Integer::to_u64),
                    | super::super::SyntaxOperand::Type(_) =>
                        self.types.alignment(self.operand_type(operand)),
                };
                if value != Some(0) {
                    if let Some(value) =
                        value.filter(|value| value.is_power_of_two() && *value <= 1 << 31)
                    {
                        requested = requested.max(u32::try_from(value).unwrap_or(0));
                        source = Some(item.source_vectors);
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
        if requested != 0 && natural.is_some_and(|natural| u64::from(requested) < natural) {
            self.error(
                SemanticErrorKind::InvalidStandardAlignment,
                source.unwrap_or_default(),
                None,
                None,
            );
            ty
        } else {
            ty.with_alignment(ty.alignment.max(requested))
        }
    }

    /// C11 §6.7.5p2, p. 127; PDF p. 145: alignment is an object
    /// declaration specifier, forbidden on typedefs, register objects,
    /// parameters, bit-fields and functions.
    pub(in crate::translation_phases::semantic_analysis) fn reject_standard_alignment(
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
