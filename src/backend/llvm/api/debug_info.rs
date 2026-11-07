//! LLVM's `DIBuilder` behind the session's brand. Metadata nodes are uniqued
//! or owned by the session's context, so handles stay valid after the builder
//! that made them is disposed; the builder only finishes what it deferred.

use std::{
    marker::PhantomData,
    ptr,
};

use llvm_sys::{
    core::{
        LLVMGlobalAddDebugInfo,
        LLVMSetCurrentDebugLocation2,
    },
    debuginfo::{
        LLVMCreateDIBuilder,
        LLVMDIBuilderCreateArrayType,
        LLVMDIBuilderCreateAutoVariable,
        LLVMDIBuilderCreateBasicType,
        LLVMDIBuilderCreateBitFieldMemberType,
        LLVMDIBuilderCreateCompileUnit,
        LLVMDIBuilderCreateDebugLocation,
        LLVMDIBuilderCreateEnumerationType,
        LLVMDIBuilderCreateEnumerator,
        LLVMDIBuilderCreateExpression,
        LLVMDIBuilderCreateFile,
        LLVMDIBuilderCreateForwardDecl,
        LLVMDIBuilderCreateFunction,
        LLVMDIBuilderCreateGlobalVariableExpression,
        LLVMDIBuilderCreateLexicalBlock,
        LLVMDIBuilderCreateLexicalBlockFile,
        LLVMDIBuilderCreateMemberType,
        LLVMDIBuilderCreateParameterVariable,
        LLVMDIBuilderCreatePointerType,
        LLVMDIBuilderCreateQualifiedType,
        LLVMDIBuilderCreateReplaceableCompositeType,
        LLVMDIBuilderCreateStructType,
        LLVMDIBuilderCreateSubroutineType,
        LLVMDIBuilderCreateTypedef,
        LLVMDIBuilderCreateUnionType,
        LLVMDIBuilderCreateUnspecifiedType,
        LLVMDIBuilderCreateVectorType,
        LLVMDIBuilderFinalize,
        LLVMDIBuilderGetOrCreateSubrange,
        LLVMDIBuilderInsertDeclareRecordAtEnd,
        LLVMDIFlagFwdDecl,
        LLVMDIFlagPrototyped,
        LLVMDIFlagZero,
        LLVMDWARFEmissionKind,
        LLVMDWARFSourceLanguage,
        LLVMDebugMetadataVersion,
        LLVMDisposeDIBuilder,
        LLVMMetadataReplaceAllUsesWith,
        LLVMSetSubprogram,
    },
    prelude::{
        LLVMDIBuilderRef,
        LLVMMetadataRef,
    },
};

use super::{
    Block,
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
#[repr(transparent)]
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

/// A member of a structure or union as debug information describes it.
#[derive(Clone, Copy)]
pub(in super::super) struct MemberDescription<'a, 'id> {
    pub(in super::super) name:                &'a str,
    pub(in super::super) file:                Meta<'id>,
    pub(in super::super) line:                u32,
    pub(in super::super) size_bits:           u64,
    pub(in super::super) offset_bits:         u64,
    /// What a bit-field's unit starts at, in the record.
    pub(in super::super) storage_offset_bits: u64,
    pub(in super::super) bit_field:           Option<u32>,
    pub(in super::super) ty:                  Meta<'id>,
}

/// A variable as debug information describes it.
#[derive(Clone, Copy)]
pub(in super::super) struct VariableDescription<'a, 'id> {
    pub(in super::super) name:     &'a str,
    pub(in super::super) scope:    Meta<'id>,
    pub(in super::super) file:     Meta<'id>,
    pub(in super::super) line:     u32,
    pub(in super::super) ty:       Meta<'id>,
    pub(in super::super) argument: Option<u32>,
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

#[expect(
    clippy::unused_self,
    reason = "The receiver binds handle types to this session's invariant brand."
)]
impl<'id> Session<'id> {
    /// A `DIBuilder` for the session's module, after recording in the module
    /// which debug format its objects carry: `dwarf` names a DWARF version,
    /// otherwise the module's debug information is `CodeView`, as Clang's is
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

impl Meta<'_> {
    /// The handle as LLVM's C API takes it.
    pub(in super::super) const fn raw(self) -> LLVMMetadataRef {
        self.0
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

    /// A base type. `encoding` is a `DW_ATE_` code.
    pub(in super::super) fn base_type(&self, name: &str, bits: u64, encoding: u32) -> Meta<'id> {
        // SAFETY: The counted string is live while LLVM copies it.
        Meta(
            unsafe {
                LLVMDIBuilderCreateBasicType(
                    self.builder,
                    name.as_ptr().cast(),
                    name.len(),
                    bits,
                    encoding,
                    LLVMDIFlagZero,
                )
            },
            PhantomData,
        )
    }

