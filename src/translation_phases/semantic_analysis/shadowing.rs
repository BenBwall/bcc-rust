//! GCC's and Clang's warnings about a declaration that hides another,
//! `-Wshadow` and GCC's `-Wshadow=local` and `-Wshadow=compatible-local`, and
//! about a statement whose body is only `;`, `-Wempty-body`. C99 §6.2.1p4
//! hides the outer identifier and §6.8.3p3 makes `;` a statement, so each is
//! a warning §5.1.1.3p1 permits.
//! GCC: `warn_if_shadowing` in `c-decl.cc`, `c_parser_if_body`.
//! Clang: `Sema::CheckShadow`, `Sema::DiagnoseEmptyStmtBody` and
//! `Sema::DiagnoseEmptyLoopBody`.
//! C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42; §6.8.3p3, p. 132; PDF p. 144.

use super::{
    Analyzer,
    BindingKind,
    Linkage,
    ScopeKind,
    SemanticErrorKind,
    scopes::Entry,
    types::TypeId,
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
            BlockItem,
            Identifier,
            Statement,
            StatementType as S,
        },
    },
};

/// What a hidden declaration was, in GCC's words.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hidden {
    Parameter,
    Global,
    Local,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the declaration of `name`, which hides `visible` if it was
    /// declared in an enclosing scope.
    /// C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42.
    pub(super) fn check_shadow(
        &mut self,
        name: Identifier,
        ty: TypeId,
        kind: BindingKind,
        linkage: Linkage,
        visible: Option<Entry>,
    ) {
        if !self.unused.shadow || self.tainted {
            return;
        }
        let Some(visible) = visible.filter(|entry| entry.scope != self.scope) else {
            return;
        };
        let scope = self.scopes[self.scope].kind;
        if matches!(scope, ScopeKind::File | ScopeKind::Prototype) {
            return;
        }
        let old = self.bindings[visible.binding];
        if !matches!(old.origin, super::BindingOrigin::User) {
            return;
        }
        let old_global = self.scopes[old.scope].kind == ScopeKind::File;
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        let site = name.source_vectors;
        let previous = Some(old.name.source_vectors);
        if !gcc {
            self.clang_shadow(kind, linkage, old.kind, old_global, site, previous);
            return;
        }
        // GCC looks only at a hidden declaration whose warnings it would
        // report, so not at one in a system header
        // (`diagnostic_report_warnings_p`).
        if self.context.withholds(
            crate::translation_phases::ErrorSeverity::Warning,
            false,
            old.name.source_vectors,
        ) {
            return;
        }
        // GCC's: an `extern` declaration hides nothing, and only a pointer
        // to a function hides a function.
        if kind == BindingKind::Object && linkage == Linkage::External {
            return;
        }
        if old.kind == BindingKind::Function && !self.function_pointer(ty) {
            return;
        }
        let hidden = match old.kind {
            | BindingKind::Parameter => Hidden::Parameter,
            | BindingKind::Enumerator => Hidden::Local,
            | _ if old_global => Hidden::Global,
            | _ => Hidden::Local,
        };
        // GCC compares the two declarations' types whatever they declare: a
        // typedef has the type it names and an enumeration constant being
        // defined has its value's, but one of a finished enumeration has the
        // enumeration's, which no other local declaration has.
        let compatible = hidden != Hidden::Global
            && old.kind != BindingKind::Enumerator
            && self.types.composite(old.ty, ty).is_some();
        let group = if self.context.group_severity(G::Shadow, site).is_some() {
            G::Shadow
        } else if compatible
            && self
                .context
                .group_severity(G::ShadowCompatibleLocal, site)
                .is_some()
        {
            G::ShadowCompatibleLocal
        } else if hidden != Hidden::Global {
            G::ShadowLocal
        } else {
            return;
        };
        let spelled = self.context.string_cache.at(name.name);
        let text = match hidden {
            | Hidden::Parameter => {
                format_in!(
                    self.scratch,
                    "declaration of '{spelled}' shadows a parameter"
                )
            },
            | Hidden::Global => {
                format_in!(
                    self.scratch,
                    "declaration of '{spelled}' shadows a global declaration"
                )
            },
            | Hidden::Local => {
                format_in!(
                    self.scratch,
                    "declaration of '{spelled}' shadows a previous local"
                )
            },
        };
        self.shadow_warning(group, site, text, previous);
    }

    /// Clang's `CheckShadow`: a variable hides a variable or parameter, an
    /// enumeration constant hides any of those, a function or another
    /// enumeration constant, which Clang words as a variable, and a typedef
    /// hides a typedef.
    fn clang_shadow(
        &mut self,
        kind: BindingKind,
        linkage: Linkage,
        old: BindingKind,
        old_global: bool,
        site: SourceVectors,
        previous: Option<SourceVectors>,
    ) {
        let variable = |kind| matches!(kind, BindingKind::Object | BindingKind::Parameter);
        let hides = match kind {
            | BindingKind::Object | BindingKind::Parameter =>
                (linkage == Linkage::None
                    || (kind == BindingKind::Object && linkage == Linkage::Internal))
                    && variable(old),
            | BindingKind::Enumerator => old != BindingKind::Typedef,
            | BindingKind::Typedef => old == BindingKind::Typedef,
            | BindingKind::Function => false,
        };
        if !hides {
            return;
        }
        let text = if kind == BindingKind::Typedef {
            if old_global {
                "declaration shadows a typedef in the global scope"
            } else {
                let function = self
                    .functions
                    .current
                    .and_then(|function| function.binding)
                    .map_or("", |binding| {
                        self.context
                            .string_cache
                            .at(self.bindings[binding].name.name)
                    });
                format_in!(
                    self.scratch,
                    "declaration shadows a typedef in function '{function}'"
                )
            }
        } else if old_global {
            "declaration shadows a variable in the global scope"
        } else {
            "declaration shadows a local variable"
        };
        self.shadow_warning(G::Shadow, site, text, previous);
    }

    fn function_pointer(&self, ty: TypeId) -> bool {
        let ty = self.types.non_atomic(ty).unqualified();
        matches!(self.types.nodes[ty.index], super::types::TypeKind::Pointer(target)
            if matches!(self.types.nodes[self.types.non_atomic(target).unqualified().index],
                super::types::TypeKind::Function { .. }))
    }

    /// Checks the body of the selection or iteration statement `s`: GCC's
    /// `-Wextra` asks of an `if`, `else` or `do` body that is only `;`, and
    /// Clang's of an `if` or `switch` whose `;` follows on the same line.
    /// C99: §6.8.3p3, p. 132; PDF p. 144.
    pub(super) fn check_empty_body(&mut self, s: &'tu Statement<'tu>) {
        if !self.unused.empty_body {
            return;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        match s.kind {
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } =>
                if gcc {
                    if else_statement.is_none() && matches!(then_statement.kind, S::Null) {
                        self.empty_body_warning(
                            then_statement.source_vectors,
                            "suggest braces around empty body in an 'if' statement",
                        );
                    }
                    if let Some(otherwise) = else_statement
                        && matches!(otherwise.kind, S::Null)
                    {
                        self.empty_body_warning(
                            otherwise.source_vectors,
                            "suggest braces around empty body in an 'else' statement",
                        );
                    }
                } else if else_statement.is_none()
                    && super::statements::slot_expression(condition_expression).is_some()
                    && self.null_on_same_line(s, then_statement)
                {
                    self.empty_body_warning(
                        then_statement.source_vectors,
                        "if statement has empty body",
                    );
                },
            | S::Switch {
                condition_expression,
                body_statement,
            } if !gcc => {
                // Clang compares a `switch` body with its condition's end.
                if let Some(condition) = super::statements::slot_expression(condition_expression)
                    && let Some(parenthesis) =
                        crate::translation_phases::context::Context::token_before(
                            s.source_vectors,
                            body_statement.source_vectors,
                        )
                    && self.null_on_line_of(
                        body_statement,
                        crate::translation_phases::context::Context::last_source_vector(
                            condition.source_vectors,
                        ),
                        parenthesis,
                    )
                {
                    self.empty_body_warning(
                        body_statement.source_vectors,
                        "switch statement has empty body",
                    );
                }
            },
            | S::DoWhile { body_statement, .. } if gcc => {
                if matches!(body_statement.kind, S::Null) {
                    self.empty_body_warning(
                        body_statement.source_vectors,
                        "suggest braces around empty body in 'do' statement",
                    );
                }
            },
            | _ => {},
        }
    }

    /// Clang's check of a `for` or `while` loop whose body is a `;` on its own
    /// line's end, which it reports only if the statement after it looks like
    /// the intended body: a block, or one indented further than the loop.
    /// Clang: `Sema::DiagnoseEmptyLoopBody`.
    /// C99: §6.8.5p1, p. 135; PDF p. 147.
    pub(super) fn check_empty_loop_bodies(&mut self, items: &'tu [BlockItem<'tu>]) {
        if !self.unused.empty_body
            || self.context.configuration.compiler_identity() == CompilerIdentity::Gcc
        {
            return;
        }
        for pair in items.windows(2) {
            let [BlockItem::Statement(lp), BlockItem::Statement(next)] = *pair else {
                continue;
            };
            let (body, text) = match lp.kind {
                | S::While { body_statement, .. } => (body_statement, "while loop has empty body"),
                | S::For(f) => (f.body_statement, "for loop has empty body"),
                | _ => continue,
            };
            if !self.null_on_same_line(lp, body) {
                continue;
            }
            let indented = self.context.diagnostic_position(next.source_vectors).column
                > self.context.diagnostic_position(lp.source_vectors).column;
            if matches!(next.kind, S::Compound { .. }) || indented {
                self.empty_body_warning(body.source_vectors, text);
            }
        }
    }

    /// Whether `body`, the body of `statement`, is a `;` written on the line
    /// of the `)` before it, which Clang compares for an `if` or a loop
    /// (`ShouldDiagnoseEmptyStmtBody`).
    fn null_on_same_line(&self, statement: &'tu Statement<'tu>, body: &'tu Statement<'tu>) -> bool {
        crate::translation_phases::context::Context::token_before(
            statement.source_vectors,
            body.source_vectors,
        )
        .is_some_and(|parenthesis| self.null_on_line_of(body, parenthesis, parenthesis))
    }

    /// Whether `body` is a `;` written on the line where `header` is
    /// expanded, with no macro that expands to nothing between it and
    /// `before`, the token before it, which Clang's
    /// `NullStmt::hasLeadingEmptyMacro` exempts.
    fn null_on_line_of(
        &self,
        body: &'tu Statement<'tu>,
        header: SourceVectors,
        before: SourceVectors,
    ) -> bool {
        if !matches!(body.kind, S::Null)
            || body.source_vectors.length() == 0
            || header.length() == 0
            || self
                .context
                .produced_by_macro_expansion(body.source_vectors)
        {
            return false;
        }
        self.context.diagnostic_position(body.source_vectors).line
            == self.context.expansion_location(header).line
            && !self.empty_macro_between(before, body.source_vectors)
    }

    /// Whether the file spells something other than white space and
    /// comments between `before` and `after`, two consecutive tokens it
    /// spells: a macro that expanded to nothing.
    /// C99: §6.10.3p10, p. 152; PDF p. 164; §6.4.9p1-2, p. 66; PDF p. 78.
    fn empty_macro_between(&self, before: SourceVectors, after: SourceVectors) -> bool {
        if self.context.produced_by_macro_expansion(before) {
            return false;
        }
        let before = self.context.first_source_vector(before);
        let after = self.context.first_source_vector(after);
        if before.source_file_index != after.source_file_index {
            return false;
        }
        let Some(gap) = self
            .context
            .source_text(before.source_file_index)
            .and_then(|text| text.get(before.end()..after.index as usize))
        else {
            return false;
        };
        let mut rest = gap.trim_start();
        while !rest.is_empty() {
            rest = if let Some(comment) = rest.strip_prefix("/*") {
                comment.split_once("*/").map_or("", |(_, after)| after)
            } else if let Some(comment) = rest.strip_prefix("//") {
                comment.split_once('\n').map_or("", |(_, after)| after)
            } else if let Some(splice) = rest.strip_prefix('\\') {
                splice
            } else {
                return true;
            }
            .trim_start();
        }
        false
    }

    /// GCC's warning, or Clang's with its note at the same `;`.
    fn empty_body_warning(&mut self, site: SourceVectors, text: &str) {
        let name = self.context.string_cache.intern(text);
        let kind = SemanticErrorKind::ShadowOrBody(G::EmptyBody);
        if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
            self.spelled_group_diagnostic(G::EmptyBody, kind, site, Some(name), site);
        } else {
            self.group_diagnostic(G::EmptyBody, kind, site, Some(name), Some(site));
        }
    }

    fn shadow_warning(
        &mut self,
        group: G,
        site: SourceVectors,
        text: &str,
        previous: Option<SourceVectors>,
    ) {
        let name = self.context.string_cache.intern(text);
        if !self.context.spelled_in_system_header(site) {
            self.group_diagnostic(
                group,
                SemanticErrorKind::ShadowOrBody(group),
                site,
                Some(name),
                previous,
            );
        }
    }
}
