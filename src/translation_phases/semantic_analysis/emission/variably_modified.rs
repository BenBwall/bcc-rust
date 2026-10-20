//! Where Clang's code generation resumes at a declaration: a block-scope
//! object or typedef of a variably modified type needs its array sizes, so
//! `CodeGenFunction::EmitVariablyModifiedType` ensures an insertion point
//! before it evaluates them, even after a jump or a call that does not
//! return. Code generation then continues from that block, and the size
//! expressions are emitted. Declarations are simple statements, which
//! `EmitStmt` handles without an insertion point; any other statement it
//! skips there whole unless it holds a label, and a folded condition's
//! other arm or a folded switch's statements after the selected `break` it
//! never reaches. A declaration in such code resumes nothing, which
//! [`State::unemitted`](super::State) tracks.
//!
//! Semantic analysis learns that a declarator's type is variably modified
//! only after its array bounds, so a declaration met in skipped code that
//! could resume it is analyzed as emitted from its start: the references in
//! its declaration specifiers and declarator stay emitted when the declared
//! object or typedef has a variably modified type, and are marked skipped
//! again otherwise. Its initializer follows the decision.
//!
//! For `return; int v[n]; h(0);`, the reference to `n` and the call of `h`
//! are emitted; for `return; int k = n; h(0);`, neither is.
//!
//! C99: §6.7.5.2 paragraphs 2 and 4-5, pp. 116-117; PDF pp. 128-129;
//! §6.8 paragraph 3, p. 131; PDF p. 143.

use super::super::{
    Analyzer,
    BindingKind,
    TypeId,
    TypeKind,
    Work,
    functions::FunctionWork,
};
use crate::translation_phases::parsing::{
    declaration_syntax::{
        Declaration,
        DeclarationSpecifiers,
        Declarator,
    },
    syntax::{
        Statement,
        StatementType,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Marks statement `s` as one code generation never reaches when it is
    /// met in skipped code, is not simple and holds no label: `EmitStmt`
    /// returns without emitting it, so declarations inside it resume
    /// nothing. A statement expression's value statement is emitted anyway.
    pub(in crate::translation_phases::semantic_analysis) fn omit_unreached_statement(
        &mut self,
        s: &'tu Statement<'tu>,
    ) {
        use StatementType as S;
        let simple = matches!(
            s.kind,
            S::Null
                | S::Compound { .. }
                | S::Declaration(_)
                | S::Attributed(_)
                | S::Label(..)
                | S::Case(..)
                | S::CaseRange(_)
                | S::Default(_)
                | S::Goto(_)
                | S::Break
                | S::Continue
                | S::LocalLabels(_)
        );
        if !self.functions.dead
            || self.functions.emission.unemitted
            || simple
            || self
                .functions
                .emission
                .value_statements
                .contains_key(&std::ptr::from_ref(s).addr())
            || self.statement_contains_label(s, !self.case_labels_resume())
        {
            return;
        }
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreUnemitted(false)));
        self.functions.emission.unemitted = true;
    }

    /// Begins block-scope declaration `d`: in skipped code that a
    /// declaration can resume, its specifiers and first declarator are
    /// analyzed as emitted until [`Self::declarator_declared`] decides.
    pub(in crate::translation_phases::semantic_analysis) fn begin_declaration(
        &mut self,
        d: &'tu Declaration<'tu>,
    ) {
        if let Some(first) = d.init_declarators.first() {
            self.speculate(first.declarator);
        }
    }

    /// Begins declarator `d` of a block-scope declaration, as
    /// [`Self::begin_declaration`] does for one after the first.
    pub(in crate::translation_phases::semantic_analysis) fn begin_declarator(
        &mut self,
        d: Declarator<'tu>,
    ) {
        if self.functions.emission.speculation.is_none() {
            self.speculate(d);
        }
    }

    fn speculate(&mut self, d: Declarator<'tu>) {
        if self.scope != 0
            && self.functions.current.is_some()
            && self.functions.dead
            && !self.functions.emission.unemitted
            && self.functions.sizeof.is_none()
            && self.functions.reference_context.is_none()
        {
            self.functions.emission.speculation = Some((declarator_key(d), self.reference_count()));
            self.functions.dead = false;
        }
    }

    /// Decides declarator `d`, declared with specifiers `spec` and type
    /// `ty`: an object without linkage or static storage, or a typedef, of
    /// a variably modified type resumes code generation; any other leaves
    /// skipped what it was analyzed as emitted for.
    /// C99: §6.7.5.2 paragraph 2, p. 116; PDF p. 128.
    pub(in crate::translation_phases::semantic_analysis) fn declarator_declared(
        &mut self,
        d: Declarator<'tu>,
        spec: DeclarationSpecifiers<'tu>,
        ty: TypeId,
        binding: Option<usize>,
    ) {
        let Some((owner, start)) = self.functions.emission.speculation else {
            return;
        };
        if owner != declarator_key(d) {
            return;
        }
        self.functions.emission.speculation = None;
        let resumes = self.types.variably_modified(ty)
            && !matches!(self.types.nodes[ty.index], TypeKind::Function { .. })
            && binding.is_some_and(|binding| {
                matches!(
                    self.bindings[binding].kind,
                    BindingKind::Object | BindingKind::Typedef
                )
            })
            && !matches!(
                spec.storage_class,
                Some(
                    crate::translation_phases::parsing::syntax::StorageClass::Static
                        | crate::translation_phases::parsing::syntax::StorageClass::Extern
                )
            );
        if resumes {
            self.functions.emission.orphan = None;
        } else {
            self.skip_references(start..self.reference_count());
            self.functions.dead = true;
        }
    }
}

/// The identity of declarator `d`.
fn declarator_key(d: Declarator<'_>) -> usize {
    d.kind.as_slice().as_ptr().addr()
}
