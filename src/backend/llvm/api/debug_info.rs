//! LLVM's `DIBuilder` behind the session's brand. Metadata nodes are uniqued
//! or owned by the session's context, so handles stay valid after the builder
//! that made them is disposed; the builder only finishes what it deferred.

use std::{
    marker::PhantomData,
    ptr,
};

use llvm_sys::{
    core::LLVMSetCurrentDebugLocation2,
    debuginfo::{
        LLVMCreateDIBuilder,
        LLVMDIBuilderCreateCompileUnit,
        LLVMDIBuilderCreateDebugLocation,
        LLVMDIBuilderCreateFile,
        LLVMDIBuilderCreateFunction,
        LLVMDIBuilderCreateLexicalBlockFile,
        LLVMDIBuilderCreateSubroutineType,
        LLVMDIBuilderFinalize,
        LLVMDIFlagPrototyped,
        LLVMDWARFEmissionKind,
        LLVMDWARFSourceLanguage,
        LLVMDebugMetadataVersion,
        LLVMDisposeDIBuilder,
        LLVMSetSubprogram,
    },
    prelude::{
        LLVMDIBuilderRef,
        LLVMMetadataRef,
    },
};

use super::{
    Session,
    Val,
};
use crate::ir::{
    DebugLanguage,
    DebugLevel,
};

/// The module-flag behavior that warns when two linked modules disagree, as
/// Clang gives its debug flags.
const MODULE_FLAG_WARNING: u64 = 2;

/// A debug metadata node of this session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in super::super) struct Meta<'id>(LLVMMetadataRef, PhantomData<fn(&'id ()) -> &'id ()>);

/// The session's `DIBuilder`. It finalizes when finished, which every path
/// that keeps the module's debug information does before verifying it.
pub(in super::super) struct DebugBuilder<'a, 'id> {
    builder: LLVMDIBuilderRef,
    api:     &'a Session<'id>,
}

/// What a compile unit records about the compilation.
#[derive(Clone, Copy)]
pub(in super::super) struct CompileUnit<'a, 'id> {
    pub(in super::super) file:      Meta<'id>,
    pub(in super::super) language:  DebugLanguage,
    pub(in super::super) producer:  &'a str,
    pub(in super::super) optimized: bool,
    pub(in super::super) level:     DebugLevel,
}

/// A function as a subprogram describes it.
#[derive(Clone, Copy)]
pub(in super::super) struct FunctionDescription<'a, 'id> {
    pub(in super::super) scope:      Meta<'id>,
    pub(in super::super) name:       &'a str,
    pub(in super::super) symbol:     &'a str,
    pub(in super::super) file:       Meta<'id>,
    pub(in super::super) line:       u32,
    pub(in super::super) scope_line: u32,
    pub(in super::super) ty:         Meta<'id>,
    pub(in super::super) local:      bool,
    pub(in super::super) optimized:  bool,
}

impl<'id> Session<'id> {
    /// A `DIBuilder` for the session's module, after recording in the module
    /// which debug format its objects carry: `dwarf` names a DWARF version,
    /// otherwise the module's debug information is CodeView, as Clang's is
    /// for an MSVC target.
    pub(in super::super) fn debug_builder<'a>(
        &'a self,
        dwarf: Option<u8>,
    ) -> DebugBuilder<'a, 'id> {
        match dwarf {
            | Some(version) =>
                self.module_flag(MODULE_FLAG_WARNING, "Dwarf Version", u64::from(version)),
            | None => self.module_flag(MODULE_FLAG_WARNING, "CodeView", 1),
        }
        // SAFETY: Takes no arguments and only reports a constant.
        let version = unsafe { LLVMDebugMetadataVersion() };
        self.module_flag(
            MODULE_FLAG_WARNING,
            "Debug Info Version",
            u64::from(version),
        );
        DebugBuilder {
            // SAFETY: The session's module is live, and the builder is
            // disposed before the session disposes the module.
            builder: unsafe { LLVMCreateDIBuilder(self.module) },
            api:     self,
        }
    }

    /// Attaches `subprogram` to the function `function`.
    pub(in super::super) fn set_subprogram(&self, function: Val<'id>, subprogram: Meta<'id>) {
        // SAFETY: The type system ties both handles to this live session.
        unsafe { LLVMSetSubprogram(function.0, subprogram.0) }
    }

    /// Gives the instructions built from now on `location`.
    pub(in super::super) fn set_debug_location(&self, location: Option<Meta<'id>>) {
        let location = location.map_or(ptr::null_mut(), |location| location.0);
        // SAFETY: The builder is live and the location, when there is one, a
        // `DILocation` of its context; null clears it.
        unsafe { LLVMSetCurrentDebugLocation2(self.builder, location) }
    }

    /// A `DILocation` in `scope`, inlined at `inlined_at`.
    pub(in super::super) fn debug_location(
        &self,
        line: u32,
        column: u32,
        scope: Meta<'id>,
        inlined_at: Option<Meta<'id>>,
    ) -> Meta<'id> {
        let inlined_at = inlined_at.map_or(ptr::null_mut(), |location| location.0);
        // SAFETY: The context is live and the handles are this session's.
        Meta(
            unsafe {
                LLVMDIBuilderCreateDebugLocation(self.context, line, column, scope.0, inlined_at)
            },
            PhantomData,
        )
    }
}

