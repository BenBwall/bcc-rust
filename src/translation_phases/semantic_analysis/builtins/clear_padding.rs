//! GCC's `__builtin_clear_padding (T *)`: the checks on its operand and the
//! type it clears. GCC and Clang accept different operands and say so in
//! different words, so each identity follows its compiler.
//!
//! GCC lvalue converts the operand, so an array or function designator
//! decays, and asks for a pointer to a complete type that is neither `const`
//! nor `_Atomic`. It takes a pointer to a function, whose padding is empty;
//! a pointer to an array of `const` elements, which it does not see as
//! `const`; and a pointer to a variable length array. A flexible array
//! member anywhere in the object, or a member of variable size, it refuses.
//!
//! Clang wants an operand that already is a pointer, with no decay, and to a
//! type that is complete and not `const`, also through arrays. It refuses a
//! pointer to a function and a pointer to an array of atomics as not
//! trivially copyable, and a record with a flexible array member, looking
//! through nested records but not through arrays. A pointer to a variable
//! length array, or to an array of them, it takes and clears nothing.
//!
//! Clang 23 also leaves an array of constant length alone where its type was
//! spelled with a typedef, `typeof` or parentheses: `struct S (*p)[3]` is not
//! cleared and `&a` of `struct S a[3]` is. bcc's types do not keep that
//! spelling, so it clears them all, which changes only the padding Clang
//! left.
//!
//! The retained operation type is the type lowering clears: `void` where
//! the call clears nothing.
//!
//! GNU extension: GCC manual, "Other Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
//! C99: §6.2.6.1 paragraph 6, p. 38; PDF p. 50 (padding bytes).

use crate::{
    configuration::RuleSet,
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::Expression,
        },
        semantic_analysis::{
            Analyzer,
            ArrayBound,
            ExpressionInfo,
            Scalar,
            SemanticErrorKind,
            TagKind,
            TypeId,
            TypeKind,
        },
    },
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

const NAME: &str = "__builtin_clear_padding";

