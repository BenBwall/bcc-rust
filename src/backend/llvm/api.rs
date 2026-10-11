//! Safe, scoped ownership of LLVM-C objects. Handles borrow the session;
//! LLVM owns their storage on its C++ heap, outside Rust's global allocator.
//! No LLVM owner is stored in an arena. Only this module calls llvm-sys.
//! The back end passes handles only to their originating session: this is a
//! module invariant, asserted in debug builds. Transparent handle slices are
//! borrowed directly by LLVM-C, whose array inputs are read-only despite
//! `*mut`.

use std::{
    ffi::{
        CStr,
        c_char,
    },
    fmt,
    marker::PhantomData,
    ptr,
    sync::Once,
};

use llvm_sys::{
    LLVMIntPredicate,
    LLVMLinkage,
    LLVMOpcode,
    LLVMRealPredicate,
    analysis::{
        LLVMVerifierFailureAction,
        LLVMVerifyModule,
    },
    core::{
        LLVMAddAttributeAtIndex,
        LLVMAddCallSiteAttribute,
        LLVMAddCase,
        LLVMAddFunction,
        LLVMAddGlobal,
        LLVMAddIncoming,
        LLVMAppendBasicBlockInContext,
        LLVMArrayType2,
        LLVMBuildAlloca,
        LLVMBuildBinOp,
        LLVMBuildBr,
        LLVMBuildCall2,
        LLVMBuildCast,
        LLVMBuildCondBr,
        LLVMBuildFCmp,
        LLVMBuildFNeg,
        LLVMBuildFreeze,
        LLVMBuildGEP2,
        LLVMBuildICmp,
        LLVMBuildInBoundsGEP2,
        LLVMBuildLoad2,
        LLVMBuildPhi,
        LLVMBuildRet,
        LLVMBuildRetVoid,
        LLVMBuildSelect,
        LLVMBuildStore,
        LLVMBuildSwitch,
        LLVMBuildUnreachable,
        LLVMBuildVAArg,
        LLVMConstBitCast,
        LLVMConstGEP2,
        LLVMConstIntOfArbitraryPrecision,
        LLVMConstNull,
        LLVMConstStringInContext2,
        LLVMConstStructInContext,
        LLVMContextCreate,
        LLVMContextDispose,
        LLVMCreateBuilderInContext,
        LLVMCreateEnumAttribute,
        LLVMCreateMessage,
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
        LLVMGetEnumAttributeKindForName,
        LLVMGetIntrinsicDeclaration,
        LLVMGetParam,
        LLVMGetPoison,
        LLVMInstructionEraseFromParent,
        LLVMInt1TypeInContext,
        LLVMInt8TypeInContext,
        LLVMInt16TypeInContext,
        LLVMInt32TypeInContext,
        LLVMInt64TypeInContext,
        LLVMInt128TypeInContext,
        LLVMIntTypeInContext,
        LLVMIntrinsicGetType,
        LLVMLookupIntrinsicID,
        LLVMModuleCreateWithNameInContext,
        LLVMPointerTypeInContext,
        LLVMPositionBuilderAtEnd,
        LLVMPrintModuleToString,
        LLVMSetAlignment,
        LLVMSetDataLayout,
        LLVMSetExact,
        LLVMSetGlobalConstant,
        LLVMSetInitializer,
        LLVMSetLinkage,
        LLVMSetNSW,
        LLVMSetNUW,
        LLVMSetOperand,
        LLVMSetTarget,
        LLVMSetValueName2,
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
    prelude::{
        LLVMBasicBlockRef,
        LLVMBuilderRef,
        LLVMContextRef,
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
    target::Target,
    util::bump::Bump,
};

/// A session owns a context, module and builder in disposal order.
pub(super) struct Session<'s> {
    builder: LLVMBuilderRef,
    module:  LLVMModuleRef,
    context: LLVMContextRef,
    arena:   &'s Bump,
}

#[derive(Clone, Copy)]
// ABI-identical to the LLVMTypeRef it wraps, including in slices.
#[repr(transparent)]
pub(super) struct Ty<'a>(LLVMTypeRef, PhantomData<&'a Session<'a>>);
#[derive(Clone, Copy)]
// ABI-identical to the LLVMValueRef it wraps, including in slices.
#[repr(transparent)]
pub(super) struct Val<'a>(LLVMValueRef, PhantomData<&'a Session<'a>>);
#[derive(Clone, Copy)]
// ABI-identical to the LLVMBasicBlockRef it wraps, including in slices.
#[repr(transparent)]
pub(super) struct Block<'a>(LLVMBasicBlockRef, PhantomData<&'a Session<'a>>);

#[cfg_attr(
    not(debug_assertions),
    expect(
        clippy::unused_self,
        reason = "The session receiver binds handle lifetimes and ownership; validation is \
                  debug-only."
    )
)]
impl<'s> Session<'s> {
    #[cfg(debug_assertions)]
    fn check_type(&self, ty: Ty<'_>) {
        // SAFETY: Type handles borrow a live session and cannot be forged.
        let context = unsafe { llvm_sys::core::LLVMGetTypeContext(ty.0) };
        assert_eq!(
            context, self.context,
            "LLVM handle belongs to another session"
        );
    }