impl<'id> DebugBuilder<'_, 'id> {
    /// A `DIFile` for `name`, which `directory` is relative to.
    pub(in super::super) fn file(&self, name: &str, directory: &str) -> Meta<'id> {
        // SAFETY: The counted strings are live while LLVM copies them.
        Meta(
            unsafe {
                LLVMDIBuilderCreateFile(
                    self.builder,
                    name.as_ptr().cast(),
                    name.len(),
                    directory.as_ptr().cast(),
                    directory.len(),
                )
            },
            PhantomData,
        )
    }

    /// The unit's `DICompileUnit`.
    pub(in super::super) fn compile_unit(&self, unit: CompileUnit<'_, 'id>) -> Meta<'id> {
        let language = match unit.language {
            | DebugLanguage::C89 => LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC89,
            | DebugLanguage::C99 => LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC99,
            | DebugLanguage::C11 => LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC11,
        };
        let kind = if unit.level == DebugLevel::Lines {
            LLVMDWARFEmissionKind::LLVMDWARFEmissionKindLineTablesOnly
        } else {
            LLVMDWARFEmissionKind::LLVMDWARFEmissionKindFull
        };
        // SAFETY: The counted strings are live while LLVM copies them, the
        // empty ones with a length of 0 are never read, and the file is this
        // session's.
        Meta(
            unsafe {
                LLVMDIBuilderCreateCompileUnit(
                    self.builder,
                    language,
                    unit.file.0,
                    unit.producer.as_ptr().cast(),
                    unit.producer.len(),
                    i32::from(unit.optimized),
                    c"".as_ptr(),
                    0,
                    0,
                    c"".as_ptr(),
                    0,
                    kind,
                    0,
                    0,
                    0,
                    c"".as_ptr(),
                    0,
                    c"".as_ptr(),
                    0,
                )
            },
            PhantomData,
        )
    }

    /// A subroutine type that says nothing about its parameters.
    pub(in super::super) fn unknown_subroutine_type(&self, file: Meta<'id>) -> Meta<'id> {
        let mut result: [LLVMMetadataRef; 1] = [ptr::null_mut()];
        // SAFETY: The file is this session's and the one null element, which
        // stands for a void result, is read during the call.
        Meta(
            unsafe {
                LLVMDIBuilderCreateSubroutineType(
                    self.builder,
                    file.0,
                    result.as_mut_ptr(),
                    1,
                    LLVMDIFlagPrototyped,
                )
            },
            PhantomData,
        )
    }

    /// A definition's `DISubprogram`.
    pub(in super::super) fn function(&self, function: FunctionDescription<'_, 'id>) -> Meta<'id> {
        let symbol = if function.symbol == function.name {
            ""
        } else {
            function.symbol
        };
        // SAFETY: The counted strings are live while LLVM copies them, and
        // the handles are this session's.
        Meta(
            unsafe {
                LLVMDIBuilderCreateFunction(
                    self.builder,
                    function.scope.0,
                    function.name.as_ptr().cast(),
                    function.name.len(),
                    symbol.as_ptr().cast(),
                    symbol.len(),
                    function.file.0,
                    function.line,
                    function.ty.0,
                    i32::from(function.local),
                    1,
                    function.scope_line,
                    LLVMDIFlagPrototyped,
                    i32::from(function.optimized),
                )
            },
            PhantomData,
        )
    }

    /// `scope` with the code in it attributed to `file`: what lets one
    /// function's lines come from several files.
    pub(in super::super) fn file_scope(&self, scope: Meta<'id>, file: Meta<'id>) -> Meta<'id> {
        // SAFETY: The handles are this session's.
        Meta(
            unsafe { LLVMDIBuilderCreateLexicalBlockFile(self.builder, scope.0, file.0, 0) },
            PhantomData,
        )
    }

    /// Finishes what the builder deferred, as the module verifier needs.
    pub(in super::super) fn finalize(&self) {
        // SAFETY: The builder and its module are live.
        unsafe { LLVMDIBuilderFinalize(self.builder) }
    }

    pub(in super::super) const fn api(&self) -> &Session<'id> {
        self.api
    }
}

impl Drop for DebugBuilder<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: The builder is live and this is its one owner. LLVM asks
        // for it to be finalized first, which a path that keeps the module
        // has done; an error path discards the module.
        unsafe { LLVMDisposeDIBuilder(self.builder) }
    }
}
