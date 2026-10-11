//! Resource-header intrinsic types and expressions, without backend lowering.
//! C99: §7.15, pp. 249-252; PDF pp. 261-264; §7.17p3, p. 254;
//! PDF p. 266. Traversal remains on the semantic work stack.

use super::{
    Analyzer,
    ArrayBound,
    Cell,
    ConstantClass,
    Expression,
    ExpressionInfo,
    Integer,
    Scalar,
    SemanticErrorKind,
    SyntaxOperand,
    Tag,
    TagKind,
    TypeId,
    TypeKind,
    modeled,
};
use crate::translation_phases::{
    parsing::{
        Builtin,
        OffsetMember,
    },
    preprocessing::KeywordTokenType,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// C99: §7.15.1.1p2, pp. 249-250; PDF pp. 261-262; §7.15.1.4p4,
    /// p. 251; PDF p. 263. Runtime argument availability remains lowering work.
    /// Failed va-list operands leave an unknown result for error recovery.
    pub(in crate::translation_phases::semantic_analysis) fn type_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        b: &'tu Builtin<'tu>,
    ) -> ExpressionInfo<'tu> {
        use KeywordTokenType as K;
        if matches!(b.keyword, K::BuiltinTypesCompatible | K::BuiltinChooseExpr) {
            return self.type_generic_builtin(e, b);
        }
        if matches!(b.keyword, K::BuiltinBitCast | K::BuiltinConvertVector) {
            self.context.report_extension(
                crate::configuration::Feature::VectorBuiltins,
                "vector builtin",
                b.source_vectors,
            );
        }
        if b.keyword == K::BuiltinBitCast {
            return self.bit_cast_builtin(e, b);
        }
        if b.keyword == K::BuiltinConvertVector {
            return self.convert_vector_builtin(e, b);
        }
        if b.keyword == K::BuiltinOffsetof {
            return self.type_offsetof(e, b);
        }
        if !modeled(b.keyword) || b.recovered {
            return Self::expression_result(e, self.types.unknown());
        }
        let list = self.builtin_va_list();
        let pointer = match self.types.nodes[list.index] {
            | TypeKind::Array(record, _) => self.types.intern(TypeKind::Pointer(record)),
            | TypeKind::Pointer(_) => list,
            | _ => unreachable!("target va_list is an array or pointer"),
        };
        let mut failed_operand = false;
        for (index, &operand) in b.operands.iter().enumerate() {
            if index != 0 && b.keyword != K::BuiltinVaCopy {
                continue;
            }
            if let SyntaxOperand::Expression(operand) = operand {
                let info = self.expression_info(operand);
                let ty = self.converted(info);
                let requires_lvalue = self.types.target.va_list_is_pointer;
                if self.types.unanalyzed(ty) {
                    failed_operand = true;
                } else if ty != pointer
                    || (requires_lvalue
                        && info.category != super::expressions::ValueCategory::ModifiableLvalue)
                {
                    failed_operand = true;
                    self.error(
                        SemanticErrorKind::InvalidVaList,
                        operand.source_vectors,
                        None,
                        None,
                    );
                }
            }
        }
        if b.keyword == K::BuiltinVaStart {
            let variadic = self
                .functions
                .current
                .and_then(|f| f.binding)
                .is_some_and(|id| {
                    matches!(
                        self.types.nodes[self.bindings[id].ty.index],
                        TypeKind::Function { variadic: true, .. }
                    )
                });
            if !variadic {
                self.error(
                    SemanticErrorKind::VaStartOutsideVariadic,
                    e.source_vectors,
                    None,
                    None,
                );
            }
        }
        let mut result = self.types.scalar(Scalar::Void);
        if b.keyword == K::BuiltinVaArg
            && let Some(&SyntaxOperand::Type(name)) = b.operands.get(1)
        {
            result = self
                .resolved_type_names
                .get(&name.source_vectors)
                .copied()
                .unwrap_or_else(|| self.types.unknown());
            if !self.types.unanalyzed(result)
                && (!self.complete_object(result)
                    || matches!(self.types.nodes[result.index], TypeKind::Array(..)))
            {
                self.error(
                    SemanticErrorKind::InvalidVaArgType,
                    name.source_vectors,
                    None,
                    None,
                );
                result = self.types.unknown();
            }
        }
        if failed_operand {
            result = self.types.unknown();
        }
        Self::expression_result(e, result)
    }

    /// Resolves a member/index path iteratively; constant paths yield a
    /// `size_t` ICE. GNU runtime indices yield an ordinary `size_t` value.
    /// C99: §7.17p3, p. 254; PDF p. 266.
    fn type_offsetof(
        &mut self,
        e: &'tu Expression<'tu>,
        b: &'tu Builtin<'tu>,
    ) -> ExpressionInfo<'tu> {
        let Some(&SyntaxOperand::Type(name)) = b.operands.first() else {
            return Self::expression_result(e, self.types.unknown());
        };
        let mut ty = self
            .resolved_type_names
            .get(&name.source_vectors)
            .copied()
            .unwrap_or_else(|| self.types.unknown());
        if self.types.unanalyzed(ty) {
            return Self::expression_result(e, ty);
        }
        let mut offset = Some(0_u64);
        let mut valid = true;
        for &member in b.members {
            match member {
                | OffsetMember::Field(name) => {
                    let TypeKind::Tag(id) = self.types.nodes[ty.index] else {
                        valid = false;
                        break;
                    };
                    #[cfg(test)]
                    self.review_step(1);
                    let Some(member) = self
                        .member_indices
                        .get(&(id, name.name))
                        .and_then(|&index| self.types.tags[id].fields.get().get(index))
                    else {
                        valid = false;
                        break;
                    };
                    if member.width.is_some() {
                        valid = false;
                        break;
                    }
                    let computed = offset.and_then(|n| n.checked_add(member.offset));
                    valid &= offset.is_none() || computed.is_some();
                    offset = computed;
                    ty = member.ty;
                },
                | OffsetMember::Index(index) => {
                    let TypeKind::Array(element, _) = self.types.nodes[ty.index] else {
                        valid = false;
                        break;
                    };
                    let info = self.expression_info(index);
                    let index_ty = self.converted(info);
                    if self.types.unanalyzed(index_ty) {
                        return Self::expression_result(e, self.types.unknown());
                    }
                    if self.integer_type(index_ty).is_none() {
                        valid = false;
                        break;
                    }
                    // The GNU intrinsic accepts runtime integer paths; the
                    // standard macro's address-constant contract remains
                    // C99 §7.17p3, p. 254; PDF p. 266.
                    if !info.ice {
                        offset = None;
                        ty = element;
                        continue;
                    }
                    let value = info.integer.filter(|_| info.ice).and_then(Integer::to_u64);
                    let computed = offset.and_then(|n| {
                        value.and_then(|value| {
                            self.types
                                .layout(element)?
                                .size
                                .checked_mul(value)?
                                .checked_add(n)
                        })
                    });
                    valid &= offset.is_none() || computed.is_some();
                    offset = computed;
                    ty = element;
                },
            }
        }
        if !valid {
            self.error(
                SemanticErrorKind::InvalidOffsetof,
                e.source_vectors,
                None,
                None,
            );
            return Self::expression_result(e, self.types.unknown());
        }
        let ty = self.types.scalar(self.types.target.size_t);
        let mut info = Self::expression_result(e, ty);
        let Some(offset) = offset else {
            return info;
        };
        let (bits, signed) = self.types.target.integer(self.types.target.size_t).unwrap();
        info.integer = Some(Integer {
            value: i128::from(offset),
            bits,
            signed,
        });
        info.ice = true;
        info.constant = ConstantClass::Arithmetic;
        info
    }

    /// Opaque `SysV` record, wrapped in an array of one so parameter adjustment
    /// and expression decay use the ordinary array rules. C99: extension
    /// supporting §7.15p3, p. 249; PDF p. 261.
    pub(in crate::translation_phases::semantic_analysis) fn builtin_va_list(&mut self) -> TypeId {
        if let Some(ty) = self.va_list_type {
            return ty;
        }
        if self.types.target.va_list_is_pointer {
            let char_type = self.types.scalar(Scalar::Char);
            let ty = self.types.intern(TypeKind::Pointer(char_type));
            self.va_list_type = Some(ty);
            return ty;
        }
        let name = self.context.string_cache.intern("__builtin_va_list");
        let id = {
            let id = self.types.tags.len();
            self.types.tags.push(self.types.tu.alloc(Tag {
                name:              Some(name),
                kind:              TagKind::Struct,
                members:           Cell::new(&[]),
                fields:            Cell::new(&[]),
                layout:            Cell::new(Some(self.types.target.va_list)),
                complete:          Cell::new(true),
                tainted:           Cell::new(false),
                contains_flexible: Cell::new(false),
                compatible:        Cell::new(Scalar::Int),
            }));
            id
        };
        let record = self.types.intern(TypeKind::Tag(id));
        let ty = self
            .types
            .intern(TypeKind::Array(record, ArrayBound::Constant(1)));
        self.va_list_type = Some(ty);
        ty
    }

    /// GNU typeof preserves the operand's declared type, without decay.
    /// GNU extension: GCC manual, "Typeof".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Typeof.html>
    /// C23: §6.7.3.6 paragraphs 4-5, p. 118; PDF p. 131.
    pub(in crate::translation_phases::semantic_analysis) fn operand_type(
        &self,
        operand: SyntaxOperand<'tu>,
    ) -> TypeId {
        match operand {
            | SyntaxOperand::Expression(e) => self.expression_info(e).ty,
            | SyntaxOperand::Type(name) => self
                .resolved_type_names
                .get(&name.source_vectors)
                .copied()
                .unwrap_or_else(|| self.types.unknown()),
        }
    }
}