    #[cfg(debug_assertions)]
    fn check_value(&self, value: Val<'_>) {
        // SAFETY: Value handles borrow a live session and cannot be forged.
        let ty = unsafe { LLVMTypeOf(value.0) };
        self.check_type(Ty(ty, PhantomData));
    }

    #[cfg(debug_assertions)]
    fn check_block(&self, block: Block<'_>) {
        // SAFETY: Every block handle was appended to a live function in a
        // session.
        let function = unsafe { llvm_sys::core::LLVMGetBasicBlockParent(block.0) };
        self.check_value(Val(function, PhantomData));
    }

    pub(super) fn new(arena: &'s Bump, target: Target) -> Self {
        // SAFETY: Creates a new independently owned LLVM context.
        let context = unsafe { LLVMContextCreate() };
        // SAFETY: The context is live and the module name is NUL terminated.
        let module = unsafe { LLVMModuleCreateWithNameInContext(c"bcc".as_ptr(), context) };
        // SAFETY: The context is live.
        let builder = unsafe { LLVMCreateBuilderInContext(context) };
        let session = Self {
            builder,
            module,
            context,
            arena,
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
        if position_independent(target) {
            // Clang marks ELF executables as PIC and PIE (level 2), which
            // lets code generation use direct references to symbols the
            // executable defines.
            for key in ["PIC Level", "PIE Level"] {
                // SAFETY: The context is live.
                let int = unsafe { LLVMInt32TypeInContext(context) };
                // SAFETY: A constant of the live i32 type.
                let two = unsafe { llvm_sys::core::LLVMConstInt(int, 2, 0) };
                // SAFETY: Wraps the live constant as metadata.
                let value = unsafe { llvm_sys::core::LLVMValueAsMetadata(two) };
                // SAFETY: Module, key bytes and metadata are live; LLVM copies
                // the key.
                unsafe {
                    llvm_sys::core::LLVMAddModuleFlag(
                        module,
                        llvm_sys::LLVMModuleFlagBehavior::LLVMModuleFlagBehaviorOverride,
                        key.as_ptr().cast(),
                        key.len(),
                        value,
                    );
                }
            }
        }
        session
    }

    pub(super) fn name(&self, name: &str) -> &CStr {
        debug_assert!(!name.as_bytes().contains(&0), "IR names contain no NUL");
        let bytes = self
            .arena
            .alloc_slice_fill_iter(name.bytes().chain(std::iter::once(0)));
        // SAFETY: IR names and target strings contain no NUL by construction,
        // asserted above in debug builds; allocation appends exactly one NUL.
        unsafe { CStr::from_bytes_with_nul_unchecked(bytes) }
    }

    pub(super) fn ty(&self, ty: Type) -> Ty<'_> {
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
            // SAFETY: Opaque pointers need only the live context and address space.
            | Type::Ptr => unsafe { LLVMPointerTypeInContext(self.context, 0) },
        };
        Ty(raw, PhantomData)
    }

    pub(super) fn void(&self) -> Ty<'_> {
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMVoidTypeInContext(self.context) };
        Ty(raw, PhantomData)
    }

    pub(super) fn array(&self, ty: Ty<'_>, size: u64) -> Ty<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMArrayType2(ty.0, size) };
        Ty(raw, PhantomData)
    }

    pub(super) fn vector(&self, ty: Ty<'_>, size: u32) -> Ty<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMVectorType(ty.0, size) };
        Ty(raw, PhantomData)
    }

    pub(super) fn structure(&self, types: &[Ty<'_>], packed: bool) -> Ty<'_> {
        #[cfg(debug_assertions)]
        for &ty in types {
            self.check_type(ty);
        }
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds. Ty is ABI-identical to LLVMTypeRef; LLVM
        // only reads the counted array despite its *mut parameter.
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

    pub(super) fn signature(&self, result: Ty<'_>, types: &[Ty<'_>], variadic: bool) -> Ty<'_> {
        #[cfg(debug_assertions)]
        self.check_type(result);
        #[cfg(debug_assertions)]
        for &ty in types {
            self.check_type(ty);
        }
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds. Ty is ABI-identical to LLVMTypeRef; LLVM
        // only reads the counted array despite its *mut parameter.
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

    pub(super) fn function(&self, name: &CStr, ty: Ty<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMAddFunction(self.module, name.as_ptr(), ty.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn global(&self, name: &CStr, ty: Ty<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMAddGlobal(self.module, ty.0, name.as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn block(&self, function: Val<'_>, name: &CStr) -> Block<'_> {
        #[cfg(debug_assertions)]
        self.check_value(function);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMAppendBasicBlockInContext(self.context, function.0, name.as_ptr()) };
        Block(raw, PhantomData)
    }

    pub(super) fn position(&self, block: Block<'_>) {
        #[cfg(debug_assertions)]
        self.check_block(block);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        unsafe {
            LLVMPositionBuilderAtEnd(self.builder, block.0);
        }
    }

    pub(super) fn param(&self, function: Val<'_>, index: u32) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(function);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMGetParam(function.0, index) };
        Val(raw, PhantomData)
    }

    pub(super) fn set_name(&self, value: Val<'_>, name: &str) {
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: Value belongs to this live session by module invariant,
        // asserted in debug builds; globals and scalar constants must keep
        // their identity.
        if !unsafe { llvm_sys::core::LLVMIsAConstant(value.0) }.is_null() {
            return;
        }
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        unsafe {
            LLVMSetValueName2(value.0, name.as_ptr().cast(), name.len());
        }
    }

    pub(super) fn alignment(&self, value: Val<'_>, align: u64) {
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        unsafe {
            LLVMSetAlignment(
                value.0,
                count(usize::try_from(align).expect("alignment fits usize")),
            );
        }
    }

    pub(super) fn volatile(&self, value: Val<'_>, volatile: bool) {
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        unsafe {
            LLVMSetVolatile(value.0, i32::from(volatile));
        }
    }

    pub(super) fn initializer(&self, global: Val<'_>, value: Val<'_>) {
        #[cfg(debug_assertions)]
        self.check_value(global);
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        unsafe {
            LLVMSetInitializer(global.0, value.0);
        }
    }

    pub(super) fn constant_global(&self, global: Val<'_>, constant: bool) {
        #[cfg(debug_assertions)]
        self.check_value(global);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        unsafe {
            LLVMSetGlobalConstant(global.0, i32::from(constant));
        }
    }

    pub(super) fn internal(&self, value: Val<'_>) {
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        unsafe {
            LLVMSetLinkage(value.0, LLVMLinkage::LLVMInternalLinkage);
        }
    }

    pub(super) fn null(&self, ty: Ty<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMConstNull(ty.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn poison(&self, ty: Ty<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMGetPoison(ty.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn bits(&self, ty: Type, bits: u128) -> Val<'_> {
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

    pub(super) fn bytes(&self, bytes: &[u8]) -> Val<'_> {
        // SAFETY: LLVM copies the counted bytes, with no implicit terminator.
        Val(
            unsafe {
                LLVMConstStringInContext2(self.context, bytes.as_ptr().cast(), bytes.len(), 1)
            },
            PhantomData,
        )
    }

    pub(super) fn const_structure(&self, values: &[Val<'_>]) -> Val<'_> {
        #[cfg(debug_assertions)]
        for &value in values {
            self.check_value(value);
        }
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds. Val is ABI-identical to LLVMValueRef;
        // constants have the verified IR types. LLVM only reads the counted
        // array despite its *mut parameter.
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

    pub(super) fn const_gep(&self, base: Val<'_>, offset: i64) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(base);
        let index = self.bits(Type::I64, offset as u128);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds. A single byte offset indexes the opaque
        // pointer constant. Val is ABI-identical to LLVMValueRef; LLVM only
        // reads the index array despite its *mut parameter.
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

    pub(super) fn alloca(&self, ty: Ty<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildAlloca(self.builder, ty.0, c"".as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn phi(&self, ty: Ty<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildPhi(self.builder, ty.0, c"".as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn binary(&self, opcode: LLVMOpcode, a: Val<'_>, b: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(a);
        #[cfg(debug_assertions)]
        self.check_value(b);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = self.unfolded(a.0, |operand| unsafe {
            LLVMBuildBinOp(self.builder, opcode, operand, b.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn cast(&self, opcode: LLVMOpcode, value: Val<'_>, ty: Ty<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(value);
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = self.unfolded(value.0, |operand| unsafe {
            LLVMBuildCast(self.builder, opcode, operand, ty.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn fneg(&self, value: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = self.unfolded(value.0, |operand| unsafe {
            LLVMBuildFNeg(self.builder, operand, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn freeze(&self, value: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = self.unfolded(value.0, |operand| unsafe {
            LLVMBuildFreeze(self.builder, operand, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn va_arg(&self, value: Val<'_>, ty: Ty<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(value);
        #[cfg(debug_assertions)]
        self.check_type(ty);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildVAArg(self.builder, value.0, ty.0, c"".as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn icmp(&self, cond: LLVMIntPredicate, a: Val<'_>, b: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(a);
        #[cfg(debug_assertions)]
        self.check_value(b);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = self.unfolded(a.0, |operand| unsafe {
            LLVMBuildICmp(self.builder, cond, operand, b.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn fcmp(&self, cond: LLVMRealPredicate, a: Val<'_>, b: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(a);
        #[cfg(debug_assertions)]
        self.check_value(b);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = self.unfolded(a.0, |operand| unsafe {
            LLVMBuildFCmp(self.builder, cond, operand, b.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn select(&self, cond: Val<'_>, a: Val<'_>, b: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(cond);
        #[cfg(debug_assertions)]
        self.check_value(a);
        #[cfg(debug_assertions)]
        self.check_value(b);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = self.unfolded(cond.0, |operand| unsafe {
            LLVMBuildSelect(self.builder, operand, a.0, b.0, c"".as_ptr())
        });
        Val(raw, PhantomData)
    }

    pub(super) fn load(&self, ty: Ty<'_>, addr: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        #[cfg(debug_assertions)]
        self.check_value(addr);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildLoad2(self.builder, ty.0, addr.0, c"".as_ptr()) };
        Val(raw, PhantomData)
    }

    pub(super) fn store(&self, value: Val<'_>, addr: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(value);
        #[cfg(debug_assertions)]
        self.check_value(addr);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildStore(self.builder, value.0, addr.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn jump(&self, block: Block<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_block(block);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildBr(self.builder, block.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn branch(&self, cond: Val<'_>, then: Block<'_>, other: Block<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(cond);
        #[cfg(debug_assertions)]
        self.check_block(then);
        #[cfg(debug_assertions)]
        self.check_block(other);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildCondBr(self.builder, cond.0, then.0, other.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn switch(&self, value: Val<'_>, default: Block<'_>, cases: u32) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(value);
        #[cfg(debug_assertions)]
        self.check_block(default);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildSwitch(self.builder, value.0, default.0, cases) };
        Val(raw, PhantomData)
    }

    pub(super) fn ret(&self, value: Val<'_>) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildRet(self.builder, value.0) };
        Val(raw, PhantomData)
    }

    pub(super) fn ret_void(&self) -> Val<'_> {
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildRetVoid(self.builder) };
        Val(raw, PhantomData)
    }

    pub(super) fn unreachable(&self) -> Val<'_> {
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
        let raw = unsafe { LLVMBuildUnreachable(self.builder) };
        Val(raw, PhantomData)
    }

    pub(super) fn add_case(&self, switch: Val<'_>, value: Val<'_>, block: Block<'_>) {
        #[cfg(debug_assertions)]
        self.check_value(switch);
        #[cfg(debug_assertions)]
        self.check_value(value);
        #[cfg(debug_assertions)]
        self.check_block(block);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds; operands have the verified IR types.
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
        // SAFETY: Input belongs to this context by module invariant, asserted
        // by callers in debug builds, and has a first-class type.
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

    pub(super) fn incoming(&self, phi: Val<'_>, value: Val<'_>, block: Block<'_>) {
        #[cfg(debug_assertions)]
        self.check_value(phi);
        #[cfg(debug_assertions)]
        self.check_value(value);
        #[cfg(debug_assertions)]
        self.check_block(block);
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds. The phi and incoming value have matching
        // types. Val/Block are ABI-identical to LLVMValueRef/LLVMBasicBlockRef;
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

    pub(super) fn gep(&self, base: Val<'_>, offset: Val<'_>, inbounds: bool) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_value(base);
        #[cfg(debug_assertions)]
        self.check_value(offset);
        let index = ptr::from_ref(&offset).cast::<LLVMValueRef>().cast_mut();
        let ty = self.ty(Type::I8).0;
        let raw = if inbounds {
            // SAFETY: Handles belong to this live session by module invariant,
            // asserted in debug builds. One integer index addresses bytes at
            // an opaque pointer. Val is ABI-identical to LLVMValueRef; LLVM
            // only reads the index array despite its *mut parameter.
            self.unfolded(base.0, |operand| unsafe {
                LLVMBuildInBoundsGEP2(self.builder, ty, operand, index, 1, c"".as_ptr())
            })
        } else {
            // SAFETY: Handles belong to this live session by module invariant,
            // asserted in debug builds. One integer index addresses bytes at
            // an opaque pointer. Val is ABI-identical to LLVMValueRef; LLVM
            // only reads the index array despite its *mut parameter.
            self.unfolded(base.0, |operand| unsafe {
                LLVMBuildGEP2(self.builder, ty, operand, index, 1, c"".as_ptr())
            })
        };
        Val(raw, PhantomData)
    }

    pub(super) fn flags(&self, value: Val<'_>, flags: InstFlags) {
        #[cfg(debug_assertions)]
        self.check_value(value);
        if flags.contains(InstFlags::NUW) {
            // SAFETY: Value belongs to this live session by module invariant,
            // asserted in debug builds. Verified IR allows NUW only on
            // overflowing arithmetic instructions.
            unsafe {
                LLVMSetNUW(value.0, 1);
            }
        }
        if flags.contains(InstFlags::NSW) {
            // SAFETY: Value belongs to this live session by module invariant,
            // asserted in debug builds. Verified IR allows NSW only on
            // overflowing arithmetic instructions.
            unsafe {
                LLVMSetNSW(value.0, 1);
            }
        }
        if flags.contains(InstFlags::EXACT) {
            // SAFETY: Value belongs to this live session by module invariant,
            // asserted in debug builds. Verified IR allows EXACT only on
            // divisions and right shifts.
            unsafe {
                LLVMSetExact(value.0, 1);
            }
        }
    }

    pub(super) fn call(&self, ty: Ty<'_>, function: Val<'_>, args: &[Val<'_>]) -> Val<'_> {
        #[cfg(debug_assertions)]
        self.check_type(ty);
        #[cfg(debug_assertions)]
        self.check_value(function);
        #[cfg(debug_assertions)]
        for &value in args {
            self.check_value(value);
        }
        // SAFETY: Handles belong to this live session by module invariant,
        // asserted in debug builds. The callee's ABI type matches the counted
        // arguments. Val is ABI-identical to LLVMValueRef; LLVM only reads the
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

    pub(super) fn intrinsic(&self, name: &str, overloads: &[Ty<'_>]) -> (Val<'_>, Ty<'_>) {
        // SAFETY: The intrinsic lookup reads the counted name.
        let id = unsafe { LLVMLookupIntrinsicID(name.as_ptr().cast(), name.len()) };
        #[cfg(debug_assertions)]
        for &ty in overloads {
            self.check_type(ty);
        }
        let types = overloads.as_ptr().cast::<LLVMTypeRef>().cast_mut();
        // SAFETY: Known intrinsic ID and correct overload types; handles belong
        // to this live session by module invariant, asserted in debug builds.
        // Ty is ABI-identical to LLVMTypeRef; LLVM only reads the counted array
        // despite its *mut parameter.
        let function =
            unsafe { LLVMGetIntrinsicDeclaration(self.module, id, types, overloads.len()) };
        // SAFETY: Same live-session invariant and ABI-identical overload list
        // as above, asserted in debug builds; LLVM only reads the array despite
        // its *mut parameter.
        let ty = unsafe { LLVMIntrinsicGetType(self.context, id, types, overloads.len()) };
        (Val(function, PhantomData), Ty(ty, PhantomData))
    }

    pub(super) fn attribute(
        &self,
        value: Val<'_>,
        index: u32,
        name: &str,
        number: u64,
        ty: Option<Ty<'_>>,
        call: bool,
    ) {
        #[cfg(debug_assertions)]
        self.check_value(value);
        // SAFETY: The counted attribute name is valid during lookup.
        let kind = unsafe { LLVMGetEnumAttributeKindForName(name.as_ptr().cast(), name.len()) };
        let attribute = if let Some(ty) = ty {
            #[cfg(debug_assertions)]
            self.check_type(ty);
            // SAFETY: The attribute kind is byval/sret and the type belongs to
            // this context by module invariant, asserted in debug builds.
            unsafe { LLVMCreateTypeAttribute(self.context, kind, ty.0) }
        } else {
            // SAFETY: Known enum attribute with its appropriate integer
            // payload.
            unsafe { LLVMCreateEnumAttribute(self.context, kind, number) }
        };
        if call {
            // SAFETY: A call instruction in this live session by module
            // invariant, asserted in debug builds, with a valid ABI
            // argument/result index.
            unsafe {
                LLVMAddCallSiteAttribute(value.0, index, attribute);
            }
        } else {
            // SAFETY: A function in this live session by module invariant,
            // asserted in debug builds, with a valid ABI parameter/result
            // index.
            unsafe {
                LLVMAddAttributeAtIndex(value.0, index, attribute);
            }
        }
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
        if failed != 0 { Err(message) } else { Ok(()) }
    }

    pub(super) fn print(&self) -> PrintedModule {
        // SAFETY: A live module; the returned message must be disposed.
        PrintedModule(Message(unsafe { LLVMPrintModuleToString(self.module) }))
    }

    pub(super) fn emit(
        &self,
        target: Target,
        opt: Option<OptLevel>,
        skip_passes: bool,
        assembly: bool,
    ) -> Result<Buffer, Message> {
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
        let level = match opt.unwrap_or(OptLevel::O0) {
            | OptLevel::O0 => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            | OptLevel::O1 => LLVMCodeGenOptLevel::LLVMCodeGenLevelLess,
            | OptLevel::O2 => LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
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
                if position_independent(target) {
                    LLVMRelocMode::LLVMRelocPIC
                } else {
                    LLVMRelocMode::LLVMRelocDefault
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
        if !skip_passes {
            let pipeline = match opt.unwrap_or(OptLevel::O0) {
                | OptLevel::O0 => c"default<O0>",
                | OptLevel::O1 => c"default<O1>",
                | OptLevel::O2 => c"default<O2>",
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
        let mut buffer = ptr::null_mut();
        let kind = if assembly {
            LLVMCodeGenFileType::LLVMAssemblyFile
        } else {
            LLVMCodeGenFileType::LLVMObjectFile
        };
        // SAFETY: Verified live module and machine; outputs are separately
        // owned.
        if unsafe {
            LLVMTargetMachineEmitToMemoryBuffer(
                machine.0,
                self.module,
                kind,
                &raw mut error,
                &raw mut buffer,
            )
        } != 0
        {
            return Err(Message(error));
        }
        Ok(Buffer(buffer))
    }
}

fn count(length: usize) -> u32 {
    u32::try_from(length).expect("LLVM list fits u32")
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

/// Whether the target links position-independent executables by default,
/// as Clang and GCC do on Linux; code for them must be PIC.
fn position_independent(target: Target) -> bool {
    matches!(target, Target::LinuxGnu | Target::LinuxMusl)
}
