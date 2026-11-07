//! What `-g` asks of lowering: the compilation unit, a subprogram for each
//! function, and the source location of the code being built, which
//! [`FunctionLowerer::track_line`] keeps current as it walks the syntax. At
//! `-g` and above it also describes the variables, with the block scopes that
//! tell a shadowed name from what shadows it; `debug_types.rs` describes their
//! types.
//!
//! A location is the logical position of the file location of the code's
//! first token: `SourceVector` already holds what `#line` and `#include` make
//! its file and line, and inside a macro expansion
//! [`Context::file_vector`](crate::translation_phases::Context::file_vector)
//! gives the outermost invocation, as GCC's and Clang's line tables do.
//!
//! Nothing here runs without `-g`, and nothing else reads what it records
//! except the back end that writes debug information, so code generation
//! does not depend on it.
//!
//! C99: §6.10.4, pp. 154-155; PDF pp. 166-167 (`#line`); §5.1.1.2 paragraph 1
//! item 4, p. 10; PDF p. 22 (macro expansion); §6.2.1 paragraph 4, pp. 29-30;
//! PDF pp. 41-42 (block scopes).

use super::{
    FunctionLowerer,
    module_items::Unit,
    work::{
        Local,
        Task,
    },
};
use crate::{
    configuration::CStandard,
    ir::{
        DebugFile,
        DebugLanguage,
        DebugLevel,
        DebugLoc,
        DebugScope,
        DebugType,
        DebugUnit,
        DeclaredVariable,
        FuncId,
        GlobalId,
        GlobalVariable,
        LexicalBlock,
        Scope,
        SourceLocation,
        Subprogram,
        Variable,
        VariablePlace,
    },
    translation_phases::{
        SourceVector,
        parsing::{
            ParsedTranslationUnit,
            declaration_syntax::{
                Declaration,
                Declarator,
                DirectDeclarator,
                ParameterDeclaration,
                TypeSpecifiers,
            },
            syntax::{
                ExternalDeclaration,
                Statement,
            },
        },
        semantic_analysis::{
            BindingKind,
            FunctionRecord,
            TypeKind,
        },
    },
    util::bump::{
        ArenaString,
        ArenaVec,
    },
};

/// The source file index of the translation unit's main file, the first the
/// preprocessor interns.
pub(super) const MAIN_FILE_INDEX: u32 = 0;

