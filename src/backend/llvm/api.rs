//! Safe, scoped ownership of LLVM-C objects. Handles carry a session brand;
//! LLVM owns their storage on its C++ heap, outside Rust's global allocator.
//! No LLVM owner is stored in an arena. Only this module calls llvm-sys.
//! Invariant lifetime brands enforce handle ownership in the type system.
//! Transparent handle slices are borrowed directly by LLVM-C, whose array
//! inputs are read-only despite `*mut`.

mod call_diagnostics;

use std::{
    cell::RefCell,
    ffi::{
        CStr,
        c_char,
        c_void,
    },
    fmt,
    marker::PhantomData,
    ptr,
    sync::Once,
};

use llvm_sys::{
    LLVMDLLStorageClass,
    LLVMDiagnosticSeverity,
    LLVMIntPredicate,
    LLVMLinkage,
    LLVMOpcode,
    LLVMRealPredicate,
    LLVMThreadLocalMode,
    LLVMVisibility,
    analysis::{
        LLVMVerifierFailureAction,
        LLVMVerifyModule,
    },
    core::{
        LLVMAddAlias2,
        LLVMAddAttributeAtIndex,
        LLVMAddCallSiteAttribute,
        LLVMAddCase,
        LLVMAddDestination,
        LLVMAddFunction,
        LLVMAddGlobal,
        LLVMAddIncoming,
        LLVMAliasSetAliasee,
        LLVMAppendBasicBlockInContext,
        LLVMArrayType2,
        LLVMBlockAddress,
        LLVMBuildAlloca,
        LLVMBuildArrayAlloca,
        LLVMBuildAtomicCmpXchg,
        LLVMBuildAtomicRMW,
        LLVMBuildBinOp,
        LLVMBuildBr,
        LLVMBuildCall2,
        LLVMBuildCast,
        LLVMBuildCondBr,
        LLVMBuildExtractValue,
        LLVMBuildFCmp,
        LLVMBuildFNeg,
        LLVMBuildFence,
        LLVMBuildFreeze,
        LLVMBuildGEP2,
        LLVMBuildICmp,
        LLVMBuildInBoundsGEP2,
        LLVMBuildIndirectBr,
        LLVMBuildInsertValue,
        LLVMBuildLoad2,
        LLVMBuildPhi,
        LLVMBuildRet,
        LLVMBuildRetVoid,
        LLVMBuildSelect,
        LLVMBuildStore,
        LLVMBuildSwitch,
        LLVMBuildUnreachable,
        LLVMBuildVAArg,
        LLVMConstAdd,
        LLVMConstArray2,
        LLVMConstBitCast,
        LLVMConstGEP2,
        LLVMConstIntOfArbitraryPrecision,
        LLVMConstNull,
        LLVMConstPtrToInt,
        LLVMConstStringInContext2,
        LLVMConstStructInContext,
        LLVMConstSub,
        LLVMConstTrunc,
        LLVMContextCreate,
        LLVMContextDispose,
        LLVMContextSetDiagnosticHandler,
        LLVMCreateBuilderInContext,
        LLVMCreateEnumAttribute,
        LLVMCreateMemoryBufferWithMemoryRange,
        LLVMCreateMessage,
        LLVMCreateStringAttribute,
        LLVMCreateTypeAttribute,
        LLVMDisposeBuilder,
        LLVMDisposeMemoryBuffer,
        LLVMDisposeMessage,
        LLVMDisposeModule,
        LLVMDoubleTypeInContext,
        LLVMFP128TypeInContext,
        LLVMFloatTypeInContext,
        LLVMFunctionType,
        LLVMGetBufferSize,
        LLVMGetBufferStart,
        LLVMGetDataLayoutStr,
        LLVMGetDiagInfoDescription,
        LLVMGetDiagInfoSeverity,
        LLVMGetEnumAttributeKindForName,
        LLVMGetFirstFunction,
        LLVMGetIntrinsicDeclaration,
        LLVMGetNamedFunction,
        LLVMGetNamedGlobalAlias,
        LLVMGetNextFunction,
        LLVMGetParam,
        LLVMGetPoison,
        LLVMGetTarget,
        LLVMInstructionEraseFromParent,
        LLVMInt1TypeInContext,
        LLVMInt8TypeInContext,
        LLVMInt16TypeInContext,
        LLVMInt32TypeInContext,
        LLVMInt64TypeInContext,
        LLVMInt128TypeInContext,
        LLVMIntTypeInContext,
        LLVMIntrinsicGetType,
        LLVMIsDeclaration,
        LLVMLookupIntrinsicID,
        LLVMModuleCreateWithNameInContext,
        LLVMPointerTypeInContext,
        LLVMPositionBuilderAtEnd,
        LLVMPrintModuleToString,
        LLVMSetAlignment,
        LLVMSetDLLStorageClass,
        LLVMSetDataLayout,
        LLVMSetExact,
        LLVMSetGlobalConstant,
        LLVMSetInitializer,
        LLVMSetLinkage,
        LLVMSetNSW,
        LLVMSetNUW,
        LLVMSetOperand,
        LLVMSetOrdering,
        LLVMSetSection,
        LLVMSetTarget,
        LLVMSetThreadLocalMode,
        LLVMSetValueName2,
        LLVMSetVisibility,
        LLVMSetVolatile,
        LLVMStructTypeInContext,
        LLVMTypeOf,
        LLVMVectorType,
        LLVMVoidTypeInContext,
        LLVMX86FP80TypeInContext,
    },
    error::{
        LLVMDisposeErrorMessage,
        LLVMGetErrorMessage,
    },
    ir_reader::LLVMParseIRInContext2,
    linker::LLVMLinkModules2,
    prelude::{
        LLVMBasicBlockRef,
        LLVMBuilderRef,
        LLVMContextRef,
        LLVMDiagnosticInfoRef,
        LLVMMemoryBufferRef,
        LLVMModuleRef,
        LLVMTypeRef,
        LLVMValueRef,
    },
    target::{
        LLVMInitializeX86AsmParser,
        LLVMInitializeX86AsmPrinter,
        LLVMInitializeX86Target,
        LLVMInitializeX86TargetInfo,
        LLVMInitializeX86TargetMC,
    },
    target_machine::{
        LLVMCodeGenFileType,
        LLVMCodeGenOptLevel,
        LLVMCodeModel,
        LLVMCreateTargetMachine,
        LLVMDisposeTargetMachine,
        LLVMGetTargetFromTriple,
        LLVMRelocMode,
        LLVMTargetMachineEmitToMemoryBuffer,
        LLVMTargetMachineRef,
    },
    transforms::pass_builder::{
        LLVMCreatePassBuilderOptions,
        LLVMDisposePassBuilderOptions,
        LLVMPassBuilderOptionsRef,
        LLVMRunPasses,
    },
};

use super::driver::OptLevel;
use crate::{
    ir::{
        InstFlags,
        Type,
    },
    target::{
        CodeOptions,
        Relocation,
        Target,
        TlsModel,
    },
    util::bump::{
        ArenaString,
        Bump,
    },
};

/// A session owns a context, module and builder in disposal order.
///
/// `with` introduces a fresh invariant brand: nested sessions cannot exchange
/// types, values or blocks, and neither a session nor its handles can escape
/// the closure. Its arena is reborrowed for that brand's scope, so no separate
/// arena lifetime is needed. Owned text, object buffers and errors are
/// unbranded.
pub(super) struct Session<'id> {
    builder:     LLVMBuilderRef,
    module:      LLVMModuleRef,
    context:     LLVMContextRef,
    arena:       &'id Bump,
    diagnostics: &'id dyn DiagnosticStatus,
    /// The relocation and thread-local storage models.
    code:        CodeOptions,
    _brand:      PhantomData<fn(&'id ()) -> &'id ()>,
}

/// The non-default linkages the back end assigns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Linkage {
    Internal,
    /// A definition that a strong one replaces at link time.
    Weak,
    /// A declaration that may stay undefined, with a null address.
    ExternWeak,
    /// A definition equal to every other of its name, of which the linker
    /// keeps any one.
    WeakOdr,
    /// A `WeakOdr` definition that a unit not referencing it may drop.
    LinkOnceOdr,
    /// A body that only inlining uses; the symbol is defined elsewhere.
    AvailableExternally,
    /// The special `llvm.global_ctors`, `llvm.global_dtors` and `llvm.used`
    /// arrays.
    Appending,
}

