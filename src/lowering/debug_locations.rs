//! What `-g` asks of lowering: the compilation unit, a subprogram for each
//! function, and the source location of the code being built, which
//! [`FunctionLowerer::track_line`] keeps current as it walks the syntax.
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
//! item 4, p. 10; PDF p. 22 (macro expansion).

use super::{
    FunctionLowerer,
    module_items::Unit,
};
use crate::{
    configuration::CStandard,
    ir::{
        DebugFile,
        DebugLanguage,
        DebugLoc,
        DebugUnit,
        FuncId,
        SourceLocation,
        Subprogram,
    },
    translation_phases::{
        SourceVector,
        semantic_analysis::FunctionRecord,
    },
    util::bump::ArenaString,
};

/// The source file index of the translation unit's main file, the first the
/// preprocessor interns.
const MAIN_FILE_INDEX: u32 = 0;

impl Unit<'_, '_, '_> {
    /// Starts the module's debug information if the unit asks for it.
    pub(super) fn begin_debug_info(&mut self) {
        let configuration = self.context.configuration;
        let Some(options) = configuration.debug_info() else {
            return;
        };
        let file = self.debug_file(MAIN_FILE_INDEX);
        self.debug_columns = options.columns;
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

    /// The debug file of the source file `index`, which may be a name
    /// `#line` presumes, as `__FILE__` reports it.
    fn debug_file(&mut self, index: u32) -> DebugFile {
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
        let file = self.unit.debug_file(name.source_file_index);
        let mut text = ArenaString::new_in(self.scratch);
        text.push_str(self.name);
        let (debug, arena) = self.unit.module.debug_info_mut();
        self.debug_scope = Some(debug.add_subprogram(
            arena,
            Subprogram {
                function,
                name: &text,
                file,
                line: name.line,
                scope_line: body.line,
            },
        ));
        self.draft.locating = true;
    }

    /// The debug location of code at `vector`.
    pub(super) fn debug_location(&mut self, vector: &SourceVector) -> Option<DebugLoc> {
        let scope = self.debug_scope?;
        let file = self.unit.debug_file(vector.source_file_index);
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
                    line: vector.line,
                    column,
                    scope,
                    inlined_at: None,
                }),
        )
    }
}
