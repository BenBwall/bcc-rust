//! x86 attributes that change how the machine enters and leaves a function:
//! `interrupt`, `no_caller_saved_registers` and GCC's `naked`. Each compiler's
//! checks stand here in its words; lowering carries the retained facts to the
//! IR's [`EntryConvention`](crate::ir::EntryConvention), and the back ends
//! implement it.
//! GNU extension: GCC manual, "x86 Function Attributes".
//! <https://gcc.gnu.org/onlinedocs/gcc/x86-Function-Attributes.html>
//! Clang extension: Clang Attributes Reference, "interrupt (X86)".

use super::{
    entities::Subject,
    written::WrittenAttribute,
};
use crate::{
    target::Scalar,
    translation_phases::{
        parsing::syntax::Expression,
        semantic_analysis::{
            Analyzer,
            BindingKind,
            DefinitionKind,
            MachineEntryFault as Fault,
            SemanticErrorKind,
            TypeId,
            TypeKind,
            UnsupportedAttribute,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Applies `interrupt` to `subject` after checking the function's
    /// signature as the compiler being followed checks it.
    pub(super) fn interrupt_attribute(
        &mut self,
        subject: Subject,
        attribute: WrittenAttribute<'_>,
    ) {
        // GCC takes the attribute without arguments alone; Clang's spelling
        // ignores any.
        if self.gcc_identity() && !self.no_arguments(attribute) {
            return;
        }
        match subject {
            | Subject::Function { binding, .. } => {
                let ty = self.bindings[binding].ty;
                if self.interrupt_signature(ty, attribute) {
                    let entity = self.entity_of(binding);
                    self.attributes.entity(entity).interrupt = true;
                    self.attributes.machine_entries = true;
                }
            },
            // A pointer to a handler's type is called as any other function
            // pointer: both compilers accept the attribute and neither
            // diagnoses the call.
            | Subject::Object { binding, .. } if self.points_to_function(binding) => {},
            // `typedef __attribute__((interrupt)) void handler(void *);`
            // declares functions whose entry bcc would not see.
            | Subject::Typedef(_) =>
                self.refuse(UnsupportedAttribute::CallingConvention, attribute.source),
            | _ => self.ignored(attribute),
        }
    }

    /// Applies `no_caller_saved_registers` to `subject`.
    pub(super) fn no_caller_saved_attribute(
        &mut self,
        subject: Subject,
        attribute: WrittenAttribute<'_>,
    ) {
        if !self.no_arguments(attribute) {
            return;
        }
        match subject {
            | Subject::Function { binding, .. } => {
                let entity = self.entity_of(binding);
                self.attributes.entity(entity).no_caller_saved = true;
                self.attributes.machine_entries = true;
            },
            | Subject::Object { binding, .. } if self.points_to_function(binding) => {},
            | Subject::Typedef(_) =>
                self.refuse(UnsupportedAttribute::CallingConvention, attribute.source),
            | _ => self.ignored(attribute),
        }
    }

    /// Applies GCC's `naked` to `subject`. Clang's stays refused by name
    /// where the specifier is checked.
    pub(super) fn naked_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        if !self.gcc_identity() || !self.no_arguments(attribute) {
            return;
        }
        if let Subject::Function { binding, .. } = subject {
            let entity = self.entity_of(binding);
            self.attributes.entity(entity).naked = true;
        } else {
            self.ignored(attribute);
        }
    }

    /// Whether object `binding` is a pointer to a function.
    fn points_to_function(&self, binding: usize) -> bool {
        let TypeKind::Pointer(pointee) = self.types.nodes[self.bindings[binding].ty.index] else {
            return false;
        };
        matches!(self.types.nodes[pointee.index], TypeKind::Function { .. })
    }

    /// Whether a function of type `ty` can be a handler, reporting why not.
    fn interrupt_signature(&mut self, ty: TypeId, attribute: WrittenAttribute<'_>) -> bool {
        let TypeKind::Function {
            result,
            parameters,
            prototype,
            ..
        } = self.types.nodes[ty.index]
        else {
            return false;
        };
        let void = matches!(
            self.types.nodes[result.index],
            TypeKind::Scalar(Scalar::Void)
        );
        if !self.gcc_identity() && !prototype {
            // Clang ignores the attribute on a function without a prototype.
            self.ignored(attribute);
            return false;
        }
        let mut faults = ArenaVec::new_in(self.scratch);
        if self.gcc_identity() {
            self.gcc_interrupt_faults(parameters, void, &mut faults);
        } else {
            faults.extend(self.clang_interrupt_fault(parameters, void));
        }
        for &fault in &faults {
            self.error(
                SemanticErrorKind::InvalidMachineEntry(fault),
                attribute.source,
                None,
                None,
            );
        }
        faults.is_empty()
    }

    /// GCC's `ix86_handle_interrupt_attribute`: the second parameter is any
    /// integer type of the word's width, and every check reports.
    fn gcc_interrupt_faults(
        &self,
        parameters: &[TypeId],
        void: bool,
        faults: &mut ArenaVec<'_, Fault>,
    ) {
        if parameters
            .first()
            .is_some_and(|&first| !self.is_pointer(first))
        {
            faults.push(Fault::GccFirstParameter);
        }
        if parameters
            .get(1)
            .is_some_and(|&second| self.word_integer(second).is_none())
        {
            faults.push(Fault::GccSecondParameter);
        }
        if !(1..=2).contains(&parameters.len()) {
            faults.push(Fault::GccParameterCount);
        }
        if !void {
            faults.push(Fault::GccResult);
        }
    }

    /// Clang's `handleAnyX86InterruptAttr`: the second parameter is an
    /// unsigned integer of the word's width, and the first failure reports.
    fn clang_interrupt_fault(&self, parameters: &[TypeId], void: bool) -> Option<Fault> {
        if !void {
            return Some(Fault::ClangResult);
        }
        if !(1..=2).contains(&parameters.len()) {
            return Some(Fault::ClangParameterCount);
        }
        if !self.is_pointer(parameters[0]) {
            return Some(Fault::ClangFirstParameter);
        }
        parameters
            .get(1)
            .is_some_and(|&second| self.word_integer(second) != Some(false))
            .then_some(Fault::ClangSecondParameter)
    }

    fn is_pointer(&self, ty: TypeId) -> bool {
        matches!(self.types.nodes[ty.index], TypeKind::Pointer(_))
    }

    /// Whether `ty` is a 64-bit integer type and, if so, whether it is
    /// signed.
    fn word_integer(&self, ty: TypeId) -> Option<bool> {
        let TypeKind::Scalar(scalar) = self.types.nodes[ty.index] else {
            return None;
        };
        let (width, signed) = self.types.target.integer(scalar)?;
        (width == 64).then_some(signed)
    }

    /// Checks a call of the function `target` names, if any: nothing may call
    /// a handler, and Clang warns about the registers a handler or a function
    /// with `no_caller_saved_registers` has to save around the call.
    pub(in crate::translation_phases::semantic_analysis) fn check_machine_entry_call(
        &mut self,
        call: &'tu Expression<'tu>,
        target: Option<usize>,
    ) {
        if !self.attributes.machine_entries {
            return;
        }
        let target = target
            .filter(|&binding| self.bindings[binding].kind == BindingKind::Function)
            .and_then(|binding| {
                let entity = self.entity_of(binding);
                self.attributes.entities.get(&entity).copied()
            });
        if target.is_some_and(|attributes| attributes.interrupt) {
            self.error(
                SemanticErrorKind::InvalidMachineEntry(Fault::DirectCall),
                call.source_vectors,
                None,
                None,
            );
            return;
        }
        if self.gcc_identity() || target.is_some_and(|attributes| attributes.no_caller_saved) {
            return;
        }
        let Some(enclosing) = self.functions.current.and_then(|current| current.binding) else {
            return;
        };
        let entity = self.entity_of(enclosing);
        let Some(&enclosing) = self.attributes.entities.get(&entity) else {
            return;
        };
        if enclosing.interrupt || enclosing.no_caller_saved {
            self.error(
                SemanticErrorKind::ExcessiveRegsave(enclosing.interrupt),
                call.source_vectors,
                None,
                None,
            );
        }
    }

    /// Checks each defined function with a machine entry once redeclarations
    /// have supplied its attributes, as GCC does when it compiles the body.
    pub(super) fn check_machine_entries(&mut self) {
        if !self.gcc_identity() {
            return;
        }
        for index in 0..self.definitions.len() {
            let definition = self.definitions[index];
            if !matches!(
                definition.kind,
                DefinitionKind::Function | DefinitionKind::DiscardableFunction
            ) {
                continue;
            }
            let entity = self.entity_of(definition.binding);
            let Some(&attributes) = self.attributes.entities.get(&entity) else {
                continue;
            };
            let data = self.bindings[definition.binding];
            let source = data.name.source_vectors;
            if attributes.interrupt && attributes.naked {
                self.error(
                    SemanticErrorKind::InvalidMachineEntry(Fault::Naked),
                    source,
                    None,
                    None,
                );
                continue;
            }
            if self.context.configuration.general_regs_only() {
                continue;
            }
            let exception = matches!(
                self.types.nodes[data.ty.index],
                TypeKind::Function { parameters, .. } if parameters.len() == 2
            );
            let fault = match (attributes.interrupt, attributes.no_caller_saved) {
                | (true, _) if exception => Fault::GccExceptionNeedsGeneralRegisters,
                | (true, _) => Fault::GccInterruptNeedsGeneralRegisters,
                | (false, true) if !attributes.naked =>
                    Fault::GccNoCallerSavedNeedsGeneralRegisters,
                | _ => continue,
            };
            self.error(
                SemanticErrorKind::InvalidMachineEntry(fault),
                source,
                None,
                None,
            );
        }
    }
}
