//! GCC's and Clang's warnings about what a `switch` handles: `-Wswitch` (an
//! enumerator no `case` names, or a `case` its enumeration lacks),
//! `-Wswitch-enum` (the same with a `default`), `-Wswitch-default`, and a
//! `case` its controlling type cannot hold (GCC's `-Wswitch-outside-range`,
//! Clang's `-Wswitch`), and Clang's constant condition that no `case`
//! matches. §6.8.4.2p5 converts each `case` to the promoted controlling type,
//! so each is a warning §5.1.1.3p1 permits.
//! GCC: `c_do_switch_warnings` in `c-warn.cc`, `c_add_case_label`.
//! Clang: `Sema::ActOnFinishSwitchStmt`.
//! C99: §6.8.4.2p5, p. 134; PDF p. 146; §6.7.2.2p3, p. 105; PDF p. 117.

use super::{
    super::{
        Analyzer,
        SemanticErrorKind,
        types::{
            TagKind,
            TypeId,
            TypeKind,
        },
    },
    Case,
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticGroup as G,
    },
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator as B,
            ConstantExpressionSlot,
            Expression,
            ExpressionType as E,
            StatementType as S,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the finished `switch` at `index`, whose labels are `cases`,
    /// unless a `case` is `erroneous`.
    /// C99: §6.8.4.2p5, p. 134; PDF p. 146.
    pub(super) fn check_switch_warnings(
        &mut self,
        index: usize,
        cases: &[Case<'tu>],
        erroneous: bool,
    ) {
        if !self.unused.switches || self.tainted {
            return;
        }
        let switch = self.statements.switches[index];
        let S::Switch {
            condition_expression,
            ..
        } = switch.statement.kind
        else {
            return;
        };
        let Some(condition) = super::slot_expression(condition_expression) else {
            return;
        };
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        let keyword = self.first_vector(switch.statement.source_vectors);
        if switch.default.is_none() {
            let text = if gcc {
                "switch missing default case"
            } else {
                "'switch' missing 'default' label"
            };
            self.switch_warning(G::SwitchDefault, keyword, text);
        }
        let ty = self
            .types
            .non_atomic(self.expression_info(condition).ty)
            .unqualified();
        let Some((bits, signed)) = self
            .integer_type(switch.ty)
            .filter(|&(bits, _)| bits <= 128)
        else {
            return;
        };
        // GCC asks whether the condition is an integer constant expression;
        // Clang folds it, side effects aside, only with no `default`.
        let constant = if gcc {
            let info = self.expression_info(condition);
            info.ice.then_some(info.integer).flatten()
        } else if switch.default.is_none() {
            match self.evaluate_ignoring_side_effects(condition) {
                | Some(super::super::emission::Value::Integer(value)) => Some(value),
                | _ => None,
            }
        } else {
            None
        }
        .and_then(|value| value.cast(bits, signed).to_i128());
        let enumeration = if gcc {
            self.gcc_enumeration(condition)
        } else {
            self.enumeration_of(ty)
        };
        // Clang checks each `case` against the condition's type before
        // promotion (`checkCaseValue`). GCC does where a promotion widens
        // that type, and its conversion warning covers the rest.
        if !gcc
            || (enumeration.is_none() && self.integer_type(ty).map(|(bits, _)| bits) < Some(bits))
        {
            self.check_case_range(ty, cases, gcc);
        }
        if !gcc && erroneous {
            return;
        }
        if !gcc && let Some(value) = constant {
            self.check_constant_condition(condition, ty != switch.ty, value, cases);
        }
        let Some(tag) = enumeration else {
            return;
        };
        // Clang checks no enumeration against a constant condition.
        if !gcc && constant.is_some() {
            return;
        }
        let Some(enumerators) = self.reachability.enumerators.get(&tag) else {
            return;
        };
        // Each enumerator's value converted to the promoted type, as each
        // `case` is.
        let mut values = ArenaVec::new_in(self.scratch);
        values.extend(
            enumerators
                .iter()
                .map(|&(value, name, unused)| (converted(value, bits, signed), name, unused)),
        );
        // bcc keeps no typedef names, so an enumeration with no tag cannot be
        // named as both compilers name it, through its typedef.
        let enum_type = self.types.intern(TypeKind::Tag(tag));
        let type_name = self.type_spelling(enum_type, gcc);
        // A `case` its enumeration lacks, compared once converted.
        for case in cases.iter().filter(|case| !case.range) {
            let Some(expression) = Self::case_expression(case) else {
                continue;
            };
            let Some(value) = case.lower.to_i128() else {
                continue;
            };
            if values.iter().any(|&(enumerator, ..)| enumerator == value) {
                continue;
            }
            let Some(type_name) = type_name else {
                continue;
            };
            let (site, text) = if gcc {
                (
                    self.first_vector(case.source),
                    format_in!(
                        self.scratch,
                        "case value '{value}' not in enumerated type '{type_name}'"
                    ),
                )
            } else {
                (
                    expression.source_vectors,
                    format_in!(
                        self.scratch,
                        "case value not in enumerated type '{type_name}'"
                    ),
                )
            };
            // GCC's `match_case_to_enum_1` names `-Wswitch-enum` when
            // `-Wswitch` is off.
            let group = if gcc && self.context.group_severity(G::Switch, keyword).is_none() {
                G::SwitchEnum
            } else {
                G::Switch
            };
            self.switch_warning(group, site, text);
        }
        // The enumerators no `case` names, once for each value.
        let covered = |value: i128| {
            cases.iter().any(|case| {
                case.lower.to_i128().is_some_and(|lower| lower <= value)
                    && case.upper.to_i128().is_some_and(|upper| value <= upper)
            })
        };
        let group = if switch.default.is_some() {
            G::SwitchEnum
        } else if self.context.group_severity(G::Switch, keyword).is_some() {
            G::Switch
        } else {
            G::SwitchEnum
        };
        // Both compilers leave out an enumerator marked unused.
        values.retain(|&(.., unused)| !unused);
        if gcc {
            for &(value, name, _) in &values {
                // GCC asks only about the enumerator a constant names.
                if !covered(value) && constant.is_none_or(|constant| constant == value) {
                    let name = self.context.string_cache.at(name);
                    let text = format_in!(
                        self.scratch,
                        "enumeration value '{name}' not handled in switch"
                    );
                    self.switch_warning(group, keyword, text);
                }
            }
            return;
        }
        values.sort_by_key(|&(value, ..)| value);
        values.dedup_by_key(|&mut (value, ..)| value);
        // Copied out, so that reporting does not hold the string cache.
        let scratch = self.scratch;
        let mut missing: ArenaVec<'_, &str> = ArenaVec::new_in(scratch);
        missing.extend(
            values
                .iter()
                .filter(|&&(value, ..)| !covered(value))
                .map(|&(_, name, _)| &*scratch.alloc_str(self.context.string_cache.at(name))),
        );
        let handled = if group == G::SwitchEnum {
            "explicitly handled"
        } else {
            "handled"
        };
        let text = match missing.as_slice() {
            | [] => return,
            | [a] => format_in!(scratch, "enumeration value '{a}' not {handled} in switch"),
            | [a, b] => format_in!(
                scratch,
                "enumeration values '{a}' and '{b}' not {handled} in switch"
            ),
            | [a, b, c] => format_in!(
                scratch,
                "enumeration values '{a}', '{b}', and '{c}' not {handled} in switch"
            ),
            | [a, b, c, ..] => format_in!(
                scratch,
                "{} enumeration values not {handled} in switch: '{a}', '{b}', '{c}'...",
                missing.len()
            ),
        };
        self.switch_warning(group, condition.source_vectors, text);
    }

    /// Clang's constant condition that no `case` matches, at the condition's
    /// operator unless a promotion converts it.
    fn check_constant_condition(
        &mut self,
        condition: &'tu Expression<'tu>,
        promoted: bool,
        value: i128,
        cases: &[Case<'tu>],
    ) {
        let matched = cases.iter().any(|case| {
            case.lower.to_i128().is_none_or(|lower| lower <= value)
                && case.upper.to_i128().is_none_or(|upper| value <= upper)
        });
        if matched {
            return;
        }
        let site = if promoted {
            condition.source_vectors
        } else {
            condition
                .operator_source_vectors
                .unwrap_or(condition.source_vectors)
        };
        let text = format_in!(
            self.scratch,
            "no case matching constant switch condition '{value}'"
        );
        self.switch_warning(G::ConstantSwitchCondition, site, text);
    }

    /// The enumeration GCC's front end remembers for a condition: that of
    /// its type, or for a conditional expression, whose result the usual
    /// arithmetic conversions make an integer, the one both arms remember,
    /// an enumeration constant remembering its own: the `original_type` of
    /// `c_parser_conditional_expression`.
    /// C99: §6.5.15p5, p. 90; PDF p. 102; §6.7.2.2p3, p. 105; PDF p. 117.
    fn gcc_enumeration(&self, e: &'tu Expression<'tu>) -> Option<usize> {
        let e = Self::without_parentheses(e);
        match e.kind {
            | E::Conditional(conditional) => {
                let then = self.gcc_enumeration(conditional.then_expression)?;
                (self.gcc_enumeration(conditional.else_expression)? == then).then_some(then)
            },
            | E::OmittedConditional(_) => None,
            | E::Binary {
                operator: B::Comma,
                right_expression,
                ..
            } => self.gcc_enumeration(right_expression),
            | E::Identifier(_) => {
                let info = self.expression_info(e);
                info.binding
                    .and_then(|binding| self.reachability.enumerator_tags.get(&binding).copied())
                    .or_else(|| self.enumeration_of(info.ty))
            },
            | _ => self.enumeration_of(self.expression_info(e).ty),
        }
    }

    fn enumeration_of(&self, ty: TypeId) -> Option<usize> {
        match self.types.nodes[self.types.non_atomic(ty).index] {
            | TypeKind::Tag(tag) if self.types.tags[tag].kind == TagKind::Enum => Some(tag),
            | _ => None,
        }
    }

    /// A `case` the controlling type, before promotion, cannot hold.
    /// C99: §6.8.4.2p5, p. 134; PDF p. 146.
    fn check_case_range(&mut self, ty: TypeId, cases: &[Case<'tu>], gcc: bool) {
        let Some((bits, signed)) = self.integer_type(ty) else {
            return;
        };
        if bits >= 64 {
            return;
        }
        let (low, high) = if signed {
            (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1)
        } else {
            (0, (1_i128 << bits) - 1)
        };
        for case in cases.iter().filter(|case| !case.range) {
            let Some((expression, value)) = self.written_case_value(case) else {
                continue;
            };
            if (low..=high).contains(&value) {
                continue;
            }
            if gcc {
                let text = if value > high {
                    "case label value exceeds maximum value for type"
                } else {
                    "case label value is less than minimum value for type"
                };
                self.switch_warning(G::SwitchOutsideRange, self.first_vector(case.source), text);
            } else {
                // Clang asks only of a `case` wider than the controlling type:
                // a negative `case` of an unsigned type of its width keeps an
                // implementation-defined value.
                let written = self.integer_type(self.expression_info(expression).ty);
                if written.is_none_or(|(written, _)| written <= bits) {
                    continue;
                }
                // The value the conversion to the controlling type keeps.
                let kept = value & ((1_i128 << bits) - 1);
                let kept = if signed && kept > high {
                    kept - (1_i128 << bits)
                } else {
                    kept
                };
                let text = format_in!(
                    self.scratch,
                    "overflow converting case value to switch condition type ({value} to {kept})"
                );
                self.switch_warning(G::Switch, expression.source_vectors, text);
            }
        }
    }

    /// A `case`'s expression, which is not a range.
    fn case_expression(case: &Case<'tu>) -> Option<&'tu Expression<'tu>> {
        let S::Case(ConstantExpressionSlot::Parsed(slot), _) = case.statement.kind else {
            return None;
        };
        Some(slot.expression())
    }

    /// A `case`'s expression and its value as written, before conversion.
    fn written_case_value(&self, case: &Case<'tu>) -> Option<(&'tu Expression<'tu>, i128)> {
        let S::Case(ConstantExpressionSlot::Parsed(slot), _) = case.statement.kind else {
            return None;
        };
        let expression = slot.expression();
        let value = self.expression_info(expression).integer?.to_i128()?;
        Some((expression, value))
    }

    fn first_vector(&self, source: SourceVectors) -> SourceVectors {
        _ = self;
        crate::translation_phases::context::Context::first_token_vector(source)
    }

    fn switch_warning(&mut self, group: G, site: SourceVectors, text: &str) {
        let name = self.context.string_cache.intern(text);
        self.spelled_group_diagnostic(
            group,
            SemanticErrorKind::SwitchCases(group),
            site,
            Some(name),
            site,
        );
    }
}

/// `value` converted to an integer type of `bits` bits.
/// C99: §6.3.1.3p2-3, p. 43; PDF p. 55.
const fn converted(value: i128, bits: u32, signed: bool) -> i128 {
    if bits >= 128 {
        return value;
    }
    let kept = value & ((1_i128 << bits) - 1);
    if signed && kept >> (bits - 1) != 0 {
        kept - (1_i128 << bits)
    } else {
        kept
    }
}