#[derive(Clone, Copy)]
// ABI-identical to the LLVMTypeRef it wraps, including in slices.
#[repr(transparent)]
pub(super) struct Ty<'id>(LLVMTypeRef, PhantomData<fn(&'id ()) -> &'id ()>);
#[derive(Clone, Copy)]
// ABI-identical to the LLVMValueRef it wraps, including in slices.
#[repr(transparent)]
pub(super) struct Val<'id>(LLVMValueRef, PhantomData<fn(&'id ()) -> &'id ()>);
#[derive(Clone, Copy)]
// ABI-identical to the LLVMBasicBlockRef it wraps, including in slices.
#[repr(transparent)]
pub(super) struct Block<'id>(LLVMBasicBlockRef, PhantomData<fn(&'id ()) -> &'id ()>);

impl Session<'_> {
    /// A session for `target` with the default code options.
    pub(super) fn with<R>(
        arena: &Bump,
        target: Target,
        body: impl for<'id> FnOnce(&Session<'id>) -> R,
    ) -> R {
        Self::with_code(arena, target, CodeOptions::default(), body)
    }

    /// A session for `target` whose module and code generation follow
    /// `code`.
    pub(super) fn with_code<R>(
        arena: &Bump,
        target: Target,
        code: CodeOptions,
        body: impl for<'id> FnOnce(&Session<'id>) -> R,
    ) -> R {
        // The handler's state has a stable address until after the session
        // disposes its context, including on early returns and unwinding.
        let diagnostics = Diagnostics(RefCell::new(ArenaString::new_in(arena)));
        // SAFETY: Creates a new independently owned LLVM context.
        let context = unsafe { LLVMContextCreate() };
        // SAFETY: The callback and its stack-owned state outlive this context.
        unsafe {
            LLVMContextSetDiagnosticHandler(
                context,
                Some(record_diagnostic),
                (&raw const diagnostics).cast_mut().cast(),
            );
        }
        // SAFETY: The context is live and the module name is NUL terminated.
        let module = unsafe { LLVMModuleCreateWithNameInContext(c"bcc".as_ptr(), context) };
        // SAFETY: The context is live.
        let builder = unsafe { LLVMCreateBuilderInContext(context) };
        let session = Session {
            builder,
            module,
            context,
            arena,
            diagnostics: &diagnostics,
            code,
            _brand: PhantomData,
        };
        let triple = session.name(target.triple());
        // SAFETY: Module and terminated triple are live for the call; LLVM
        // copies it.
        unsafe {
            LLVMSetTarget(module, triple.as_ptr());
        }
        let layout = session.name(super::target::data_layout(target));
        // SAFETY: Module and terminated layout are live; LLVM copies it.
        unsafe {
            LLVMSetDataLayout(module, layout.as_ptr());
        }
        // Clang marks position-independent ELF code with its PIC level, and
        // an executable's with its PIE level too, which lets code generation
        // use direct references to symbols the executable defines. Linking
        // modules keeps the smallest PIC level and the largest PIE level,
        // so code of both levels links (with LTO too), as Clang's does.
        let (level, pie) = match session.elf_relocation(target) {
            | Some(Relocation::Pie(level)) => (Some(level), true),
            | Some(Relocation::Pic(level)) => (Some(level), false),
            | Some(Relocation::Static) | None => (None, false),
        };
        if let Some(level) = level {
            let level = u64::from(level as u8);
            session.module_flag(MODULE_FLAG_MIN, "PIC Level", level);
            if pie {
                session.module_flag(MODULE_FLAG_MAX, "PIE Level", level);
            }
        }
        body(&session)
    }
}

#[expect(
    clippy::unused_self,
    reason = "The receiver binds handle types to this session's invariant brand."
)]
impl<'id> Session<'id> {
    pub(super) fn name(&self, name: &str) -> &CStr {
        debug_assert!(!name.as_bytes().contains(&0), "IR names contain no NUL");
        let bytes = self
            .arena
            .alloc_slice_fill_iter(name.bytes().chain(std::iter::once(0)));
        // SAFETY: IR names and target strings contain no NUL by construction,
        // asserted above in debug builds; allocation appends exactly one NUL.
        unsafe { CStr::from_bytes_with_nul_unchecked(bytes) }
    }

