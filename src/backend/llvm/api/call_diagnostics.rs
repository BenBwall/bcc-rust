//! Numeric metadata follows eligible calls through LLVM optimization. Only
//! reachable instructions in emitted function bodies contribute diagnostics.

use llvm_sys::{
    LLVMLinkage,
    LLVMValueKind,
    core::{
        LLVMConstInt,
        LLVMConstIntGetZExtValue,
        LLVMGetBasicBlockTerminator,
        LLVMGetFirstBasicBlock,
        LLVMGetFirstFunction,
        LLVMGetFirstGlobal,
        LLVMGetFirstGlobalAlias,
        LLVMGetFirstInstruction,
        LLVMGetLinkage,
        LLVMGetMDKindIDInContext,
        LLVMGetMDNodeNumOperands,
        LLVMGetMDNodeOperands,
        LLVMGetMetadata,
        LLVMGetNextFunction,
        LLVMGetNextGlobal,
        LLVMGetNextGlobalAlias,
        LLVMGetNextInstruction,
        LLVMGetNumOperands,
        LLVMGetNumSuccessors,
        LLVMGetOperand,
        LLVMGetSuccessor,
        LLVMGetValueKind,
        LLVMInt32TypeInContext,
        LLVMMDNodeInContext2,
        LLVMMetadataAsValue,
        LLVMSetMetadata,
        LLVMValueAsMetadata,
    },
    prelude::LLVMValueRef,
};

use super::{
    Session,
    Val,
};
use crate::util::bump::{
    ArenaSet,
    ArenaVec,
    Bump,
};

pub(super) fn attach(session: &Session<'_>, call: Val<'_>, id: u32) {
    // SAFETY: The session owns the live context.
    let ty = unsafe { LLVMInt32TypeInContext(session.context) };
    // SAFETY: The type is the live context's i32 type.
    let number = unsafe { LLVMConstInt(ty, u64::from(id), 0) };
    // SAFETY: The number belongs to the live context.
    let mut operand = unsafe { LLVMValueAsMetadata(number) };
    // SAFETY: The context is live and LLVM copies the one metadata operand.
    let node = unsafe { LLVMMDNodeInContext2(session.context, &raw mut operand, 1) };
    // SAFETY: The metadata belongs to this live context.
    let node = unsafe { LLVMMetadataAsValue(session.context, node) };
    let kind = kind(session);
    // SAFETY: The branded call and metadata belong to this live session.
    unsafe {
        LLVMSetMetadata(call.0, kind, node);
    }
}