impl Unit<'_, '_, '_> {
    /// Starts the module's debug information if the unit asks for it.
    pub(super) fn begin_debug_info(&mut self, syntax: &ParsedTranslationUnit<'_>) {
        let configuration = self.context.configuration;
        let Some(options) = configuration.debug_info() else {
            return;
        };
        let file = self.debug_file(MAIN_FILE_INDEX);
        self.debug_columns = options.columns;
        self.debug_physical = options.physical;
        self.debug_variables = options.level >= DebugLevel::Full;
        if self.debug_variables {
            self.index_typedefs(syntax);
        }
        self.module.debug_info_mut().0.set_unit(DebugUnit {
            file,
            language: match configuration.standard() {
                | CStandard::C89 | CStandard::C95 => DebugLanguage::C89,
                | CStandard::C99 => DebugLanguage::C99,
                | CStandard::C11 | CStandard::C17 | CStandard::C23 | CStandard::C2y =>
                    DebugLanguage::C11,
            },
            level: options.level,
            dwarf_version: options.dwarf_version,
            optimized: configuration.optimization().0,
        });
    }

    /// Finds the typedefs by name, and which ones the unit's file-scope
    /// declarations name; lowering a body does the same for its own.
    fn index_typedefs(&mut self, syntax: &ParsedTranslationUnit<'_>) {
        for binding in 0..self.sema.bindings.len() {
            let data = self.sema.bindings[binding];
            if data.kind == BindingKind::Typedef {
                self.debug_typedef_names
                    .entry(data.name.name)
                    .or_insert_with(|| ArenaVec::new_in(self.tables))
                    .push(binding);
            }
        }
        for declaration in syntax.external_declarations() {
            if let ExternalDeclaration::Declaration(declaration) = *declaration {
                self.note_declaration_typedefs(declaration);
            }
        }
    }

    /// Records the typedef `declaration` names for each object it declares.
    pub(super) fn note_declaration_typedefs(&mut self, declaration: &Declaration<'_>) {
        if !matches!(
            declaration.declaration_specifiers.type_specifiers,
            TypeSpecifiers::TypedefName(_)
        ) {
            return;
        }
        for init in declaration.init_declarators {
            if let Some(binding) = self
                .sema
                .correspondence
                .declaration(init.declarator)
                .and_then(|fact| fact.binding)
            {
                self.note_typedef_use(&declaration.declaration_specifiers, binding);
            }
        }
    }

    /// Describes the object `binding` as the global `global`, once, with
    /// `scope` for a static local.
    pub(super) fn describe_global(
        &mut self,
        binding: usize,
        global: GlobalId,
        scope: Option<DebugScope>,
    ) {
        if !self.debug_variables || !self.debug_described_globals.insert(binding) {
            return;
        }
        let data = self.sema.bindings[binding];
        let vector = self.context.file_vector(data.name.source_vectors);
        let line = vector.line;
        let file = self.debug_file(vector.source_file_index);
        let ty = self.debug_binding_type(binding);
        let (debug, arena) = self.module.debug_info_mut();
        let name = &*arena.alloc_str(self.context.string_cache.at(data.name.name));
        debug.add_global(GlobalVariable {
            global,
            name,
            scope,
            file,
            line,
            ty,
        });
    }

    /// The debug file of the source file `index`, which may be a name
    /// `#line` presumes, as `__FILE__` reports it.
    pub(super) fn debug_file(&mut self, index: u32) -> DebugFile {
        if let Some(&file) = self.debug_files.get(&index) {
            return file;
        }
        let path = self.context.get_source_file(index).to_string_lossy();
        let (debug, arena) = self.module.debug_info_mut();
        let file = debug.file(arena, &path);
        _ = self.debug_files.insert(index, file);
        file
    }
}

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Gives the function being lowered its scope, if the module has debug
    /// information, and starts recording the locations of its code. Its
    /// entry code, up to the first statement, has none: that is the
    /// prologue, which a back end reports at the scope line, as Clang does.
    pub(super) fn begin_subprogram(&mut self, record: &FunctionRecord<'_>, function: FuncId) {
        if self.unit.module.debug_info().unit().is_none() {
            return;
        }
        let context = self.unit.context;
        let source = record.syntax.source_vectors;
        let name_source = record
            .syntax
            .declarator
            .identifier()
            .map_or(source, |identifier| identifier.source_vectors);
        let name = context.file_vector(name_source);
        let body = context.file_vector(record.syntax.body.source_vectors);
        let (line, scope_line) = (self.debug_line(name), self.debug_line(body));
        let file = self.location_file(name);
        let ty = self.subprogram_type(record);
        let mut text = ArenaString::new_in(self.scratch);
        text.push_str(self.name);
        let (debug, arena) = self.unit.module.debug_info_mut();
        self.debug_scope = Some(debug.add_subprogram(
            arena,
            Subprogram {
                function,
                name: &text,
                file,
                line,
                scope_line,
                ty,
            },
        ));
        self.draft.locating = true;
    }

    /// The function type of the definition `record`, with the typedefs its
    /// result and parameters name, which also become the parameter
    /// variables'. `None` at `-g1`.
    fn subprogram_type(&mut self, record: &FunctionRecord<'_>) -> Option<DebugType> {
        if !self.unit.debug_variables {
            return None;
        }
        let binding = record.binding?;
        let ty = self.unit.sema.bindings[binding].ty;
        let syntax = record.syntax;
        let TypeKind::Function {
            result, parameters, ..
        } = self.unit.sema.types.kind(ty)
        else {
            return None;
        };
        let result_typedef = self
            .unit
            .typedef_named(&syntax.declaration_specifiers, result);
        let list =
            function_parameters(syntax.declarator).filter(|list| list.len() == parameters.len());
        let mut typedefs = ArenaVec::with_capacity_in(parameters.len(), self.scratch);
        for (index, &parameter) in parameters.iter().enumerate() {
            let typedef = list.and_then(|list| {
                self.unit
                    .typedef_named(&list[index].declaration_specifiers, parameter)
            });
            typedefs.push(typedef);
            if let Some(typedef) = typedef
                && let Some(body) = record
                    .parameter_facts
                    .get(index)
                    .and_then(|facts| facts.body_binding)
            {
                _ = self.unit.debug_typedefs.insert(body, typedef);
            }
        }
        self.unit.debug_function_type(ty, result_typedef, &typedefs)
    }

    /// Enters the block `statement` opens, unless it is the function's body,
    /// whose declarations share the function's scope with its parameters
    /// (C99 §6.2.1p4, pp. 29-30; PDF pp. 41-42). A task restores the scope
    /// around it once the tasks pushed after this have run.
    pub(super) fn enter_debug_block(&mut self, statement: &Statement<'_>) {
        if !self.unit.debug_variables || std::ptr::eq(statement, self.record.syntax.body) {
            return;
        }
        let Some(parent) = self.debug_scope else {
            return;
        };
        let function = self
            .unit
            .module
            .debug_info()
            .scope(parent)
            .map(Scope::function);
        let Some(function) = function else {
            return;
        };
        let vector = self.unit.context.file_vector(statement.source_vectors);
        let line = vector.line;
        let column = if self.unit.debug_columns {
            vector.column
        } else {
            0
        };
        let file = self.unit.debug_file(vector.source_file_index);
        self.debug_scope = Some(self.unit.module.debug_info_mut().0.add_block(LexicalBlock {
            parent,
            function,
            file,
            line,
            column,
        }));
        self.tasks.push(Task::RestoreDebugScope(parent));
    }

    /// Describes the automatic object `binding`, kept at `local`. A variable
    /// in SSA values has no one place, so it is described without one, and a
    /// debugger reports it as optimized out.
    /// C99: §6.2.4 paragraph 4, p. 32; PDF p. 44.
    pub(super) fn describe_local(&mut self, binding: usize, local: Local) {
        if !self.unit.debug_variables {
            return;
        }
        let Some(scope) = self.debug_scope else {
            return;
        };
        let data = self.unit.sema.bindings[binding];
        let context = self.unit.context;
        let vector = context.file_vector(data.name.source_vectors);
        let line = vector.line;
        let file = self.unit.debug_file(vector.source_file_index);
        let ty = self.unit.debug_binding_type(binding);
        let argument = self
            .record
            .parameter_facts
            .iter()
            .position(|facts| facts.body_binding == Some(binding))
            .filter(|_| data.kind == BindingKind::Parameter)
            .and_then(|index| u32::try_from(index + 1).ok());
        let (debug, arena) = self.unit.module.debug_info_mut();
        let name = &*arena.alloc_str(context.string_cache.at(data.name.name));
        let variable = debug.add_variable(Variable {
            name,
            scope,
            file,
            line,
            ty,
            argument,
        });
        let place = match local {
            | Local::Slot(slot) => VariablePlace::Slot(slot),
            | Local::Address(address) => VariablePlace::Address(address),
            | Local::Variable(_) | Local::StackPointer => return,
        };
        if let Some(location) = self.debug_location(vector) {
            self.draft.declared.push(DeclaredVariable {
                variable,
                place,
                location,
            });
        }
    }

    /// The debug location of code at `vector`.
    pub(super) fn debug_location(&mut self, vector: &SourceVector) -> Option<DebugLoc> {
        let scope = self.debug_scope?;
        let file = self.location_file(vector);
        let line = self.debug_line(vector);
        let column = if self.unit.debug_columns {
            vector.column
        } else {
            0
        };
        Some(
            self.unit
                .module
                .debug_info_mut()
                .0
                .location(SourceLocation {
                    file,
                    line,
                    column,
                    scope,
                    inlined_at: None,
                }),
        )
    }

    /// The line a location at `vector` holds: `#line`'s unless the unit asks
    /// for physical lines ([`DebugOptions::physical`]), where 0 stands for a
    /// line in another file.
    ///
    /// [`DebugOptions::physical`]: crate::configuration::DebugOptions::physical
    fn debug_line(&self, vector: &SourceVector) -> u32 {
        if self.unit.debug_physical {
            self.home_line(vector).unwrap_or(0)
        } else {
            vector.line
        }
    }

    /// The file a location at `vector` holds, chosen as [`Self::debug_line`]
    /// chooses its line.
    fn location_file(&mut self, vector: &SourceVector) -> DebugFile {
        let index = if self.unit.debug_physical {
            self.unit
                .context
                .physical_source_file(vector.source_file_index)
        } else {
            vector.source_file_index
        };
        self.unit.debug_file(index)
    }
}

/// The parameters of the function a definition's declarator declares: the
/// first parameter list after the name, at the nesting that holds the name,
/// since an outer one belongs to a function it returns a pointer to.
fn function_parameters(mut declarator: Declarator<'_>) -> Option<&[ParameterDeclaration<'_>]> {
    loop {
        let nested = declarator.kind.iter().find_map(|direct| match *direct {
            | DirectDeclarator::Parenthesized(parenthesized) => Some(parenthesized.declarator),
            | _ => None,
        });
        match nested {
            | Some(inner) => declarator = inner,
            | None =>
                return declarator.kind.iter().find_map(|direct| match *direct {
                    | DirectDeclarator::Function { parameter_list, .. } =>
                        Some(parameter_list.as_slice()),
                    | _ => None,
                }),
        }
    }
}