    pub(super) fn ty(&self, ty: Type) -> Ty<'id> {
        let raw = match ty {
            // SAFETY: The session owns the live context.
            | Type::I1 => unsafe { LLVMInt1TypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::I8 => unsafe { LLVMInt8TypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::I16 => unsafe { LLVMInt16TypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::I32 => unsafe { LLVMInt32TypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::I64 => unsafe { LLVMInt64TypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::I128 => unsafe { LLVMInt128TypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::F32 => unsafe { LLVMFloatTypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::F64 => unsafe { LLVMDoubleTypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::F80 => unsafe { LLVMX86FP80TypeInContext(self.context) },
            // SAFETY: The session owns the live context.
            | Type::F128 => unsafe { LLVMFP128TypeInContext(self.context) },
            // SAFETY: Opaque pointers need only the live context and address
            // space.
            | Type::Ptr => unsafe { LLVMPointerTypeInContext(self.context, 0) },
        };
        Ty(raw, PhantomData)
    }

    pub(super) fn void(&self) -> Ty<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMVoidTypeInContext(self.context) };
        Ty(raw, PhantomData)
    }

    pub(super) fn array(&self, ty: Ty<'id>, size: u64) -> Ty<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMArrayType2(ty.0, size) };
        Ty(raw, PhantomData)
    }

    pub(super) fn vector(&self, ty: Ty<'id>, size: u32) -> Ty<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMVectorType(ty.0, size) };
        Ty(raw, PhantomData)
    }

    pub(super) fn structure(&self, types: &[Ty<'id>], packed: bool) -> Ty<'id> {
        // SAFETY: The type system ties handles to this live session. Ty is
        // ABI-identical to LLVMTypeRef; LLVM only reads the counted
        // array despite its *mut parameter.
        let raw = unsafe {
            LLVMStructTypeInContext(
                self.context,
                types.as_ptr().cast::<LLVMTypeRef>().cast_mut(),
                count(types.len()),
                i32::from(packed),
            )
        };
        Ty(raw, PhantomData)
    }

    pub(super) fn signature(&self, result: Ty<'id>, types: &[Ty<'id>], variadic: bool) -> Ty<'id> {
        // SAFETY: The type system ties handles to this live session. Ty is
        // ABI-identical to LLVMTypeRef; LLVM only reads the counted
        // array despite its *mut parameter.
        let raw = unsafe {
            LLVMFunctionType(
                result.0,
                types.as_ptr().cast::<LLVMTypeRef>().cast_mut(),
                count(types.len()),
                i32::from(variadic),
            )
        };
        Ty(raw, PhantomData)
    }

    pub(super) fn function(&self, name: &CStr, ty: Ty<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMAddFunction(self.module, name.as_ptr(), ty.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn global(&self, name: &CStr, ty: Ty<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMAddGlobal(self.module, ty.0, name.as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn block(&self, function: Val<'id>, name: &CStr) -> Block<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMAppendBasicBlockInContext(self.context, function.0, name.as_ptr()) };
        Block(raw, PhantomData)
    }

    pub(super) fn position(&self, block: Block<'id>) {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        unsafe {
            LLVMPositionBuilderAtEnd(self.builder, block.0);
        }
    }

    pub(super) fn param(&self, function: Val<'id>, index: u32) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMGetParam(function.0, index) };
        Val(raw, PhantomData)
    }

    pub(super) fn set_name(&self, value: Val<'id>, name: &str) {
        // SAFETY: The type system ties the value to this live session;
        // globals and scalar constants must keep
        // their identity.
        if !unsafe { llvm_sys::core::LLVMIsAConstant(value.0) }.is_null() {
            return;
        }
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        unsafe {
            LLVMSetValueName2(value.0, name.as_ptr().cast(), name.len());
        }
    }

    pub(super) fn alignment(&self, value: Val<'id>, align: u64) {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        unsafe {
            LLVMSetAlignment(
                value.0,
                count(usize::try_from(align).expect("alignment fits usize")),
            );
        }
    }

    pub(super) fn volatile(&self, value: Val<'id>, volatile: bool) {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        unsafe {
            LLVMSetVolatile(value.0, i32::from(volatile));
        }
    }

    pub(super) fn initializer(&self, global: Val<'id>, value: Val<'id>) {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        unsafe {
            LLVMSetInitializer(global.0, value.0);
        }
    }

    pub(super) fn constant_global(&self, global: Val<'id>, constant: bool) {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        unsafe {
            LLVMSetGlobalConstant(global.0, i32::from(constant));
        }
    }

    /// Makes a global thread-local with at least the specificity of
    /// `model`; LLVM may choose a more specific model from the relocation
    /// model and DSO locality.
    pub(super) fn thread_local(&self, global: Val<'id>, model: TlsModel) {
        let mode = match model {
            | TlsModel::GlobalDynamic => LLVMThreadLocalMode::LLVMGeneralDynamicTLSModel,
            | TlsModel::LocalDynamic => LLVMThreadLocalMode::LLVMLocalDynamicTLSModel,
            | TlsModel::InitialExec => LLVMThreadLocalMode::LLVMInitialExecTLSModel,
            | TlsModel::LocalExec => LLVMThreadLocalMode::LLVMLocalExecTLSModel,
        };
        // SAFETY: The type system ties the global to this live session.
        unsafe {
            LLVMSetThreadLocalMode(global.0, mode);
        }
    }

    pub(super) fn linkage(&self, value: Val<'id>, linkage: Linkage) {
        let linkage = match linkage {
            | Linkage::Internal => LLVMLinkage::LLVMInternalLinkage,
            | Linkage::Weak => LLVMLinkage::LLVMWeakAnyLinkage,
            | Linkage::ExternWeak => LLVMLinkage::LLVMExternalWeakLinkage,
            | Linkage::WeakOdr => LLVMLinkage::LLVMWeakODRLinkage,
            | Linkage::LinkOnceOdr => LLVMLinkage::LLVMLinkOnceODRLinkage,
            | Linkage::AvailableExternally => LLVMLinkage::LLVMAvailableExternallyLinkage,
            | Linkage::Appending => LLVMLinkage::LLVMAppendingLinkage,
        };
        // SAFETY: The type system ties handles to this live session; every
        // caller passes a function, global or alias.
        unsafe {
            LLVMSetLinkage(value.0, linkage);
        }
    }

    /// Marks a function, global or alias `dllimport` or `dllexport`, as
    /// Clang marks Windows DLL storage.
    pub(super) fn dll_storage(&self, value: Val<'id>, export: bool) {
        let class = if export {
            LLVMDLLStorageClass::LLVMDLLExportStorageClass
        } else {
            LLVMDLLStorageClass::LLVMDLLImportStorageClass
        };
        // SAFETY: The value is a function, global or alias of this session.
        unsafe {
            LLVMSetDLLStorageClass(value.0, class);
        }
    }

    /// Places the function or global `value` in an `any` comdat of its own
    /// name, as Clang places Microsoft `selectany` data and inline
    /// functions.
    pub(super) fn own_comdat(&self, value: Val<'id>, name: &CStr) {
        // SAFETY: The module belongs to this live session; the name is
        // NUL-terminated and LLVM copies it.
        let comdat = unsafe { llvm_sys::comdat::LLVMGetOrInsertComdat(self.module, name.as_ptr()) };
        // SAFETY: The comdat was just returned by this live module.
        unsafe {
            llvm_sys::comdat::LLVMSetComdatSelectionKind(
                comdat,
                llvm_sys::comdat::LLVMComdatSelectionKind::LLVMAnyComdatSelectionKind,
            );
        }
        // SAFETY: The type system ties the global to this live session, and
        // the comdat belongs to its module.
        unsafe {
            llvm_sys::comdat::LLVMSetComdat(value.0, comdat);
        }
    }

    pub(super) fn hidden(&self, value: Val<'id>, protected: bool) {
        let visibility = if protected {
            LLVMVisibility::LLVMProtectedVisibility
        } else {
            LLVMVisibility::LLVMHiddenVisibility
        };
        // SAFETY: The value is a function, global or alias of this session.
        unsafe {
            LLVMSetVisibility(value.0, visibility);
        }
    }

    pub(super) fn section(&self, value: Val<'id>, name: &str) {
        let name = self.name(name);
        // SAFETY: The value is a function or global of this live session;
        // LLVM copies the NUL-terminated section name.
        unsafe {
            LLVMSetSection(value.0, name.as_ptr());
        }
    }

    /// A global alias of `aliasee` whose value has type `ty`.
    pub(super) fn alias(&self, name: &CStr, ty: Ty<'id>, aliasee: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session; the
        // aliasee is a function or global constant in address space 0.
        let raw = unsafe { LLVMAddAlias2(self.module, ty.0, 0, aliasee.0, name.as_ptr()) };
        Val(raw, PhantomData)
    }

    /// A `thread_local` alias of `size` bytes named `name` for the
    /// thread-local global `aliasee`, as Clang emits for an alias of a
    /// thread-local object. LLVM-C can mark only a global variable
    /// thread-local, so the alias is parsed from textual IR in a module of
    /// its own (aliasing null), linked into this one, and then pointed at
    /// its aliasee. `model` is requested as `thread_local` requests it for
    /// a global.
    pub(super) fn thread_local_alias(
        &self,
        name: &str,
        size: u64,
        aliasee: Val<'id>,
        model: TlsModel,
    ) -> Val<'id> {
        let mode = match model {
            | TlsModel::GlobalDynamic => "",
            | TlsModel::LocalDynamic => "(localdynamic)",
            | TlsModel::InitialExec => "(initialexec)",
            | TlsModel::LocalExec => "(localexec)",
        };
        // Every byte as `\XX`, which a quoted LLVM name always accepts.
        let quoted = fmt::from_fn(|f| name.bytes().try_for_each(|byte| write!(f, "\\{byte:02X}")));
        let text = crate::diagnostics::format_in!(
            self.arena,
            "@\"{quoted}\" = thread_local{mode} alias [{size} x i8], ptr null\n\0"
        );
        // SAFETY: The buffer borrows `text` up to its final NUL, which the IR
        // lexer requires, and is disposed below while `text` lives.
        let buffer = unsafe {
            LLVMCreateMemoryBufferWithMemoryRange(
                text.as_ptr().cast(),
                text.len() - 1,
                c"bcc.thread_local_alias".as_ptr(),
                1,
            )
        };
        let mut source = ptr::null_mut();
        let mut message = ptr::null_mut();
        // SAFETY: The session owns the live context; the buffer is live, and
        // the parser does not take it.
        let failed = unsafe {
            LLVMParseIRInContext2(self.context, buffer, &raw mut source, &raw mut message)
        };
        // SAFETY: This disposes the buffer created above exactly once.
        unsafe {
            LLVMDisposeMemoryBuffer(buffer);
        }
        let message = Message(message);
        assert!(failed == 0, "thread-local alias IR parses: {message}");
        // SAFETY: The session's module is live; LLVM returns its layout.
        let layout = unsafe { LLVMGetDataLayoutStr(self.module) };
        // SAFETY: The parsed module is live, and LLVM copies the layout.
        unsafe {
            LLVMSetDataLayout(source, layout);
        }
        // SAFETY: The session's module is live; LLVM returns its triple.
        let triple = unsafe { LLVMGetTarget(self.module) };
        // SAFETY: The parsed module is live, and LLVM copies the triple. The
        // same layout and triple leave linking nothing to report.
        unsafe {
            LLVMSetTarget(source, triple);
        }
        // SAFETY: Both modules are live in this session's context; linking
        // destroys the parsed source module.
        let failed = unsafe { LLVMLinkModules2(self.module, source) };
        assert!(failed == 0, "a fresh thread-local alias links");
        let name = self.name(name);
        // SAFETY: The session's module is live and the name NUL-terminated.
        let raw =
            unsafe { LLVMGetNamedGlobalAlias(self.module, name.as_ptr(), name.count_bytes()) };
        assert!(!raw.is_null(), "the linked alias keeps its name");
        // SAFETY: The alias and its thread-local aliasee are globals of this
        // session's module.
        unsafe {
            LLVMAliasSetAliasee(raw, aliasee.0);
        }
        Val(raw, PhantomData)
    }

    /// A constant array of `element` values.
    pub(super) fn const_array(&self, element: Ty<'id>, values: &[Val<'id>]) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session. Val is
        // ABI-identical to LLVMValueRef; every value has type `element`.
        // LLVM only reads the counted array despite its *mut parameter.
        Val(
            unsafe {
                LLVMConstArray2(
                    element.0,
                    values.as_ptr().cast::<LLVMValueRef>().cast_mut(),
                    values.len() as u64,
                )
            },
            PhantomData,
        )
    }

    /// A constant structure with natural (unpacked) field layout.
    pub(super) fn const_record(&self, values: &[Val<'id>]) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session. Val is
        // ABI-identical to LLVMValueRef; LLVM only reads the counted array
        // despite its *mut parameter.
        Val(
            unsafe {
                LLVMConstStructInContext(
                    self.context,
                    values.as_ptr().cast::<LLVMValueRef>().cast_mut(),
                    count(values.len()),
                    0,
                )
            },
            PhantomData,
        )
    }

    pub(super) fn null(&self, ty: Ty<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMConstNull(ty.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn poison(&self, ty: Ty<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMGetPoison(ty.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn bits(&self, ty: Type, bits: u128) -> Val<'id> {
        let words = [
            u64::try_from(bits & u128::from(u64::MAX)).unwrap(),
            u64::try_from(bits >> 64).unwrap(),
        ];
        let int_ty = if ty.is_float() {
            match ty {
                | Type::F32 => self.ty(Type::I32),
                | Type::F64 => self.ty(Type::I64),
                | _ => {
                    // SAFETY: The context is live and floating bit widths are
                    // valid integer widths.
                    Ty(
                        unsafe { LLVMIntTypeInContext(self.context, ty.bits()) },
                        PhantomData,
                    )
                },
            }
        } else {
            self.ty(ty)
        };
        // SAFETY: The integer type is live and words provide all bits for its
        // width.
        let value = unsafe {
            LLVMConstIntOfArbitraryPrecision(
                int_ty.0,
                if ty.bits() > 64 { 2 } else { 1 },
                words.as_ptr(),
            )
        };
        if ty.is_float() {
            // SAFETY: Integer and floating types have identical bit widths.
            Val(
                unsafe { LLVMConstBitCast(value, self.ty(ty).0) },
                PhantomData,
            )
        } else {
            Val(value, PhantomData)
        }
    }

    pub(super) fn bytes(&self, bytes: &[u8]) -> Val<'id> {
        // SAFETY: LLVM copies the counted bytes, with no implicit terminator.
        Val(
            unsafe {
                LLVMConstStringInContext2(self.context, bytes.as_ptr().cast(), bytes.len(), 1)
            },
            PhantomData,
        )
    }

    pub(super) fn const_structure(&self, values: &[Val<'id>]) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session. Val is
        // ABI-identical to LLVMValueRef; constants have the verified IR
        // types. LLVM only reads the counted array despite its *mut
        // parameter.
        Val(
            unsafe {
                LLVMConstStructInContext(
                    self.context,
                    values.as_ptr().cast::<LLVMValueRef>().cast_mut(),
                    count(values.len()),
                    1,
                )
            },
            PhantomData,
        )
    }

    pub(super) fn const_gep(&self, base: Val<'id>, offset: i64) -> Val<'id> {
        let index = self.bits(Type::I64, offset as u128);
        // SAFETY: The type system ties handles to this live session. A single
        // byte offset indexes the opaque pointer constant. Val is
        // ABI-identical to LLVMValueRef; LLVM only reads the index
        // array despite its *mut parameter.
        Val(
            unsafe {
                LLVMConstGEP2(
                    self.ty(Type::I8).0,
                    base.0,
                    ptr::from_ref(&index).cast::<LLVMValueRef>().cast_mut(),
                    1,
                )
            },
            PhantomData,
        )
    }

    /// The address of `block`, a block of `function` other than its entry.
    pub(super) fn block_address(&self, function: Val<'id>, block: Block<'id>) -> Val<'id> {
        // SAFETY: The type system ties both handles to this live session; the
        // verifier keeps labels off the entry block, as LLVM requires.
        Val(
            unsafe { LLVMBlockAddress(function.0, block.0) },
            PhantomData,
        )
    }

    /// `label - base + addend` truncated to `width`, for two block addresses
    /// of one function: the constant expression Clang emits for a GNU label
    /// difference in a static initializer.
    pub(super) fn label_difference(
        &self,
        label: Val<'id>,
        base: Val<'id>,
        addend: i64,
        width: Type,
    ) -> Val<'id> {
        let i64_ty = self.ty(Type::I64);
        // SAFETY: Both operands are pointer constants of this session and
        // i64 is a live integer type; LLVM folds or retains the expression.
        let label = unsafe { LLVMConstPtrToInt(label.0, i64_ty.0) };
        // SAFETY: As for the label.
        let base = unsafe { LLVMConstPtrToInt(base.0, i64_ty.0) };
        // SAFETY: Both operands are i64 constants of this session.
        let difference = unsafe { LLVMConstSub(label, base) };
        let difference = if addend == 0 {
            difference
        } else {
            // SAFETY: Both operands are i64 constants of this session.
            unsafe { LLVMConstAdd(difference, self.bits(Type::I64, addend as u128).0) }
        };
        if width == Type::I64 {
            return Val(difference, PhantomData);
        }
        // SAFETY: The verifier restricts the width to a narrower integer.
        Val(
            unsafe { LLVMConstTrunc(difference, self.ty(width).0) },
            PhantomData,
        )
    }

    pub(super) fn alloca(&self, ty: Ty<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildAlloca(self.builder, ty.0, c"".as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn array_alloca(&self, ty: Ty<'id>, size: Val<'id>) -> Val<'id> {
        // SAFETY: Both handles belong to this session; size has integer type.
        Val(
            unsafe { LLVMBuildArrayAlloca(self.builder, ty.0, size.0, c"".as_ptr()) },
            PhantomData,
        )
    }

    pub(super) fn extract_value(&self, aggregate: Val<'id>, index: u32) -> Val<'id> {
        // SAFETY: Callers supply an in-range index of a verified structure.
        Val(
            unsafe { LLVMBuildExtractValue(self.builder, aggregate.0, index, c"".as_ptr()) },
            PhantomData,
        )
    }

    pub(super) fn phi(&self, ty: Ty<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildPhi(self.builder, ty.0, c"".as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn binary(&self, opcode: LLVMOpcode, a: Val<'id>, b: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = self.unfolded(a.0, |operand| unsafe {
            LLVMBuildBinOp(self.builder, opcode, operand, b.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn cast(&self, opcode: LLVMOpcode, value: Val<'id>, ty: Ty<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = self.unfolded(value.0, |operand| unsafe {
            LLVMBuildCast(self.builder, opcode, operand, ty.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn fneg(&self, value: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = self.unfolded(value.0, |operand| unsafe {
            LLVMBuildFNeg(self.builder, operand, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn freeze(&self, value: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = self.unfolded(value.0, |operand| unsafe {
            LLVMBuildFreeze(self.builder, operand, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn va_arg(&self, value: Val<'id>, ty: Ty<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildVAArg(self.builder, value.0, ty.0, c"".as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn icmp(&self, cond: LLVMIntPredicate, a: Val<'id>, b: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = self.unfolded(a.0, |operand| unsafe {
            LLVMBuildICmp(self.builder, cond, operand, b.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn fcmp(&self, cond: LLVMRealPredicate, a: Val<'id>, b: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = self.unfolded(a.0, |operand| unsafe {
            LLVMBuildFCmp(self.builder, cond, operand, b.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn select(&self, cond: Val<'id>, a: Val<'id>, b: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = self.unfolded(cond.0, |operand| unsafe {
            LLVMBuildSelect(self.builder, operand, a.0, b.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    /// Calls the libatomic generic ABI for arbitrary object widths.
    pub(super) fn atomic_bytes(
        &self,
        op: crate::ir::AtomicOp,
        args: &[Val<'id>],
        order: crate::ir::MemoryOrder,
        failure: crate::ir::MemoryOrder,
    ) -> Val<'id> {
        use crate::ir::{
            AtomicOp as A,
            MemoryOrder as M,
        };
        let c_order = |order| {
            self.bits(
                Type::I32,
                match order {
                    | M::Relaxed => 0,
                    | M::Acquire => 2,
                    | M::Release => 3,
                    | M::AcqRel => 4,
                    | M::SeqCst => 5,
                },
            )
        };
        let ptr = self.ty(Type::Ptr);
        let int = self.ty(Type::I32);
        let size = self.ty(Type::I64);
        let (name, result, params, values): (&CStr, _, &[_], &[_]) = match op {
            | A::LoadBytes => (
                c"__atomic_load",
                self.void(),
                &[size, ptr, ptr, int],
                &[args[2], args[0], args[1], c_order(order)],
            ),
            | A::StoreBytes => (
                c"__atomic_store",
                self.void(),
                &[size, ptr, ptr, int],
                &[args[2], args[0], args[1], c_order(order)],
            ),
            | A::ExchangeBytes => (
                c"__atomic_exchange",
                self.void(),
                &[size, ptr, ptr, ptr, int],
                &[args[3], args[0], args[1], args[2], c_order(order)],
            ),
            | A::CompareExchangeBytes => (
                c"__atomic_compare_exchange",
                self.ty(Type::I1),
                &[size, ptr, ptr, ptr, int, int],
                &[
                    args[3],
                    args[0],
                    args[1],
                    args[2],
                    c_order(order),
                    c_order(failure),
                ],
            ),
            | _ => unreachable!("byte atomic operation"),
        };
        let signature = self.signature(result, params, false);
        // SAFETY: The reserved runtime symbol has the libatomic ABI specified
        // above; all values have the verified scalar/pointer types.
        let raw = unsafe { LLVMGetNamedFunction(self.module, name.as_ptr()) };
        let function = if raw.is_null() {
            self.function(name, signature)
        } else {
            Val(raw, PhantomData)
        };
        let value = self.call(signature, function, values);
        if op == A::CompareExchangeBytes {
            self.cast(LLVMOpcode::LLVMZExt, value, self.ty(Type::I8))
        } else {
            value
        }
    }

    /// Constructs real LLVM atomic operations; relaxed fences are no-ops.
    pub(super) fn atomic(
        &self,
        op: crate::ir::AtomicOp,
        ty: Type,
        args: &[Val<'id>],
        order: crate::ir::MemoryOrder,
        failure: crate::ir::MemoryOrder,
    ) -> Option<Val<'id>> {
        use llvm_sys::{
            LLVMAtomicOrdering as O,
            LLVMAtomicRMWBinOp as R,
        };

        use crate::ir::{
            AtomicOp as A,
            MemoryOrder as M,
        };
        let ordering = |order| match order {
            | M::Relaxed => O::LLVMAtomicOrderingMonotonic,
            | M::Acquire => O::LLVMAtomicOrderingAcquire,
            | M::Release => O::LLVMAtomicOrderingRelease,
            | M::AcqRel => O::LLVMAtomicOrderingAcquireRelease,
            | M::SeqCst => O::LLVMAtomicOrderingSequentiallyConsistent,
        };
        let value = if op.is_fence() {
            if order == M::Relaxed {
                return None;
            }
            // SAFETY: The verifier checks this fence ordering; the builder is
            // live.
            unsafe {
                LLVMBuildFence(
                    self.builder,
                    ordering(order),
                    i32::from(op == A::SignalFence),
                    c"".as_ptr(),
                )
            }
        } else if op == A::Load || op == A::Store {
            let value = if op == A::Load {
                self.load(self.ty(ty), args[0])
            } else {
                self.store(args[1], args[0])
            };
            // SAFETY: This is a load/store instruction with verified atomic
            // ordering.
            unsafe {
                LLVMSetOrdering(value.0, ordering(order));
            }
            return Some(value);
        } else if op == A::CompareExchange {
            // SAFETY: Session brands own the handles. The verifier checks
            // widths, operand counts and success/failure order
            // constraints.
            unsafe {
                LLVMBuildAtomicCmpXchg(
                    self.builder,
                    args[0].0,
                    args[2].0,
                    args[1].0,
                    ordering(order),
                    ordering(failure),
                    0,
                )
            }
        } else {
            let operation = match op {
                | A::Exchange => R::LLVMAtomicRMWBinOpXchg,
                | A::Add => R::LLVMAtomicRMWBinOpAdd,
                | A::Sub => R::LLVMAtomicRMWBinOpSub,
                | A::And => R::LLVMAtomicRMWBinOpAnd,
                | A::Or => R::LLVMAtomicRMWBinOpOr,
                | A::Xor => R::LLVMAtomicRMWBinOpXor,
                | A::Nand => R::LLVMAtomicRMWBinOpNand,
                | A::Min => R::LLVMAtomicRMWBinOpMin,
                | A::Max => R::LLVMAtomicRMWBinOpMax,
                | A::Umin => R::LLVMAtomicRMWBinOpUMin,
                | A::Umax => R::LLVMAtomicRMWBinOpUMax,
                | A::Fadd => R::LLVMAtomicRMWBinOpFAdd,
                | A::Fsub => R::LLVMAtomicRMWBinOpFSub,
                | A::Fmin => R::LLVMAtomicRMWBinOpFMin,
                | A::Fmax => R::LLVMAtomicRMWBinOpFMax,
                | _ => unreachable!("handled atomic opcode"),
            };
            // SAFETY: Session brands own the handles. The verifier checks the
            // operation's value type, width, operands and memory order.
            unsafe {
                LLVMBuildAtomicRMW(
                    self.builder,
                    operation,
                    args[0].0,
                    args[1].0,
                    ordering(order),
                    0,
                )
            }
        };
        Some(Val(value, PhantomData))
    }

    pub(super) fn load(&self, ty: Ty<'id>, addr: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildLoad2(self.builder, ty.0, addr.0, c"".as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn store(&self, value: Val<'id>, addr: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildStore(self.builder, value.0, addr.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn insert_value(
        &self,
        aggregate: Val<'id>,
        value: Val<'id>,
        index: u32,
    ) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session. The field
        // index and type agree with the verified register-return structure.
        let raw = unsafe {
            LLVMBuildInsertValue(self.builder, aggregate.0, value.0, index, c"".as_ptr())
        };
        Val(raw, PhantomData)
    }

    pub(super) fn jump(&self, block: Block<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildBr(self.builder, block.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn branch(&self, cond: Val<'id>, then: Block<'id>, other: Block<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildCondBr(self.builder, cond.0, then.0, other.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn switch(&self, value: Val<'id>, default: Block<'id>, cases: u32) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildSwitch(self.builder, value.0, default.0, cases) };
        Val(raw, PhantomData)
    }

    /// An `indirectbr` to `addr` with room for `dests` destinations, which
    /// [`Session::add_destination`] lists.
    pub(super) fn indirect_br(&self, addr: Val<'id>, dests: u32) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session; the
        // address has pointer type.
        let raw = unsafe { LLVMBuildIndirectBr(self.builder, addr.0, dests) };
        Val(raw, PhantomData)
    }

    pub(super) fn add_destination(&self, indirect: Val<'id>, block: Block<'id>) {
        // SAFETY: The type system ties handles to this live session; the
        // value is an indirectbr built by this session.
        unsafe {
            LLVMAddDestination(indirect.0, block.0);
        }
    }

    pub(super) fn ret(&self, value: Val<'id>) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildRet(self.builder, value.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn ret_void(&self) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildRetVoid(self.builder) };
        Val(raw, PhantomData)
    }

    pub(super) fn unreachable(&self) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        let raw = unsafe { LLVMBuildUnreachable(self.builder) };
        Val(raw, PhantomData)
    }

    pub(super) fn add_case(&self, switch: Val<'id>, value: Val<'id>, block: Block<'id>) {
        // SAFETY: The type system ties handles to this live session;
        // operands have the verified IR types.
        unsafe {
            LLVMAddCase(switch.0, value.0, block.0);
        }
    }

    /// Prevent LLVM's builder folder from consuming flags before attachment.
    /// The temporary phi is removed before any verification or pass runs.
    fn unfolded(
        &self,
        input: LLVMValueRef,
        build: impl FnOnce(LLVMValueRef) -> LLVMValueRef,
    ) -> LLVMValueRef {
        // SAFETY: Callers supply a branded value from this context with a
        // first-class type; ownership is enforced by the type system.
        let ty = unsafe { LLVMTypeOf(input) };
        // SAFETY: A live insertion block and first-class type; this temporary
        // phi is never verified.
        let placeholder = unsafe { LLVMBuildPhi(self.builder, ty, c"".as_ptr()) };
        let result = build(placeholder);
        if result == placeholder {
            // SAFETY: An identity cast returned the unused temporary phi.
            unsafe {
                LLVMInstructionEraseFromParent(placeholder);
            }
            return input;
        }
        // SAFETY: The builder made an instruction with placeholder as operand
        // zero; input has the same type.
        unsafe {
            LLVMSetOperand(result, 0, input);
        }
        // SAFETY: The only placeholder use was replaced; erase it before
        // verifying the completed module.
        unsafe {
            LLVMInstructionEraseFromParent(placeholder);
        }
        result
    }

    pub(super) fn incoming(&self, phi: Val<'id>, value: Val<'id>, block: Block<'id>) {
        // SAFETY: The type system ties handles to this live session. The phi
        // and incoming value have matching types. Val/Block are
        // ABI-identical to LLVMValueRef/LLVMBasicBlockRef;
        // LLVM only reads both one-element arrays despite *mut parameters.
        unsafe {
            LLVMAddIncoming(
                phi.0,
                ptr::from_ref(&value).cast::<LLVMValueRef>().cast_mut(),
                ptr::from_ref(&block).cast::<LLVMBasicBlockRef>().cast_mut(),
                1,
            );
        }
    }

    pub(super) fn gep(&self, base: Val<'id>, offset: Val<'id>, inbounds: bool) -> Val<'id> {
        let index = ptr::from_ref(&offset).cast::<LLVMValueRef>().cast_mut();
        let ty = self.ty(Type::I8).0;
        let raw = if inbounds {
            // SAFETY: The type system ties handles to this live session. One
            // integer index addresses bytes at an opaque pointer.
            // Val is ABI-identical to LLVMValueRef; LLVM only reads
            // the index array despite its *mut parameter.
            self.unfolded(base.0, |operand| unsafe {
                LLVMBuildInBoundsGEP2(self.builder, ty, operand, index, 1, c"".as_ptr())
            })
        } else {
            // SAFETY: The type system ties handles to this live session. One
            // integer index addresses bytes at an opaque pointer.
            // Val is ABI-identical to LLVMValueRef; LLVM only reads
            // the index array despite its *mut parameter.
            self.unfolded(base.0, |operand| unsafe {
                LLVMBuildGEP2(self.builder, ty, operand, index, 1, c"".as_ptr())
            })
        };
        Val(raw, PhantomData)
    }

    pub(super) fn flags(&self, value: Val<'id>, flags: InstFlags) {
        if flags.contains(InstFlags::NUW) {
            // SAFETY: The type system ties the value to this live session.
            // Verified IR allows NUW only on
            // overflowing arithmetic instructions.
            unsafe {
                LLVMSetNUW(value.0, 1);
            }
        }
        if flags.contains(InstFlags::NSW) {
            // SAFETY: The type system ties the value to this live session.
            // Verified IR allows NSW only on
            // overflowing arithmetic instructions.
            unsafe {
                LLVMSetNSW(value.0, 1);
            }
        }
        if flags.contains(InstFlags::EXACT) {
            // SAFETY: The type system ties the value to this live session.
            // Verified IR allows EXACT only on
            // divisions and right shifts.
            unsafe {
                LLVMSetExact(value.0, 1);
            }
        }
    }

    pub(super) fn call(&self, ty: Ty<'id>, function: Val<'id>, args: &[Val<'id>]) -> Val<'id> {
        // SAFETY: The type system ties handles to this live session. The
        // callee's ABI type matches the counted arguments. Val is
        // ABI-identical to LLVMValueRef; LLVM only reads the
        // array despite its *mut parameter.
        Val(
            unsafe {
                LLVMBuildCall2(
                    self.builder,
                    ty.0,
                    function.0,
                    args.as_ptr().cast::<LLVMValueRef>().cast_mut(),
                    count(args.len()),
                    c"".as_ptr(),
                )
            },
            PhantomData,
        )
    }

    pub(super) fn intrinsic(&self, name: &str, overloads: &[Ty<'id>]) -> (Val<'id>, Ty<'id>) {
        // SAFETY: The intrinsic lookup reads the counted name.
        let id = unsafe { LLVMLookupIntrinsicID(name.as_ptr().cast(), name.len()) };
        let types = overloads.as_ptr().cast::<LLVMTypeRef>().cast_mut();
        // SAFETY: Known intrinsic ID and correct overload types; handles belong
        // to this live session by the type system.
        // Ty is ABI-identical to LLVMTypeRef; LLVM only reads the counted array
        // despite its *mut parameter.
        let function =
            unsafe { LLVMGetIntrinsicDeclaration(self.module, id, types, overloads.len()) };
        // SAFETY: Same live-session invariant and ABI-identical overload list
        // as above, enforced by the type system; LLVM only reads the array
        // despite its *mut parameter.
        let ty = unsafe { LLVMIntrinsicGetType(self.context, id, types, overloads.len()) };
        (Val(function, PhantomData), Ty(ty, PhantomData))
    }

    pub(super) fn attribute(
        &self,
        value: Val<'id>,
        index: u32,
        name: &str,
        number: u64,
        ty: Option<Ty<'id>>,
        call: bool,
    ) {
        // SAFETY: The counted attribute name is valid during lookup.
        let kind = unsafe { LLVMGetEnumAttributeKindForName(name.as_ptr().cast(), name.len()) };
        let attribute = if let Some(ty) = ty {
            // SAFETY: The attribute kind is byval/sret and the type belongs to
            // this context by the type system.
            unsafe { LLVMCreateTypeAttribute(self.context, kind, ty.0) }
        } else {
            // SAFETY: Known enum attribute with its appropriate integer
            // payload.
            unsafe { LLVMCreateEnumAttribute(self.context, kind, number) }
        };
        if call {
            // SAFETY: A call instruction in this live session by the type
            // system, with a valid ABI argument/result index.
            unsafe {
                LLVMAddCallSiteAttribute(value.0, index, attribute);
            }
        } else {
            // SAFETY: The type system ties the function to this live session,
            // with a valid ABI parameter/result index.
            unsafe {
                LLVMAddAttributeAtIndex(value.0, index, attribute);
            }
        }
    }

    /// Adds the string attribute `key`=`value` to the function `function`.
    pub(super) fn string_attribute(&self, function: Val<'id>, key: &str, value: &str) {
        let key_length = u32::try_from(key.len()).expect("attribute key fits in u32");
        let value_length = u32::try_from(value.len()).expect("attribute value fits in u32");
        // SAFETY: The counted key and value are valid while LLVM copies them
        // into the context.
        let attribute = unsafe {
            LLVMCreateStringAttribute(
                self.context,
                key.as_ptr().cast(),
                key_length,
                value.as_ptr().cast(),
                value_length,
            )
        };
        // SAFETY: The type system ties the function to this live session;
        // the function index addresses its function attributes.
        unsafe {
            LLVMAddAttributeAtIndex(function.0, u32::MAX, attribute);
        }
    }

    pub(super) fn call_diagnostic(&self, call: Val<'id>, id: u32) {
        call_diagnostics::attach(self, call, id);
    }

    pub(super) fn call_diagnostic_counts<'a>(&self, count: usize, arena: &'a Bump) -> &'a [usize] {
        call_diagnostics::counts(self, count, arena)
    }

    pub(super) fn verify(&self) -> Result<(), Message> {
        let mut error = ptr::null_mut();
        // SAFETY: A live module; verifier returns a separately owned message.
        let failed = unsafe {
            LLVMVerifyModule(
                self.module,
                LLVMVerifierFailureAction::LLVMReturnStatusAction,
                &raw mut error,
            )
        };
        let message = Message(error);
        self.diagnostics.check()?;
        if failed != 0 { Err(message) } else { Ok(()) }
    }

    pub(super) fn print(&self) -> PrintedModule {
        // SAFETY: A live module; the returned message must be disposed.
        PrintedModule(Message(unsafe { LLVMPrintModuleToString(self.module) }))
    }

    fn prepare_emission(
        &self,
        target: Target,
        opt: Option<OptLevel>,
        skip_passes: bool,
    ) -> Result<Machine, Message> {
        static INITIALIZE: Once = Once::new();
        INITIALIZE.call_once(|| {
            // SAFETY: Process-global registrations execute exactly once before
            // target lookup.
            unsafe {
                LLVMInitializeX86TargetInfo();
            }
            // SAFETY: Serialized registration of the compiled-in X86 target.
            unsafe {
                LLVMInitializeX86Target();
            }
            // SAFETY: Serialized registration of X86 machine-code support.
            unsafe {
                LLVMInitializeX86TargetMC();
            }
            // SAFETY: Serialized registration of X86 assembly emission.
            unsafe {
                LLVMInitializeX86AsmPrinter();
            }
            // SAFETY: Serialized registration of X86 assembly parsing.
            unsafe {
                LLVMInitializeX86AsmParser();
            }
        });
        let triple = self.name(target.triple());
        let mut llvm_target = ptr::null_mut();
        let mut error = ptr::null_mut();
        // SAFETY: The registered target lookup writes the target and owned
        // error message.
        if unsafe { LLVMGetTargetFromTriple(triple.as_ptr(), &raw mut llvm_target, &raw mut error) }
            != 0
        {
            return Err(Message(error));
        }
        // LLVM 23 expresses size optimization with function attributes and
        // the O2 pipeline; default<Os>/default<Oz> are rejected.
        if matches!(opt, Some(OptLevel::Os | OptLevel::Oz)) {
            // SAFETY: This session owns the live module and its function list.
            let mut function = unsafe { LLVMGetFirstFunction(self.module) };
            while !function.is_null() {
                // SAFETY: A live function from this module's function list.
                if unsafe { LLVMIsDeclaration(function) } == 0 {
                    let value = Val(function, PhantomData);
                    self.attribute(value, u32::MAX, "optsize", 0, None, false);
                    if opt == Some(OptLevel::Oz) {
                        self.attribute(value, u32::MAX, "minsize", 0, None, false);
                    }
                }
                // SAFETY: A live function; attributes do not alter the list.
                function = unsafe { LLVMGetNextFunction(function) };
            }
        }
        let level = match opt.unwrap_or(OptLevel::O0) {
            | OptLevel::O0 => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            | OptLevel::O1 => LLVMCodeGenOptLevel::LLVMCodeGenLevelLess,
            | OptLevel::O2 | OptLevel::Os | OptLevel::Oz =>
                LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
            | OptLevel::O3 => LLVMCodeGenOptLevel::LLVMCodeGenLevelAggressive,
        };
        // SAFETY: Known target, terminated triple/CPU/features, valid machine
        // options.
        let machine = Machine(unsafe {
            LLVMCreateTargetMachine(
                llvm_target,
                triple.as_ptr(),
                c"x86-64".as_ptr(),
                c"".as_ptr(),
                level,
                match self.elf_relocation(target) {
                    | Some(Relocation::Pie(_) | Relocation::Pic(_)) => LLVMRelocMode::LLVMRelocPIC,
                    | Some(Relocation::Static) => LLVMRelocMode::LLVMRelocStatic,
                    | None => LLVMRelocMode::LLVMRelocDefault,
                },
                LLVMCodeModel::LLVMCodeModelDefault,
            )
        });
        if machine.0.is_null() {
            // SAFETY: Static terminated diagnostic copied into LLVM-owned
            // storage.
            return Err(Message(unsafe {
                LLVMCreateMessage(c"Cannot create LLVM target machine".as_ptr())
            }));
        }
        if matches!(target, Target::LinuxGnu | Target::LinuxMusl) {
            // SAFETY: The machine is live and exclusively owned here, before
            // emission initializes its ELF sections. LLVM-C omits this option.
            unsafe {
                bcc_llvm_use_init_array(machine.0);
            }
        }
        if !skip_passes {
            let pipeline = match opt.unwrap_or(OptLevel::O0) {
                | OptLevel::O0 => c"default<O0>",
                | OptLevel::O1 => c"default<O1>",
                | OptLevel::O2 | OptLevel::Os | OptLevel::Oz => c"default<O2>",
                | OptLevel::O3 => c"default<O3>",
            };
            // SAFETY: Creates independently owned pass-builder options.
            let options = PassOptions(unsafe { LLVMCreatePassBuilderOptions() });
            // SAFETY: Live module, target machine and pass options, with a
            // known pipeline.
            let result =
                unsafe { LLVMRunPasses(self.module, pipeline.as_ptr(), machine.0, options.0) };
            if !result.is_null() {
                // SAFETY: Consumes the LLVMErrorRef and returns a message with
                // error-specific disposal.
                let error = unsafe { LLVMGetErrorMessage(result) };
                let message = Message::copy_error(error);
                return Err(message);
            }
        }
        self.verify()?;
        Ok(machine)
    }

    #[cfg(test)]
    pub(super) fn emit(
        &self,
        target: Target,
        opt: Option<OptLevel>,
        skip_passes: bool,
        assembly: bool,
    ) -> Result<Buffer, Message> {
        self.emit_checked(target, opt, skip_passes, assembly, |_| true)
            .map(|buffer| buffer.expect("unconditional emission produces a buffer"))
    }

    pub(super) fn emit_checked(
        &self,
        target: Target,
        opt: Option<OptLevel>,
        skip_passes: bool,
        assembly: bool,
        check: impl FnOnce(&Self) -> bool,
    ) -> Result<Option<Buffer>, Message> {
        let machine = self.prepare_emission(target, opt, skip_passes)?;
        if !check(self) {
            return Ok(None);
        }
        let mut error = ptr::null_mut();
        let mut buffer = ptr::null_mut();
        let kind = if assembly {
            LLVMCodeGenFileType::LLVMAssemblyFile
        } else {
            LLVMCodeGenFileType::LLVMObjectFile
        };
        // SAFETY: Verified live module and machine; outputs are separately
        // owned.
        let failed = unsafe {
            LLVMTargetMachineEmitToMemoryBuffer(
                machine.0,
                self.module,
                kind,
                &raw mut error,
                &raw mut buffer,
            )
        };
        let message = Message(error);
        let buffer = (!buffer.is_null()).then(|| Buffer(buffer));
        // MC can report errors through the context handler while the C API
        // returns success and a corrupt object. Drop that buffer on any error.
        self.diagnostics.check()?;
        if failed != 0 {
            return Err(message);
        }
        Ok(buffer)
    }
}

fn count(length: usize) -> u32 {
    u32::try_from(length).expect("LLVM list fits u32")
}

/// All LLVM errors in one context, copied before their diagnostic handles
/// expire. The caller prints the complete set before failing compilation.
struct Diagnostics<'a>(RefCell<ArenaString<'a>>);

trait DiagnosticStatus {
    fn check(&self) -> Result<(), Message>;
}

impl DiagnosticStatus for Diagnostics<'_> {
    fn check(&self) -> Result<(), Message> {
        let mut errors = self.0.borrow_mut();
        if errors.is_empty() {
            return Ok(());
        }
        errors.push('\0');
        // SAFETY: The collected descriptions contain no interior NUL and the
        // appended terminator stays live while LLVM copies the text.
        let message = Message(unsafe { LLVMCreateMessage(errors.as_ptr().cast()) });
        errors.clear();
        Err(message)
    }
}

extern "C" fn record_diagnostic(info: LLVMDiagnosticInfoRef, state: *mut c_void) {
    // SAFETY: LLVM supplies a live diagnostic for this callback invocation.
    let severity = unsafe { LLVMGetDiagInfoSeverity(info) };
    // The C callback receives optimization remarks even when the default
    // handler would filter them. The CLI does not enable these remarks.
    if severity == LLVMDiagnosticSeverity::LLVMDSRemark {
        return;
    }
    // SAFETY: The description is copied to independently owned LLVM storage.
    let message = Message(unsafe { LLVMGetDiagInfoDescription(info) });
    // SAFETY: with_code keeps this state alive until the context is disposed;
    // this context's synchronous compilation owns all accesses to it.
    let diagnostics = unsafe { &*state.cast::<Diagnostics<'_>>() };
    if severity == LLVMDiagnosticSeverity::LLVMDSError {
        let mut errors = diagnostics.0.borrow_mut();
        if !errors.is_empty() {
            errors.push_str("\nerror: ");
        }
        errors.push_str(message.text());
    } else {
        let prefix = if severity == LLVMDiagnosticSeverity::LLVMDSWarning {
            "warning"
        } else {
            "note"
        };
        drop(std::io::Write::write_fmt(
            &mut std::io::stderr().lock(),
            format_args!("{prefix}: {message}\n"),
        ));
    }
}

/// LLVM-owned IR text, kept distinct from external diagnostics.
#[derive(Debug)]
pub(super) struct PrintedModule(Message);

impl PrintedModule {
    pub(super) fn text(&self) -> &str {
        // SAFETY: LLVMPrintModuleToString returns an owned NUL-terminated
        // string; the nested Message keeps it live until Drop.
        let bytes = unsafe { CStr::from_ptr(self.0.0) }.to_bytes();
        debug_assert!(std::str::from_utf8(bytes).is_ok(), "LLVM IR is UTF-8");
        // SAFETY: Our module contains only IR names from &str and generated
        // target strings, with no user file paths or external diagnostics.
        // LLVM's IR printer escapes arbitrary constant/name bytes into ASCII,
        // so its output is valid UTF-8, asserted above in debug builds.
        unsafe { std::str::from_utf8_unchecked(bytes) }
    }
}

impl fmt::Display for PrintedModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}

/// An LLVM-owned textual result/error with `LLVMDisposeMessage` ownership.
#[derive(Debug)]
pub(crate) struct Message(*mut c_char);
impl Message {
    pub(super) fn text(&self) -> &str {
        if self.0.is_null() {
            return "";
        }
        // SAFETY: Owned NUL-terminated LLVM message remains live until Drop.
        unsafe { CStr::from_ptr(self.0) }
            .to_str()
            .unwrap_or("LLVM returned a non-UTF-8 diagnostic")
    }

    fn copy_error(error: *mut c_char) -> Self {
        // SAFETY: Live error message; LLVMCreateMessage copies it to regular
        // message ownership.
        let result = Self(unsafe { LLVMCreateMessage(error) });
        // SAFETY: LLVMGetErrorMessage transferred unique ownership to us.
        unsafe {
            LLVMDisposeErrorMessage(error);
        }
        result
    }
}
impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}
impl std::error::Error for Message {}
impl Drop for Message {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This owner disposes its message exactly once.
            unsafe {
                LLVMDisposeMessage(self.0);
            }
        }
    }
}

/// Object/assembly bytes owned by LLVM.
#[derive(Debug)]
pub(super) struct Buffer(LLVMMemoryBufferRef);
impl Buffer {
    pub(super) fn bytes(&self) -> &[u8] {
        // SAFETY: This buffer is live and immutable.
        let start = unsafe { LLVMGetBufferStart(self.0) };
        // SAFETY: This buffer is live and immutable.
        let size = unsafe { LLVMGetBufferSize(self.0) };
        // SAFETY: LLVM guarantees the buffer's storage covers its size until
        // disposal.
        unsafe { std::slice::from_raw_parts(start.cast(), size) }
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: This owner disposes its buffer exactly once.
        unsafe {
            LLVMDisposeMemoryBuffer(self.0);
        }
    }
}
#[derive(Debug)]
struct Machine(LLVMTargetMachineRef);

unsafe extern "C" {
    /// Selects ELF initialization arrays on a live, unused target machine.
    fn bcc_llvm_use_init_array(machine: LLVMTargetMachineRef);
}
impl Drop for Machine {
    fn drop(&mut self) {
        // SAFETY: This owner disposes its target machine exactly once.
        unsafe {
            LLVMDisposeTargetMachine(self.0);
        }
    }
}
#[derive(Debug)]
struct PassOptions(LLVMPassBuilderOptionsRef);
impl Drop for PassOptions {
    fn drop(&mut self) {
        // SAFETY: This owner disposes its options exactly once.
        unsafe {
            LLVMDisposePassBuilderOptions(self.0);
        }
    }
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        // SAFETY: Dispose the builder before its module and context.
        unsafe {
            LLVMDisposeBuilder(self.builder);
        }
        // SAFETY: Handles cannot outlive the session; dispose module before
        // context.
        unsafe {
            LLVMDisposeModule(self.module);
        }
        // SAFETY: All objects in this uniquely owned context have now been
        // disposed.
        unsafe {
            LLVMContextDispose(self.context);
        }
    }
}

impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.text() == other.text()
    }
}
impl Eq for Message {}

impl fmt::Debug for Session<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("module", &self.module)
            .finish_non_exhaustive()
    }
}

/// LLVM's `Module::ModFlagBehavior` values `Max` and `Min`
/// (`llvm/IR/Module.h`; the LLVM Language Reference, "Module Flags Metadata"),
/// which LLVM-C's `LLVMModuleFlagBehavior` lacks: linking keeps the larger or
/// the smaller of two integer values.
const MODULE_FLAG_MAX: u64 = 7;
const MODULE_FLAG_MIN: u64 = 8;

impl Session<'_> {
    /// Appends a file-scope assembly block to the module's inline assembly,
    /// as Clang does for each top-level `asm`; LLVM ends each block with a
    /// newline and assembles them, in order, before the functions.
    /// GNU extension: GCC manual, "Basic Asm", outside functions.
    pub(super) fn append_module_asm(&self, text: &str) {
        // SAFETY: The module is live; LLVM copies `text.len()` bytes, which
        // need no terminator.
        unsafe {
            llvm_sys::core::LLVMAppendModuleInlineAsm(
                self.module,
                text.as_ptr().cast(),
                text.len(),
            );
        }
    }

    /// Adds the integer module flag `key` with the merge `behavior`, as the
    /// `!{i32 behavior, !"key", i32 value}` operand of `llvm.module.flags`
    /// that `Module::addModuleFlag` creates; LLVM-C's `LLVMAddModuleFlag`
    /// cannot express `Min` or `Max`.
    fn module_flag(&self, behavior: u64, key: &str, value: u64) {
        // SAFETY: The context is live.
        let int = unsafe { LLVMInt32TypeInContext(self.context) };
        let operands = [behavior, value].map(|number| {
            // SAFETY: A constant of the live i32 type.
            let constant = unsafe { llvm_sys::core::LLVMConstInt(int, number, 0) };
            // SAFETY: Wraps the live constant as metadata.
            unsafe { llvm_sys::core::LLVMValueAsMetadata(constant) }
        });
        // SAFETY: The context is live; LLVM copies the key bytes.
        let key = unsafe {
            llvm_sys::core::LLVMMDStringInContext2(self.context, key.as_ptr().cast(), key.len())
        };
        let mut tuple = [operands[0], key, operands[1]];
        // SAFETY: The context and the three metadata operands are live.
        let node = unsafe {
            llvm_sys::core::LLVMMDNodeInContext2(self.context, tuple.as_mut_ptr(), tuple.len())
        };
        // SAFETY: The context and its metadata node are live.
        let node = unsafe { llvm_sys::core::LLVMMetadataAsValue(self.context, node) };
        // SAFETY: The module is live, the name is terminated, and the operand
        // is the context's metadata node as a value.
        unsafe {
            llvm_sys::core::LLVMAddNamedMetadataOperand(
                self.module,
                c"llvm.module.flags".as_ptr(),
                node,
            );
        }
    }

    /// The relocation model of ELF `target`s, or `None` on Windows, where
    /// LLVM's default model is kept and the one TLS model ignores it.
    pub(super) fn elf_relocation(&self, target: Target) -> Option<Relocation> {
        matches!(target, Target::LinuxGnu | Target::LinuxMusl).then(|| self.code.relocation(target))
    }

    /// The TLS model a thread-local global of ELF `target` requests;
    /// `defined` tells whether the module defines it. Windows has one
    /// model, through `_tls_index` and the TEB.
    pub(super) fn thread_local_model(&self, target: Target, defined: bool) -> TlsModel {
        if self.elf_relocation(target).is_some() {
            self.code.thread_local_model(target, defined)
        } else {
            TlsModel::GlobalDynamic
        }
    }
}