    /// The named type for what has no other description.
    pub(in super::super) fn unspecified_type(&self, name: &str) -> Meta<'id> {
        // SAFETY: The counted string is live while LLVM copies it.
        Meta(
            unsafe {
                LLVMDIBuilderCreateUnspecifiedType(self.builder, name.as_ptr().cast(), name.len())
            },
            PhantomData,
        )
    }

    pub(in super::super) fn pointer_type(
        &self,
        pointee: Option<Meta<'id>>,
        bits: u64,
    ) -> Meta<'id> {
        // SAFETY: The pointee is this session's or null for `void`.
        Meta(
            unsafe {
                LLVMDIBuilderCreatePointerType(
                    self.builder,
                    raw(pointee),
                    bits,
                    0,
                    0,
                    c"".as_ptr(),
                    0,
                )
            },
            PhantomData,
        )
    }

    /// `ty` with the qualifier `tag`, a `DW_TAG_` code.
    pub(in super::super) fn qualified_type(&self, tag: u32, ty: Option<Meta<'id>>) -> Meta<'id> {
        // SAFETY: The type is this session's or null for `void`.
        Meta(
            unsafe { LLVMDIBuilderCreateQualifiedType(self.builder, tag, raw(ty)) },
            PhantomData,
        )
    }

    /// An array or vector of `element`, with `count` elements or, for -1, an
    /// unknown number.
    pub(in super::super) fn array_type(
        &self,
        element: Meta<'id>,
        count: i64,
        (size_bits, align_bits): (u64, u32),
        vector: bool,
    ) -> Meta<'id> {
        // SAFETY: The builder is live.
        let mut subscripts = [unsafe { LLVMDIBuilderGetOrCreateSubrange(self.builder, 0, count) }];
        let create = if vector {
            LLVMDIBuilderCreateVectorType
        } else {
            LLVMDIBuilderCreateArrayType
        };
        // SAFETY: The element is this session's and the one subscript is read
        // during the call.
        Meta(
            unsafe {
                create(
                    self.builder,
                    size_bits,
                    align_bits,
                    element.0,
                    subscripts.as_mut_ptr(),
                    1,
                )
            },
            PhantomData,
        )
    }

    /// A structure or union to fill in once its members exist, which may
    /// point back at it. `tag` is a `DW_TAG_` code.
    pub(in super::super) fn replaceable_record(
        &self,
        tag: u32,
        name: &str,
        file: Meta<'id>,
        line: u32,
    ) -> Meta<'id> {
        // SAFETY: The counted strings are live while LLVM copies them, and
        // the file is this session's.
        Meta(
            unsafe {
                LLVMDIBuilderCreateReplaceableCompositeType(
                    self.builder,
                    tag,
                    name.as_ptr().cast(),
                    name.len(),
                    file.0,
                    file.0,
                    line,
                    0,
                    0,
                    0,
                    LLVMDIFlagFwdDecl,
                    c"".as_ptr(),
                    0,
                )
            },
            PhantomData,
        )
    }

    /// A structure or union declared and never defined here.
    pub(in super::super) fn incomplete_record(
        &self,
        tag: u32,
        name: &str,
        file: Meta<'id>,
        line: u32,
    ) -> Meta<'id> {
        // SAFETY: The counted strings are live while LLVM copies them, and
        // the file is this session's.
        Meta(
            unsafe {
                LLVMDIBuilderCreateForwardDecl(
                    self.builder,
                    tag,
                    name.as_ptr().cast(),
                    name.len(),
                    file.0,
                    file.0,
                    line,
                    0,
                    0,
                    0,
                    c"".as_ptr(),
                    0,
                )
            },
            PhantomData,
        )
    }

    /// A member of the record `scope`, a bit-field if `bit_field` gives its
    /// width and otherwise `size_bits` wide.
    pub(in super::super) fn member(
        &self,
        scope: Meta<'id>,
        member: MemberDescription<'_, 'id>,
    ) -> Meta<'id> {
        let name = member.name;
        let node = match member.bit_field {
            // SAFETY: The counted name is live while LLVM copies it, and the
            // handles are this session's.
            | Some(width) => unsafe {
                LLVMDIBuilderCreateBitFieldMemberType(
                    self.builder,
                    scope.0,
                    name.as_ptr().cast(),
                    name.len(),
                    member.file.0,
                    member.line,
                    u64::from(width),
                    member.offset_bits,
                    member.storage_offset_bits,
                    LLVMDIFlagZero,
                    member.ty.0,
                )
            },
            // SAFETY: As above.
            | None => unsafe {
                LLVMDIBuilderCreateMemberType(
                    self.builder,
                    scope.0,
                    name.as_ptr().cast(),
                    name.len(),
                    member.file.0,
                    member.line,
                    member.size_bits,
                    0,
                    member.offset_bits,
                    LLVMDIFlagZero,
                    member.ty.0,
                )
            },
        };
        Meta(node, PhantomData)
    }

    /// The finished node of a record with `elements` as its members.
    pub(in super::super) fn record(
        &self,
        union: bool,
        name: &str,
        (file, line): (Meta<'id>, u32),
        (size_bits, align_bits): (u64, u32),
        elements: &mut [Meta<'id>],
    ) -> Meta<'id> {
        let count = u32::try_from(elements.len()).expect("a record's members fit in u32");
        let elements = elements.as_mut_ptr().cast::<LLVMMetadataRef>();
        let node = if union {
            // SAFETY: The counted strings and elements are read during the
            // call; `Meta` is a transparent handle, and the rest are this
            // session's.
            unsafe {
                LLVMDIBuilderCreateUnionType(
                    self.builder,
                    file.0,
                    name.as_ptr().cast(),
                    name.len(),
                    file.0,
                    line,
                    size_bits,
                    align_bits,
                    LLVMDIFlagZero,
                    elements,
                    count,
                    0,
                    c"".as_ptr(),
                    0,
                )
            }
        } else {
            // SAFETY: As above.
            unsafe {
                LLVMDIBuilderCreateStructType(
                    self.builder,
                    file.0,
                    name.as_ptr().cast(),
                    name.len(),
                    file.0,
                    line,
                    size_bits,
                    align_bits,
                    LLVMDIFlagZero,
                    ptr::null_mut(),
                    elements,
                    count,
                    0,
                    ptr::null_mut(),
                    c"".as_ptr(),
                    0,
                )
            }
        };
        Meta(node, PhantomData)
    }

    /// Makes everything that names `temporary`, a replaceable record, name
    /// `replacement`. LLVM frees the temporary.
    #[expect(
        clippy::unused_self,
        reason = "The receiver ties the replacement to the builder that made the temporary."
    )]
    pub(in super::super) fn replace_record(&self, temporary: Meta<'id>, replacement: Meta<'id>) {
        // SAFETY: The temporary came from `replaceable_record`, is replaced
        // once, and no handle to it is used afterwards.
        unsafe { LLVMMetadataReplaceAllUsesWith(temporary.0, replacement.0) }
    }

    /// An enumeration of `constants`, each made by [`Self::enumerator`].
    pub(in super::super) fn enumeration(
        &self,
        name: &str,
        (file, line): (Meta<'id>, u32),
        (size_bits, align_bits): (u64, u32),
        constants: &mut [Meta<'id>],
        base: Option<Meta<'id>>,
    ) -> Meta<'id> {
        let count = u32::try_from(constants.len()).expect("an enum's constants fit in u32");
        // SAFETY: The counted string and constants are read during the call;
        // `Meta` is a transparent handle, and the rest are this session's.
        Meta(
            unsafe {
                LLVMDIBuilderCreateEnumerationType(
                    self.builder,
                    file.0,
                    name.as_ptr().cast(),
                    name.len(),
                    file.0,
                    line,
                    size_bits,
                    align_bits,
                    constants.as_mut_ptr().cast::<LLVMMetadataRef>(),
                    count,
                    raw(base),
                )
            },
            PhantomData,
        )
    }

    pub(in super::super) fn enumerator(&self, name: &str, value: i64) -> Meta<'id> {
        // SAFETY: The counted string is live while LLVM copies it.
        Meta(
            unsafe {
                LLVMDIBuilderCreateEnumerator(
                    self.builder,
                    name.as_ptr().cast(),
                    name.len(),
                    value,
                    0,
                )
            },
            PhantomData,
        )
    }

    pub(in super::super) fn typedef(
        &self,
        name: &str,
        ty: Option<Meta<'id>>,
        file: Meta<'id>,
        line: u32,
    ) -> Meta<'id> {
        // SAFETY: The counted string is live while LLVM copies it, and the
        // handles are this session's.
        Meta(
            unsafe {
                LLVMDIBuilderCreateTypedef(
                    self.builder,
                    raw(ty),
                    name.as_ptr().cast(),
                    name.len(),
                    file.0,
                    line,
                    file.0,
                    0,
                )
            },
            PhantomData,
        )
    }

    /// A subroutine type; the first of `types` is the result, null for
    /// `void`, and a null last marks unspecified parameters.
    pub(in super::super) fn subroutine_type(
        &self,
        file: Meta<'id>,
        types: &mut [LLVMMetadataRef],
        prototyped: bool,
    ) -> Meta<'id> {
        let count = u32::try_from(types.len()).expect("a function's parameters fit in u32");
        let flags = if prototyped {
            LLVMDIFlagPrototyped
        } else {
            LLVMDIFlagZero
        };
        // SAFETY: The file is this session's and the elements are read during
        // the call.
        Meta(
            unsafe {
                LLVMDIBuilderCreateSubroutineType(
                    self.builder,
                    file.0,
                    types.as_mut_ptr(),
                    count,
                    flags,
                )
            },
            PhantomData,
        )
    }

    /// A block scope inside `parent`.
    pub(in super::super) fn lexical_block(
        &self,
        parent: Meta<'id>,
        file: Meta<'id>,
        line: u32,
        column: u32,
    ) -> Meta<'id> {
        // SAFETY: The handles are this session's.
        Meta(
            unsafe {
                LLVMDIBuilderCreateLexicalBlock(self.builder, parent.0, file.0, line, column)
            },
            PhantomData,
        )
    }

    /// A local variable, or a parameter when `argument` gives its position.
    /// It is kept even if no record describes where it is, so that a debugger
    /// can say it was optimized out.
    pub(in super::super) fn local_variable(
        &self,
        variable: VariableDescription<'_, 'id>,
    ) -> Meta<'id> {
        let name = variable.name;
        let node = match variable.argument {
            // SAFETY: The counted name is live while LLVM copies it, and the
            // handles are this session's.
            | Some(argument) => unsafe {
                LLVMDIBuilderCreateParameterVariable(
                    self.builder,
                    variable.scope.0,
                    name.as_ptr().cast(),
                    name.len(),
                    argument,
                    variable.file.0,
                    variable.line,
                    variable.ty.0,
                    1,
                    LLVMDIFlagZero,
                )
            },
            // SAFETY: As above.
            | None => unsafe {
                LLVMDIBuilderCreateAutoVariable(
                    self.builder,
                    variable.scope.0,
                    name.as_ptr().cast(),
                    name.len(),
                    variable.file.0,
                    variable.line,
                    variable.ty.0,
                    1,
                    LLVMDIFlagZero,
                    0,
                )
            },
        };
        Meta(node, PhantomData)
    }

    /// An expression that says the variable is where its record points.
    fn empty_expression(&self) -> LLVMMetadataRef {
        // SAFETY: The builder is live, and a length of 0 reads no elements.
        unsafe { LLVMDIBuilderCreateExpression(self.builder, ptr::null_mut(), 0) }
    }

    /// Records at the end of `block`, as it is built, that `variable` lives
    /// at the address `storage`.
    pub(in super::super) fn declare(
        &self,
        storage: Val<'id>,
        variable: Meta<'id>,
        location: Meta<'id>,
        block: Block<'id>,
    ) {
        let expression = self.empty_expression();
        // SAFETY: The builder's module holds the handles.
        _ = unsafe {
            LLVMDIBuilderInsertDeclareRecordAtEnd(
                self.builder,
                storage.0,
                variable.0,
                expression,
                location.0,
                block.0,
            )
        };
    }

    /// Describes the LLVM global `global` as the object `variable` says.
    pub(in super::super) fn global_variable(
        &self,
        global: Val<'id>,
        variable: VariableDescription<'_, 'id>,
        local: bool,
    ) {
        let name = variable.name;
        let expression = self.empty_expression();
        // SAFETY: The counted name is live while LLVM copies it, and the
        // handles are this session's.
        let described = unsafe {
            LLVMDIBuilderCreateGlobalVariableExpression(
                self.builder,
                variable.scope.0,
                name.as_ptr().cast(),
                name.len(),
                c"".as_ptr(),
                0,
                variable.file.0,
                variable.line,
                variable.ty.0,
                i32::from(local),
                expression,
                ptr::null_mut(),
                0,
            )
        };
        // SAFETY: The global and its description are this session's.
        unsafe { LLVMGlobalAddDebugInfo(global.0, described) }
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

/// `meta` as LLVM's C API takes it, with null for none.
fn raw(meta: Option<Meta<'_>>) -> LLVMMetadataRef {
    meta.map_or(ptr::null_mut(), |meta| meta.0)
}

impl Drop for DebugBuilder<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: The builder is live and this is its one owner. LLVM asks
        // for it to be finalized first, which a path that keeps the module
        // has done; an error path discards the module.
        unsafe { LLVMDisposeDIBuilder(self.builder) }
    }
}