/// A part of an object that has no defined padding.
#[derive(Clone, Copy)]
enum Undefined {
    /// A flexible array member, with its name.
    Flexible(Option<StringCacheId>),
    /// A member of variable size, GNU's.
    VariableSize,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks a call of `__builtin_clear_padding`, as the module map says.
    /// GNU extension: GCC manual, "Other Builtins".
    pub(in crate::translation_phases::semantic_analysis) fn clear_padding_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        args: &[&'tu Expression<'tu>],
    ) -> ExpressionInfo<'tu> {
        let gcc = self.context.configuration.compiler_identity().rules() == RuleSet::Gcc;
        let &[argument] = args else {
            self.clear_padding_argument_count(e, args, gcc);
            return Self::expression_result(e, self.types.unknown());
        };
        let operand = self.expression_info(argument);
        let converted = self.converted(operand);
        if self.types.unanalyzed(operand.ty) || self.types.unanalyzed(converted) {
            return Self::expression_result(e, self.types.unknown());
        }
        let cleared = if gcc {
            let ty = self.types.non_atomic(converted);
            self.gcc_cleared_type(e, argument, ty)
        } else {
            self.clang_cleared_type(argument, operand.ty)
        };
        let Some(cleared) = cleared else {
            return Self::expression_result(e, self.types.unknown());
        };
        let mut info = Self::expression_result(e, self.types.scalar(Scalar::Void));
        info.operation_type = Some(cleared);
        info
    }

    /// Reports a call without exactly one argument, where Clang points at
    /// the first excess one.
    fn clear_padding_argument_count(
        &mut self,
        e: &'tu Expression<'tu>,
        args: &[&'tu Expression<'tu>],
        gcc: bool,
    ) {
        let (few_or_many, source) = match args.get(1) {
            | Some(excess) if !gcc => ("many", excess.source_vectors),
            | Some(_) => ("many", e.source_vectors),
            | None => ("few", e.source_vectors),
        };
        let text = if gcc {
            format_in!(
                self.scratch,
                "too {few_or_many} arguments to function '{NAME}'"
            )
        } else {
            format_in!(
                self.scratch,
                "too {few_or_many} arguments to function call, expected 1, have {}",
                args.len()
            )
        };
        self.clear_padding_error(source, text);
    }

    /// GCC's checks of the lvalue-converted operand type `ty`, in its order.
    fn gcc_cleared_type(
        &mut self,
        e: &'tu Expression<'tu>,
        argument: &'tu Expression<'tu>,
        ty: TypeId,
    ) -> Option<TypeId> {
        let source = argument.source_vectors;
        let void = self.types.scalar(Scalar::Void);
        let refused = format_in!(self.scratch, "argument 1 in call to function '{NAME}'");
        let TypeKind::Pointer(pointee) = self.types.nodes[ty.index] else {
            let text = if self.types.nodes[ty.index] == TypeKind::Scalar(Scalar::Void) {
                "invalid use of void expression"
            } else {
                format_in!(self.scratch, "{refused} does not have pointer type")
            };
            self.clear_padding_error(source, text);
            return None;
        };
        // Lowering refuses by name a pointee semantic analysis does not model.
        if self.types.unanalyzed(pointee) {
            return Some(pointee);
        }
        let kind = self.types.nodes[pointee.index];
        if matches!(kind, TypeKind::Function { .. }) {
            return Some(void);
        }
        if !self.types.complete_object(pointee) {
            let text = format_in!(self.scratch, "{refused} points to incomplete type");
            self.clear_padding_error(source, text);
            return None;
        }
        let array = matches!(kind, TypeKind::Array(..));
        if !array && pointee.qualifiers.contains(TypeQualifiers::CONST) {
            let text = match self.type_spelling(ty, true) {
                | Some(ty) => format_in!(
                    self.scratch,
                    "{refused} has pointer to 'const' type ('{ty}')"
                ),
                | None => format_in!(self.scratch, "{refused} has pointer to 'const' type"),
            };
            self.clear_padding_error(source, text);
            return None;
        }
        if let TypeKind::Atomic(value) = kind {
            let text = match self.type_spelling(value.qualified(pointee.qualifiers), true) {
                | Some(value) => format_in!(
                    self.scratch,
                    "{refused} has pointer to '_Atomic' type ('_Atomic {value} *')"
                ),
                | None => format_in!(self.scratch, "{refused} has pointer to '_Atomic' type"),
            };
            self.clear_padding_error(source, text);
            return None;
        }
        match self.undefined_padding(pointee, true) {
            | Some(Undefined::Flexible(member)) => {
                let member = member.map_or("", |name| self.context.string_cache.at(name));
                let text = format_in!(
                    self.scratch,
                    "flexible array member '{member}' does not have well defined padding bits for \
                     '{NAME}'"
                );
                self.clear_padding_error(e.source_vectors, text);
                return None;
            },
            | Some(Undefined::VariableSize) => {
                let text = format_in!(
                    self.scratch,
                    "sorry, unimplemented: {NAME} not supported for variable length aggregates"
                );
                self.clear_padding_error(e.source_vectors, text);
                return None;
            },
            | None => {},
        }
        // GCC folds a call on a type it sees no padding in away, and this
        // is the one such type that a member of a record clears.
        let mut element = pointee;
        while let TypeKind::Array(inner, _) = self.types.nodes[element.index] {
            element = inner;
        }
        Some(if self.types.nodes[element.index] == TypeKind::Nullptr {
            void
        } else {
            pointee
        })
    }

    /// Clang's checks of the unconverted operand type `ty`, in its order.
    fn clang_cleared_type(&mut self, argument: &'tu Expression<'tu>, ty: TypeId) -> Option<TypeId> {
        let source = argument.source_vectors;
        let TypeKind::Pointer(pointee) = self.types.nodes[ty.index] else {
            let text = match self.type_spelling(ty, false) {
                | Some(ty) => format_in!(
                    self.scratch,
                    "passing '{ty}' to parameter of incompatible type pointer: type mismatch at \
                     1st parameter ('{ty}' vs pointer)"
                ),
                | None => "passing an argument to parameter of incompatible type pointer",
            };
            self.clear_padding_error(source, text);
            return None;
        };
        if self.types.unanalyzed(pointee) {
            return Some(pointee);
        }
        let mut element = pointee;
        let mut read_only = pointee.qualifiers.contains(TypeQualifiers::CONST);
        while let TypeKind::Array(inner, _) = self.types.nodes[element.index] {
            element = inner;
            read_only |= element.qualifiers.contains(TypeQualifiers::CONST);
        }
        if read_only {
            self.clear_padding_error(source, "read-only variable is not assignable");
            return None;
        }
        let kind = self.types.nodes[pointee.index];
        let array = matches!(kind, TypeKind::Array(..));
        let function = matches!(kind, TypeKind::Function { .. });
        if !function && !self.types.complete_object(pointee) {
            let text = match self.type_spelling(pointee, false) {
                | Some(pointee) =>
                    format_in!(self.scratch, "variable has incomplete type '{pointee}'"),
                | None => "variable has incomplete type",
            };
            self.clear_padding_error(source, text);
            return None;
        }
        if function || (array && matches!(self.types.nodes[element.index], TypeKind::Atomic(_))) {
            let text = match self.type_spelling(ty, false) {
                | Some(ty) => format_in!(
                    self.scratch,
                    "argument to {NAME} must be a pointer to a trivially-copyable type ('{ty}' \
                     invalid)"
                ),
                | None => format_in!(
                    self.scratch,
                    "argument to {NAME} must be a pointer to a trivially-copyable type"
                ),
            };
            self.clear_padding_error(source, text);
            return None;
        }
        let mut outer = pointee;
        while let TypeKind::Array(inner, bound) = self.types.nodes[outer.index] {
            if matches!(bound, ArrayBound::Variable(_)) {
                return Some(self.types.scalar(Scalar::Void));
            }
            outer = inner;
        }
        if self.undefined_padding(pointee, false).is_some() {
            let text = match self.type_spelling(pointee, false) {
                | Some(pointee) => format_in!(
                    self.scratch,
                    "'{pointee}' has flexible array member, which is unsupported by {NAME}"
                ),
                | None => format_in!(
                    self.scratch,
                    "a flexible array member is unsupported by {NAME}"
                ),
            };
            self.clear_padding_error(source, text);
            return None;
        }
        Some(pointee)
    }

    /// The first member, in declaration order, whose padding is undefined
    /// in an object of type `ty`, seen through nested records and, under
    /// GCC's rules (`through_arrays`), arrays.
    fn undefined_padding(&self, ty: TypeId, through_arrays: bool) -> Option<Undefined> {
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push(ty);
        while let Some(ty) = pending.pop() {
            match self.types.nodes[ty.index] {
                | TypeKind::Array(element, _) if through_arrays => pending.push(element),
                | TypeKind::Atomic(value) if through_arrays => pending.push(value),
                | TypeKind::Tag(id) => {
                    let tag = self.types.tags[id];
                    if tag.kind == TagKind::Enum {
                        continue;
                    }
                    if tag.variable.get().is_some() {
                        return Some(Undefined::VariableSize);
                    }
                    for member in tag.members.get().iter().rev() {
                        if let TypeKind::Array(_, ArrayBound::Incomplete) =
                            self.types.nodes[member.ty.index]
                        {
                            return Some(Undefined::Flexible(member.name.map(|name| name.name)));
                        }
                        pending.push(member.ty);
                    }
                },
                | _ => {},
            }
        }
        None
    }

    fn clear_padding_error(&mut self, source: SourceVectors, text: &str) {
        let text = self.context.string_cache.intern(text);
        self.error(
            SemanticErrorKind::ClearPaddingOperand,
            source,
            Some(text),
            None,
        );
    }
}
