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
};
use crate::translation_phases::{
    parsing::{
        Builtin,
        OffsetMember,
    },
    preprocessing::KeywordTokenType,
};

/// Intrinsics supporting standard headers are available even in strict modes.
pub(super) fn modeled(keyword: KeywordTokenType) -> bool {
    matches!(
        keyword,
        KeywordTokenType::BuiltinVaArg
            | KeywordTokenType::BuiltinVaStart
            | KeywordTokenType::BuiltinVaEnd
            | KeywordTokenType::BuiltinVaCopy
            | KeywordTokenType::BuiltinOffsetof
    )
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Opaque `SysV` record, wrapped in an array of one so parameter adjustment
    /// and expression decay use the ordinary array rules. C99: extension
    /// supporting §7.15p3, p. 249; PDF p. 261.
    pub(super) fn builtin_va_list(&mut self) -> TypeId {
        if let Some(ty) = self.va_list_type {
            return ty;
        }
        let name = self.context.string_cache.intern("__builtin_va_list");
        let id = {
            let id = self.types.tags.len();
            self.types.tags.push(self.types.tu.alloc(Tag {
                name:              Some(name),
                kind:              TagKind::Struct,
                members:           Cell::new(&[]),
                layout:            Cell::new(Some(self.types.target.va_list)),
                complete:          Cell::new(true),
                tainted:           Cell::new(false),
                contains_flexible: Cell::new(false),
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

    /// C99: §7.15.1.1p2, pp. 249-250; PDF pp. 261-262; §7.15.1.4p4,
    /// p. 251; PDF p. 263. Runtime argument availability remains lowering work.
    pub(super) fn type_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        b: &'tu Builtin<'tu>,
    ) -> ExpressionInfo<'tu> {
        use KeywordTokenType as K;
        if b.keyword == K::BuiltinOffsetof {
            return self.type_offsetof(e, b);
        }
        if !modeled(b.keyword) || b.recovered {
            return Self::expression_result(e, self.types.unknown());
        }
        let list = self.builtin_va_list();
        let TypeKind::Array(record, _) = self.types.nodes[list.index] else {
            unreachable!()
        };
        let pointer = self.types.intern(TypeKind::Pointer(record));
        for (index, &operand) in b.operands.iter().enumerate() {
            if index != 0 && b.keyword != K::BuiltinVaCopy {
                continue;
            }
            if let SyntaxOperand::Expression(operand) = operand {
                let info = self.expression_info(operand);
                let ty = self.converted(info);
                if !self.types.unanalyzed(ty) && ty != pointer {
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
        Self::expression_result(e, result)
    }

    /// Resolves a member/index path iteratively and yields a `size_t` ICE.
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
        for &member in b.members {
            match member {
                | OffsetMember::Field(name) => {
                    let TypeKind::Tag(id) = self.types.nodes[ty.index] else {
                        offset = None;
                        break;
                    };
                    let Some(member) =
                        self.types.tags[id].members.get().iter().find(|member| {
                            member.name.is_some_and(|field| field.name == name.name)
                        })
                    else {
                        offset = None;
                        break;
                    };
                    if member.width.is_some() {
                        offset = None;
                        break;
                    }
                    offset = offset.and_then(|n| n.checked_add(member.offset));
                    ty = member.ty;
                },
                | OffsetMember::Index(index) => {
                    let TypeKind::Array(element, _) = self.types.nodes[ty.index] else {
                        offset = None;
                        break;
                    };
                    let info = self.expression_info(index);
                    let value = info
                        .integer
                        .filter(|_| info.ice)
                        .and_then(|n| u64::try_from(n.value).ok());
                    offset = offset.and_then(|n| {
                        value.and_then(|value| {
                            self.types
                                .layout(element)?
                                .size
                                .checked_mul(value)?
                                .checked_add(n)
                        })
                    });
                    ty = element;
                },
            }
        }
        let Some(offset) = offset else {
            self.error(
                SemanticErrorKind::InvalidOffsetof,
                e.source_vectors,
                None,
                None,
            );
            return Self::expression_result(e, self.types.unknown());
        };
        let ty = self.types.scalar(self.types.target.size_t);
        let mut info = Self::expression_result(e, ty);
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
}
