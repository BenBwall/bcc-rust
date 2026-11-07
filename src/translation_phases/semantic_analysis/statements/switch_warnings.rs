//! GCC's and Clang's warnings about what a `switch` handles: `-Wswitch` (an
//! enumerator no `case` names, or a `case` its enumeration lacks),
//! `-Wswitch-enum` (the same with a `default`), `-Wswitch-default`, and a
//! `case` its controlling type cannot hold (GCC's `-Wswitch-outside-range`,
//! Clang's `-Wswitch`). §6.8.4.2p5 converts each `case` to the promoted
//! controlling type, so each is a warning §5.1.1.3p1 permits.
//! GCC: `c_do_switch_warnings` in `c-warn.cc`, `c_add_case_label`.
//! Clang: `Sema::ActOnFinishSwitchStmt`.
//! C99: §6.8.4.2p5, p. 134; PDF p. 146; §6.7.2.2p3, p. 105; PDF p. 117.

use super::{
    super::{
        Analyzer,
        SemanticErrorKind,
        types::{
            TagKind,
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
            ConstantExpressionSlot,
            Expression,
            StatementType as S,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the finished `switch` at `index`, whose labels are `cases`.
    /// C99: §6.8.4.2p5, p. 134; PDF p. 146.
    pub(super) fn check_switch_warnings(&mut self, index: usize, cases: &[Case<'tu>]) {
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
        let enumeration = match self.types.nodes[ty.index] {
            | TypeKind::Tag(tag) if self.types.tags[tag].kind == TagKind::Enum => Some(tag),
            | _ => None,
        };
        let Some(tag) = enumeration else {
            // Only a type promotion widens holds a `case` it cannot.
            if self.integer_type(ty).map(|(bits, _)| bits)
                < self.integer_type(switch.ty).map(|(bits, _)| bits)
            {
                self.check_case_range(ty, cases, gcc);
            }
            return;
        };
        let Some(enumerators) = self.reachability.enumerators.get(&tag) else {
            return;
        };
        let mut values = ArenaVec::new_in(self.scratch);
        values.extend(enumerators.iter().copied());
        // bcc keeps no typedef names, so an enumeration with no tag cannot be
        // named as both compilers name it, through its typedef.
        let type_name = self.type_spelling(ty, gcc);
        // A `case` its enumeration lacks.
        for case in cases.iter().filter(|case| !case.range) {
            let Some((expression, value)) = self.written_case_value(case) else {
                continue;
            };
            if values.iter().any(|&(enumerator, _)| enumerator == value) {
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
            self.switch_warning(G::Switch, site, text);
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
        if gcc {
            for &(value, name) in &values {
                if !covered(value) {
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
        values.sort_by_key(|&(value, _)| value);
        values.dedup_by_key(|&mut (value, _)| value);
        // Copied out, so that reporting does not hold the string cache.
        let scratch = self.scratch;
        let mut missing: ArenaVec<'_, &str> = ArenaVec::new_in(scratch);
        missing.extend(
            values
                .iter()
                .filter(|&&(value, _)| !covered(value))
                .map(|&(_, name)| &*scratch.alloc_str(self.context.string_cache.at(name))),
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

    /// A `case` the controlling type, before promotion, cannot hold.
    /// C99: §6.8.4.2p5, p. 134; PDF p. 146.
    fn check_case_range(
        &mut self,
        ty: crate::translation_phases::semantic_analysis::types::TypeId,
        cases: &[Case<'tu>],
        gcc: bool,
    ) {
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
