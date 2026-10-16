//! Data-driven Clang x86 builtin signatures and immediate constraints.
//! Extensions: GCC x86 Built-in Functions and Clang Language Extensions.
//! Table generation independently compiles every signature on four targets.
//!
//! These extensions are checked during translation phase 7. Operand types and
//! constant requirements are validated here; backend instructions are not
//! emitted. C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22;
//! §6.5.2.2, pp. 71-72; PDF pp. 83-84 (extended function calls).

use super::x86_builtin_table::BUILTINS;
use crate::{
    translation_phases::{
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::Expression,
        },
        semantic_analysis::{
            Analyzer,
            SemanticErrorKind,
            expressions::ExpressionInfo,
            types::{
                Scalar,
                TypeId,
                TypeKind,
            },
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Builds the declared result and parameter types of a recognized x86
    /// intrinsic.
    /// GNU extension: GCC manual, "x86 Built-in Functions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/x86-Built-in-Functions.html>
    pub(in crate::translation_phases::semantic_analysis) fn x86_builtin_type(
        &mut self,
        name: super::Identifier,
    ) -> Option<TypeId> {
        let i = BUILTINS
            .binary_search_by(|entry| entry.0.cmp(self.context.string_cache.at(name.name)))
            .ok()?;
        self.context.report_extension(
            crate::configuration::Feature::VectorBuiltins,
            "x86 builtin",
            name.source_vectors,
        );
        let mut codes = BUILTINS[i].1.split(',');
        let result = self.x86_builtin_operand(codes.next()?)?;
        let mut parameters = ArenaVec::new_in(self.types.tu);
        for code in codes {
            parameters.push(self.x86_builtin_operand(code)?);
        }
        Some(self.types.intern(TypeKind::Function {
            result,
            parameters: parameters.leak(),
            prototype: true,
            variadic: false,
        }))
    }

    fn x86_builtin_operand(&mut self, mut code: &str) -> Option<TypeId> {
        let pointer = code.starts_with('P');
        if pointer {
            code = &code[1..];
        }
        let constant = code.starts_with('C');
        if constant {
            code = &code[1..];
        }
        let scalar = match code.as_bytes().last()? {
            | b'v' => Scalar::Void,
            | b'c' => Scalar::Char,
            | b'a' => Scalar::SignedChar,
            | b'A' => Scalar::UnsignedChar,
            | b's' => Scalar::Short,
            | b'S' => Scalar::UnsignedShort,
            | b'i' => Scalar::Int,
            | b'I' => Scalar::UnsignedInt,
            | b'l' => Scalar::Long,
            | b'L' => Scalar::UnsignedLong,
            | b'q' => Scalar::LongLong,
            | b'Q' => Scalar::UnsignedLongLong,
            | b'f' => Scalar::Float,
            | b'd' => Scalar::Double,
            | _ => return None,
        };
        let mut ty = self.types.scalar(scalar);
        if code.starts_with('V') {
            let count: u64 = code[1..code.len() - 1].parse().ok()?;
            let align = self.types.layout(ty)?.size.checked_mul(count)?;
            ty = self.types.intern(TypeKind::Vector {
                element: ty,
                count,
                align,
            });
        }
        if constant {
            ty = ty.qualified(TypeQualifiers::CONST);
        }
        if pointer {
            ty = self.types.intern(TypeKind::Pointer(ty));
        }
        Some(ty)
    }

    /// Checks constant immediate arguments after prototype argument conversion.
    /// GNU extension: GCC manual, "x86 Built-in Functions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/x86-Built-in-Functions.html>
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84.
    pub(in crate::translation_phases::semantic_analysis) fn x86_immediates(
        &mut self,
        function: ExpressionInfo<'tu>,
        args: super::ArenaList<'tu, &'tu Expression<'tu>>,
    ) {
        let Some(binding) = function.binding else {
            return;
        };
        let name = self
            .context
            .string_cache
            .at(self.bindings[binding].name.name);
        let Ok(i) = BUILTINS.binary_search_by(|entry| entry.0.cmp(name)) else {
            return;
        };
        // C99 §6.5.2.2p7, p. 72; PDF p. 84: prototype arguments convert
        // to parameter types before GNU intrinsic immediate constraints apply.
        let TypeKind::Function { parameters, .. } = self.types.nodes[function.ty.index] else {
            return;
        };
        for &(index, low, high, mask) in BUILTINS[i].2 {
            if let Some(&arg) = args.get(index) {
                let info = self.expression_info(arg);
                if self.types.unanalyzed(info.ty) {
                    continue;
                }
                let value = info.integer.filter(|_| info.ice).and_then(|value| {
                    let parameter = parameters.get(index)?;
                    let (bits, signed) = self.integer_type(*parameter)?;
                    value.cast(bits, signed).to_i128()
                });
                if !value.is_some_and(|v| {
                    v >= low && v <= high && (mask == 0 || (v < 16 && (mask & (1 << v)) != 0))
                }) {
                    self.error(
                        SemanticErrorKind::InvalidImmediate,
                        arg.source_vectors,
                        None,
                        None,
                    );
                }
            }
        }
    }

    /// Clang overloads these builtins by their operands; they cannot be given
    /// a single C prototype. Retain the checked operand-dependent result.
    /// Clang extension: Clang Language Extensions, "Builtin Functions".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-functions>
    pub(in crate::translation_phases::semantic_analysis) fn vector_overload(
        &mut self,
        e: &'tu Expression<'tu>,
        function: &'tu Expression<'tu>,
        args: super::ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> Option<ExpressionInfo<'tu>> {
        let super::ExpressionType::Identifier(id) = function.kind else {
            return None;
        };
        let name = self.context.string_cache.at(id.name);
        if name == "__builtin_prefetch" {
            self.context.report_extension(
                crate::configuration::Feature::VectorBuiltins,
                "prefetch builtin",
                id.source_vectors,
            );
            return Some(self.prefetch_builtin(e, args));
        }
        let (arity, integer, floating) = match name {
            | "__builtin_elementwise_abs"
            | "__builtin_nondeterministic_value"
            | "__builtin_nontemporal_load" => (1, false, false),
            | "__builtin_elementwise_sqrt" => (1, false, true),
            | "__builtin_elementwise_add_sat" | "__builtin_elementwise_sub_sat" => (2, true, false),
            | "__builtin_elementwise_min"
            | "__builtin_elementwise_max"
            | "__builtin_nontemporal_store" => (2, false, false),
            | _ => return None,
        };
        let load = name == "__builtin_nontemporal_load";
        let store = name == "__builtin_nontemporal_store";
        let absolute = name == "__builtin_elementwise_abs";
        let min_max = matches!(
            name,
            "__builtin_elementwise_min" | "__builtin_elementwise_max"
        );
        self.context.report_extension(
            crate::configuration::Feature::VectorBuiltins,
            "vector builtin",
            id.source_vectors,
        );
        // GNU overload constraints cannot add errors for already failed
        // operands.
        if args
            .iter()
            .any(|&arg| self.types.unanalyzed(self.expression_info(arg).ty))
        {
            return Some(Self::expression_result(e, self.types.unknown()));
        }
        let input = args.first().map(|&arg| self.expression_info(arg));
        if args.len() == arity
            && let Some(input) = input
        {
            let mut ty = self.converted(input);
            if load {
                if let TypeKind::Pointer(element) = self.types.nodes[ty.index]
                    && self.nontemporal_element(element)
                {
                    return Some(Self::expression_result(e, element.unqualified()));
                }
            } else if store {
                let pointer = self.expression_info(args[1]);
                let pointer = self.converted(pointer);
                if let TypeKind::Pointer(to) = self.types.nodes[pointer.index]
                    && self.nontemporal_element(to)
                    && !to.qualifiers.contains(TypeQualifiers::CONST)
                    && self.assignment_compatible(to, input)
                {
                    ty = self.types.scalar(Scalar::Void);
                    return Some(Self::expression_result(e, ty));
                }
            } else {
                let element = self.vector(ty).map_or(ty, |v| v.0);
                let valid = (self.integer_type(element).is_some()
                    || matches!(
                        self.types.nodes[element.index],
                        TypeKind::Scalar(Scalar::Float | Scalar::Double | Scalar::LongDouble)
                    ))
                    // Clang's GNU min/max overloads exclude C99 §6.2.5p2 _Bool.
                    && (!min_max || !matches!(self.types.nodes[element.index], TypeKind::Scalar(Scalar::Bool)))
                    && (!absolute || self.integer_type(element).is_none_or(|(_, signed)| signed))
                    && (!integer || self.integer_type(element).is_some())
                    && (!floating
                        || matches!(
                            self.types.nodes[element.index],
                            TypeKind::Scalar(Scalar::Float | Scalar::Double | Scalar::LongDouble)
                        ))
                    && args
                        .iter()
                        .all(|&arg| self.expression_info(arg).ty.unqualified() == ty.unqualified());
                if valid {
                    return Some(Self::expression_result(e, ty.unqualified()));
                }
            }
        }
        self.error(
            SemanticErrorKind::InvalidVectorBuiltin,
            e.source_vectors,
            None,
            None,
        );
        Some(Self::expression_result(e, self.types.unknown()))
    }

    /// Checks scalar element eligibility for non-temporal load and store
    /// builtins.
    /// Clang extension: Clang Language Extensions, "Non-temporal load/store
    /// builtins".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#non-temporal-load-store-builtins>
    fn nontemporal_element(&self, ty: TypeId) -> bool {
        let element = self.vector(ty).map_or(ty, |v| v.0);
        self.integer_type(element).is_some()
            || matches!(
                self.types.nodes[element.index],
                TypeKind::Pointer(_)
                    | TypeKind::Scalar(Scalar::Float | Scalar::Double | Scalar::LongDouble)
            )
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// GCC Other Builtins: prefetch accepts a pointer and two optional ICEs.
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    fn prefetch_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        args: super::ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        if args.is_empty() || args.len() > 3 {
            self.error(
                SemanticErrorKind::InvalidArgumentCount,
                e.source_vectors,
                None,
                None,
            );
            return Self::expression_result(e, self.types.unknown());
        }
        let address = self.expression_info(args[0]);
        let ty = self.converted(address);
        if !self.types.unanalyzed(ty) && !matches!(self.types.nodes[ty.index], TypeKind::Pointer(_))
        {
            self.error(
                SemanticErrorKind::InvalidArgumentType,
                args[0].source_vectors,
                None,
                None,
            );
        }
        for (i, &argument) in args.iter().enumerate().skip(1) {
            let info = self.expression_info(argument);
            if self.types.unanalyzed(info.ty) {
                continue;
            }
            let value = info
                .integer
                .filter(|_| info.ice)
                .and_then(super::Integer::to_i128);
            let high = if i == 1 { 1 } else { 3 };
            if !value.is_some_and(|v| v >= 0 && v <= high) {
                self.error(
                    SemanticErrorKind::InvalidImmediate,
                    argument.source_vectors,
                    None,
                    None,
                );
            }
        }
        let ty = self.types.scalar(Scalar::Void);
        Self::expression_result(e, ty)
    }
}

/// Used by semantic lookup and __`has_builtin`; no implicit unknown signatures.
pub(crate) fn known(name: &str) -> bool {
    BUILTINS.binary_search_by(|entry| entry.0.cmp(name)).is_ok()
}