pub(super) fn counts<'a>(session: &Session<'_>, length: usize, arena: &'a Bump) -> &'a [usize] {
    let mut counts = ArenaVec::new_in(arena);
    counts.resize(length, 0_usize);
    if length == 0 {
        return counts.leak();
    }
    let kind = kind(session);
    let mut pending = ArenaVec::new_in(arena);
    // SAFETY: The session owns the live module.
    let mut function = unsafe { LLVMGetFirstFunction(session.module) };
    while !function.is_null() {
        // SAFETY: This is a function in the live module.
        let linkage = unsafe { LLVMGetLinkage(function) };
        if !matches!(
            linkage,
            LLVMLinkage::LLVMInternalLinkage
                | LLVMLinkage::LLVMPrivateLinkage
                | LLVMLinkage::LLVMAvailableExternallyLinkage
        ) {
            pending.push(function);
        }
        // SAFETY: This is a function in the live module.
        function = unsafe { LLVMGetNextFunction(function) };
    }
    // Global initializers include escaped function addresses, llvm.used and
    // constructor/destructor arrays, matching the frontend's reachability
    // roots. SAFETY: The session owns the live module.
    let mut global = unsafe { LLVMGetFirstGlobal(session.module) };
    while !global.is_null() {
        references(global, &mut pending, arena);
        // SAFETY: This is a global in the live module.
        global = unsafe { LLVMGetNextGlobal(global) };
    }
    // SAFETY: The session owns the live module.
    let mut alias = unsafe { LLVMGetFirstGlobalAlias(session.module) };
    while !alias.is_null() {
        references(alias, &mut pending, arena);
        // SAFETY: This is an alias in the live module.
        alias = unsafe { LLVMGetNextGlobalAlias(alias) };
    }
    let mut seen_functions = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    let mut seen_blocks = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    let mut blocks = ArenaVec::new_in(arena);
    while let Some(function) = pending.pop() {
        if !seen_functions.insert(function as usize) {
            continue;
        }
        // SAFETY: Pending values are functions in the live module.
        let entry = unsafe { LLVMGetFirstBasicBlock(function) };
        if entry.is_null() {
            continue;
        }
        blocks.push(entry);
        while let Some(block) = blocks.pop() {
            if !seen_blocks.insert(block as usize) {
                continue;
            }
            // SAFETY: The block belongs to a live module function.
            let mut inst = unsafe { LLVMGetFirstInstruction(block) };
            while !inst.is_null() {
                // SAFETY: The instruction and metadata kind belong to this
                // context.
                let metadata = unsafe { LLVMGetMetadata(inst, kind) };
                if !metadata.is_null() {
                    // SAFETY: Only attach above creates this one-operand
                    // metadata.
                    let operands = unsafe { LLVMGetMDNodeNumOperands(metadata) };
                    debug_assert_eq!(operands, 1, "call diagnostic metadata contains one ID");
                    let mut id = std::ptr::null_mut();
                    // SAFETY: The one-operand metadata writes one live
                    // constant.
                    unsafe {
                        LLVMGetMDNodeOperands(metadata, &raw mut id);
                    }
                    // SAFETY: attach creates the metadata operand as an i32
                    // constant.
                    let id = usize::try_from(unsafe { LLVMConstIntGetZExtValue(id) })
                        .expect("i32 ID fits usize");
                    if let Some(count) = counts.get_mut(id) {
                        *count += 1;
                    }
                }
                references(inst, &mut pending, arena);
                // SAFETY: The instruction belongs to the live block.
                inst = unsafe { LLVMGetNextInstruction(inst) };
            }
            // SAFETY: The block belongs to the verified live module.
            let terminator = unsafe { LLVMGetBasicBlockTerminator(block) };
            if !terminator.is_null() {
                // SAFETY: This is a live block terminator.
                let length = unsafe { LLVMGetNumSuccessors(terminator) };
                for index in 0..length {
                    // SAFETY: The index is within this live terminator's
                    // successors.
                    blocks.push(unsafe { LLVMGetSuccessor(terminator, index) });
                }
            }
        }
    }
    counts.leak()
}

fn kind(session: &Session<'_>) -> u32 {
    let name = "bcc.call.diagnostic";
    // SAFETY: The context is live and the counted bytes remain valid for
    // lookup.
    unsafe {
        LLVMGetMDKindIDInContext(
            session.context,
            name.as_ptr().cast(),
            super::count(name.len()),
        )
    }
}

/// Visits function constants in the operands of one live instruction or global.
/// Instruction operands are not recursively followed: their definitions are
/// already visited in reachable blocks, avoiding SSA and CFG cycles.
fn references(value: LLVMValueRef, functions: &mut ArenaVec<'_, LLVMValueRef>, arena: &Bump) {
    let mut pending = ArenaVec::new_in(arena);
    pending.push(value);
    while let Some(value) = pending.pop() {
        // SAFETY: All values originate in the live module or its operand lists.
        let length = unsafe { LLVMGetNumOperands(value) };
        for index in 0..u32::try_from(length).expect("LLVM operand count is nonnegative") {
            // SAFETY: The index is within this live value's operand list.
            let operand = unsafe { LLVMGetOperand(value, index) };
            // SAFETY: LLVM returned a live operand of the live module.
            let kind = unsafe { LLVMGetValueKind(operand) };
            match kind {
                | LLVMValueKind::LLVMFunctionValueKind => functions.push(operand),
                | LLVMValueKind::LLVMConstantExprValueKind
                | LLVMValueKind::LLVMConstantArrayValueKind
                | LLVMValueKind::LLVMConstantStructValueKind
                | LLVMValueKind::LLVMConstantVectorValueKind
                | LLVMValueKind::LLVMBlockAddressValueKind
                | LLVMValueKind::LLVMGlobalAliasValueKind => pending.push(operand),
                | _ => {},
            }
        }
    }
}
