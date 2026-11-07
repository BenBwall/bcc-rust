//! Phase-7 diagnostics that only the compiler a personality stands in for
//! gives, under `--compiler-extensions`: what it refuses of syntax bcc accepts,
//! and what it warns of a declaration or a constant bcc itself says nothing of.
//! Each was measured on GCC 14.2 and Clang 23.1. C99 leaves all of them to the
//! implementation, as extensions (§4p6) or as diagnostics it need not give.
//! C99: §4 paragraph 6, p. 7; PDF p. 19; §5.1.1.3 paragraph 1, p. 11; PDF p.
//! 23.

use super::{
    Analyzer,
    SemanticErrorKind,
    expressions::ExpressionInfo,
};
use crate::{
    configuration::{
        CStandard,
        DiagnosticGroup,
        FeatureOrigin,
        RuleSet,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Declaration,
                TypeSpecifiers,
            },
            syntax::{
                BinaryOperator,
                Expression,
                ExpressionType,
                Identifier,
                StorageClass,
            },
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Reports `text`, in the compiler's words, as the error it gives.
    pub(super) fn compiler_refusal(&mut self, text: &str, source: SourceVectors) {
        let text = self.context.string_cache.intern(text);
        self.error(SemanticErrorKind::CompilerRefusal(text), source, None, None);
    }

    fn gcc_personality(&self) -> bool {
        self.context.configuration.compiler_extensions()
            && self.context.configuration.compiler_identity().rules() == RuleSet::Gcc
    }

    fn clang_personality(&self) -> bool {
        self.context.configuration.compiler_extensions()
            && self.context.configuration.compiler_identity().rules() == RuleSet::Clang
    }

    /// GCC refuses a `for` loop's declaration before C99, where bcc and
    /// Clang take it as an extension.
    /// C99: §6.8.5 paragraph 3, p. 135; PDF p. 147.
    pub(super) fn report_for_declaration(&mut self, source: SourceVectors) {
        if self.gcc_personality()
            && self.context.configuration.standard() < CStandard::C99
            && !self.tainted
        {
            self.compiler_refusal(
                "'for' loop initial declarations are only allowed in C99 or C11 mode",
                source,
            );
        }
    }

    /// What GCC's `-pedantic` says of `__FUNCTION__` and
    /// `__PRETTY_FUNCTION__`; Clang says nothing.
    /// GNU extension: GCC manual, "Function Names as Strings".
    pub(super) fn report_predefined_identifier(&mut self, name: Identifier) {
        if !self.gcc_personality() {
            return;
        }
        let text = match self.context.string_cache.at(name.name) {
            | "__FUNCTION__" => "ISO C does not support '__FUNCTION__' predefined identifier",
            | "__PRETTY_FUNCTION__" =>
                "ISO C does not support '__PRETTY_FUNCTION__' predefined identifier",
            | _ => return,
        };
        self.report_pedantic_constant(text, name.source_vectors);
    }

    /// What GCC's `-pedantic` says of `__func__` before C99, which a function
    /// declares ahead of any use, so that this reads the identifier.
    /// C99: §6.4.2.2 paragraph 1, p. 52; PDF p. 64.
    pub(super) fn report_c90_func(&mut self, name: Identifier) {
        if self.gcc_personality()
            && self.context.configuration.standard() < CStandard::C99
            && self.context.string_cache.at(name.name) == "__func__"
        {
            self.report_pedantic_constant(
                "ISO C90 does not support '__func__' predefined identifier",
                name.source_vectors,
            );
        }
    }

    /// `-Wmultichar`, on by default in both. Clang leaves a four-character
    /// constant to `-Wfour-char-constants`, which is off.
    /// C99: §6.4.4.4 paragraph 10, p. 62; PDF p. 74.
    pub(super) fn report_multicharacter_constant(&mut self, e: &'tu Expression<'tu>, packed: i32) {
        if !self.context.configuration.compiler_extensions() {
            return;
        }
        if self.clang_personality() && packed.cast_unsigned() > 0x00FF_FFFF {
            return;
        }
        let text = self
            .context
            .string_cache
            .intern("multi-character character constant");
        self.group_diagnostic(
            DiagnosticGroup::Multichar,
            SemanticErrorKind::ExceptionalConstant(DiagnosticGroup::Multichar, text),
            e.source_vectors,
            None,
            None,
        );
    }

    /// A variable length array before C99, which both report under
    /// `-pedantic`.
    /// C99: §6.7.5.2 paragraph 4, p. 116; PDF p. 128.
    pub(super) fn report_variable_length_array(&mut self, expression: &'tu Expression<'tu>) {
        if self.context.configuration.compiler_extensions() && !self.tainted {
            self.context.report_extension_since(
                "variable length array",
                FeatureOrigin::Standard(CStandard::C99),
                expression.source_vectors,
            );
        }
    }

    /// A declaration of no declarators, tag or enumerators, which both warn
    /// of by default: `int;`, `struct { int x; };`.
    /// C99: §6.7 paragraph 2, p. 97; PDF p. 109.
    pub(super) fn report_empty_declaration(&mut self, d: &'tu Declaration<'tu>) {
        if !self.context.configuration.compiler_extensions()
            || self.old_parameter_mode
            || !d.init_declarators.is_empty()
            || d.declaration_specifiers.storage_class == Some(StorageClass::Typedef)
        {
            return;
        }
        let gcc = self.gcc_personality();
        let text = match d.declaration_specifiers.type_specifiers {
            | TypeSpecifiers::Empty | TypeSpecifiers::Enum(_) => return,
            | TypeSpecifiers::StructOrUnion(record) if record.identifier.is_some() => return,
            | TypeSpecifiers::StructOrUnion(_) if gcc =>
                "unnamed struct/union that defines no instances",
            | _ if gcc => "useless type name in empty declaration",
            | _ => "declaration does not declare anything",
        };
        let group = if gcc {
            DiagnosticGroup::GccEmptyDeclaration
        } else {
            DiagnosticGroup::MissingDeclarations
        };
        let text = self.context.string_cache.intern(text);
        self.group_diagnostic(
            group,
            SemanticErrorKind::ExceptionalConstant(group, text),
            d.source_vectors,
            None,
            None,
        );
    }

    /// Whether `e` evaluates a comma operator, which no constant expression
    /// may (C99 §6.6p3), through what a constant `?:` selects.
    fn evaluates_comma(&self, e: &'tu Expression<'tu>) -> bool {
        use ExpressionType as E;
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(e);
        while let Some(e) = work.pop() {
            match e.kind {
                | E::Parenthesized { expression } => work.push(expression),
                | E::Unary {
                    operand_expression, ..
                }
                | E::Cast {
                    operand_expression, ..
                } => work.push(operand_expression),
                | E::Binary {
                    operator: BinaryOperator::Comma,
                    ..
                } => return true,
                | E::Binary {
                    left_expression,
                    right_expression,
                    ..
                } => {
                    work.push(right_expression);
                    work.push(left_expression);
                },
                | E::Conditional(c) | E::OmittedConditional(c) => {
                    work.push(c.condition_expression);
                    match Self::constant_truth(self.expression_info(c.condition_expression)) {
                        | Some(true) => work.push(c.then_expression),
                        | Some(false) => work.push(c.else_expression),
                        | None => {
                            work.push(c.then_expression);
                            work.push(c.else_expression);
                        },
                    }
                },
                | _ => {},
            }
        }
        false
    }

    /// What a constant context reads from `e` if it evaluates a comma: GCC
    /// reads nothing, Clang folds it in every mode. Whether it pushed a value.
    /// C99: §6.6 paragraph 3, p. 95; PDF p. 107.
    pub(super) fn evaluate_comma(
        &mut self,
        e: &'tu Expression<'tu>,
        info: ExpressionInfo<'tu>,
    ) -> bool {
        if !self.context.configuration.compiler_extensions() || !self.evaluates_comma(e) {
            return false;
        }
        if self.gcc_personality() {
            self.integers.push(None);
            return true;
        }
        if !self.runtime_bound && !self.tainted && info.integer.is_some() {
            self.report_folded_constant(e);
        }
        self.integers.push(info.integer);
        true
    }

    /// Whether Clang folds the array bound `e` for its comma, as it does a
    /// bound with an address in it.
    pub(super) fn clang_folds_comma(&self, e: &'tu Expression<'tu>) -> bool {
        self.clang_personality() && self.evaluates_comma(e)
    }

    /// GCC takes a static initializer that is a comma expression itself,
    /// with a report under `-pedantic`, and from C99 on refuses one that has
    /// it deeper. Whether it refuses `e`.
    /// C99: §6.6 paragraph 3, p. 95; PDF p. 107.
    pub(super) fn gcc_refuses_comma_initializer(&mut self, e: &'tu Expression<'tu>) -> bool {
        if !self.gcc_personality() || !self.evaluates_comma(e) {
            return false;
        }
        let mut root = e;
        while let ExpressionType::Parenthesized { expression } = root.kind {
            root = expression;
        }
        let whole = matches!(
            root.kind,
            ExpressionType::Binary {
                operator: BinaryOperator::Comma,
                ..
            }
        );
        if !whole && self.context.configuration.standard() >= CStandard::C99 {
            return true;
        }
        self.report_pedantic_constant("initializer element is not constant", e.source_vectors);
        false
    }
}
