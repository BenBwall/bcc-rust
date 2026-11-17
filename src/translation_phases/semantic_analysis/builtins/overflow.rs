//! The arguments of the type-generic overflow builtins,
//! `__builtin_add_overflow`, `__builtin_sub_overflow` and
//! `__builtin_mul_overflow`, which C23's `<stdckdint.h>` macros call. Both
//! GCC and Clang check the two operands and then the result pointer, stop
//! at the first they refuse, and word it each its own way. Clang also holds
//! a call that `ckd_add`, `ckd_sub` or `ckd_mul` wrote to the narrower
//! types C23 gives those macros; GCC does not.
//! GNU extension: GCC "Integer Overflow Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
//! C23: §7.20.1p3, p. 313; PDF p. 326.

use super::Identifier;
use crate::{
    builtins::OverflowOperation,
    configuration::{
        DiagnosticFormat,
        RuleSet,
    },
    diagnostics::format_in,
    translation_phases::{
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::Expression,
        },
        semantic_analysis::{
            Analyzer,
            Scalar,
            SemanticErrorKind,
            TypeId,
            TypeKind,
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the arguments `args` of a call of the type-generic overflow
    /// builtin `name`, whose designator is `callee`, and gives the type the
    /// result points to, or reports the first argument the compiler refuses
    /// and gives `None`. An argument whose type analysis gave up on is left
    /// alone.
    pub(super) fn overflow_arguments(
        &mut self,
        name: Identifier,
        callee: &'tu Expression<'tu>,
        operation: OverflowOperation,
        args: &[&'tu Expression<'tu>],
    ) -> Option<TypeId> {
        let checked = self.checked_arithmetic_macro(callee, operation);
        for (position, &arg) in args[..2].iter().enumerate() {
            let ty = self.converted(self.expression_info(arg));
            if self.types.unanalyzed(ty) {
                continue;
            }
            let valid = if checked {
                self.checked_integer(ty)
            } else {
                self.integer_type(ty).is_some()
            };
            if !valid {
                self.overflow_argument_error(name, position, arg, ty, checked, None);
                return None;
            }
        }
        let arg = args[2];
        let ty = self.converted(self.expression_info(arg));
        if self.types.unanalyzed(ty) {
            return None;
        }
        let TypeKind::Pointer(target) = self.types.nodes[ty.index] else {
            self.overflow_argument_error(name, 2, arg, ty, checked, Some(GccRefusal::Integral));
            return None;
        };
        let value = self.types.non_atomic(target);
        let atomic = value.index != target.index;
        // GCC's order: an integer at all, then an enumeration, a boolean,
        // `const` and `_Atomic`.
        let refusal = if self.integer_type(value).is_none() {
            Some(GccRefusal::Integral)
        } else if self.enumerated(value) {
            Some(GccRefusal::Enumerated)
        } else if self.types.nodes[value.index] == TypeKind::Scalar(Scalar::Bool) {
            Some(GccRefusal::Boolean)
        } else if target.qualifiers.contains(TypeQualifiers::CONST) {
            Some(GccRefusal::Qualified("const"))
        } else if atomic {
            Some(GccRefusal::Qualified("_Atomic"))
        } else if checked && !self.checked_integer(value) {
            // Only Clang's rules refuse more, and Clang's words do not say
            // why.
            Some(GccRefusal::Integral)
        } else {
            None
        };
        if refusal.is_some() {
            self.overflow_argument_error(name, 2, arg, ty, checked, refusal);
            return None;
        }
        Some(target)
    }

    /// Whether the call's designator `callee` is spelled in the replacement
    /// list of the `<stdckdint.h>` macro for `operation`, which Clang's
    /// rules hold to C23's types (`Sema::BuiltinOverflow` asks for the
    /// immediate macro's name, whoever defined it).
    /// C23: §7.20.1p3, p. 313; PDF p. 326.
    fn checked_arithmetic_macro(
        &self,
        callee: &'tu Expression<'tu>,
        operation: OverflowOperation,
    ) -> bool {
        let macro_name = match operation {
            | OverflowOperation::Add => "ckd_add",
            | OverflowOperation::Sub => "ckd_sub",
            | OverflowOperation::Mul => "ckd_mul",
        };
        self.context.configuration.compiler_identity().rules() == RuleSet::Clang
            && self.context.immediate_macro_name(callee.source_vectors) == Some(macro_name)
    }

    /// Whether `ty` is an integer type C23's checked arithmetic takes: any
    /// but plain `char`, `bool`, a bit-precise type or an enumeration.
    /// C23: §7.20.1p3, p. 313; PDF p. 326.
    fn checked_integer(&self, ty: TypeId) -> bool {
        matches!(
            self.types.nodes[ty.index],
            TypeKind::Scalar(scalar)
                if self.types.target.integer(scalar).is_some()
                    && !matches!(scalar, Scalar::Bool | Scalar::Char)
        )
    }

    /// Reports argument `position` (0-based) `arg` of type `ty`, in the
    /// words of the compiler whose format is in use: GCC's names the
    /// builtin and, for the result, why it refuses the type it points to
    /// (`refusal`); Clang's names the type and whether C23's checked
    /// arithmetic (`checked`) asked.
    fn overflow_argument_error(
        &mut self,
        name: Identifier,
        position: usize,
        arg: &'tu Expression<'tu>,
        ty: TypeId,
        checked: bool,
        refusal: Option<GccRefusal>,
    ) {
        let kind = if position < 2 {
            SemanticErrorKind::OverflowBuiltinOperand
        } else {
            SemanticErrorKind::OverflowBuiltinResult
        };
        let words = match self.context.configuration.diagnostic_format() {
            | DiagnosticFormat::Gcc => {
                let function = self.context.string_cache.at(name.name);
                let argument = position + 1;
                let reason = match refusal {
                    | None => "does not have integral type",
                    | Some(GccRefusal::Integral) => "does not have pointer to integral type",
                    | Some(GccRefusal::Enumerated) => "has pointer to enumerated type",
                    | Some(GccRefusal::Boolean) => "has pointer to boolean type",
                    | Some(GccRefusal::Qualified(qualifier)) => {
                        let spelled = self.type_spelling(ty, true).unwrap_or_default();
                        format_in!(
                            self.scratch,
                            "has pointer to '{qualifier}' type ('{spelled}')"
                        )
                    },
                };
                Some(format_in!(
                    self.scratch,
                    "argument {argument} in call to function '{function}' {reason}"
                ))
            },
            | DiagnosticFormat::Clang => {
                let spelled = self.type_spelling(ty, false).unwrap_or_default();
                let (operation, other) = if checked {
                    (
                        "checked integer operation",
                        " other than plain 'char', 'bool', bit-precise, or an enumeration",
                    )
                } else {
                    ("overflow builtin", "")
                };
                Some(if position < 2 {
                    format_in!(
                        self.scratch,
                        "operand argument to {operation} must be an integer type{other} \
                         ('{spelled}' invalid)"
                    )
                } else {
                    format_in!(
                        self.scratch,
                        "result argument to {operation} must be a pointer to a non-const integer \
                         type{other} ('{spelled}' invalid)"
                    )
                })
            },
            | _ => None,
        };
        let words = words.map(|words| self.context.string_cache.intern(words));
        self.error(kind, arg.source_vectors, words, None);
    }
}

/// Why GCC refuses an overflow builtin's result pointer
/// (`check_builtin_function_arguments`).
#[derive(Clone, Copy)]
enum GccRefusal {
    /// It does not point to an integer.
    Integral,
    /// It points to an enumeration.
    Enumerated,
    /// It points to `bool`.
    Boolean,
    /// It points to a `const` or `_Atomic` integer.
    Qualified(&'static str),
}
