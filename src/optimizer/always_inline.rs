//! Inline definitions for the back ends that LLVM does not optimize: the
//! interpreter and the native back end. Read [`resolve_inline_copies`].
//!
//! Lowering gives an inline-only definition a private copy that records the
//! external function it stands for ([`Module::inline_copy_target`]). The
//! LLVM back end emits the copy as that function's `available_externally`
//! body, as Clang emits a C99 inline definition, so LLVM's always-inliner
//! expands the calls it can and every other call reaches the external
//! definition. This pass makes the same choices on bcc IR so that every back
//! end calls the same definition: it runs the always-inliner's single walk,
//! then points each call still naming a copy at the external function and
//! drops the copies.

use super::{
    draft::Draft,
    inline,
};
use crate::{
    ir::{
        Body,
        Entity,
        FuncId,
        InlineControl,
        InstData,
        Linkage,
        Module,
        Opcode,
        Type,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Expands `always_inline` calls as LLVM's always-inliner does at every
/// optimization level, then makes the calls that still name an inline copy
/// call its external function, and removes the copies' bodies.
///
/// LLVM visits the functions in module order, where a copy takes the place
/// of its external function, and expands every call of a function whose
/// body is inline viable at that point: it does not call itself (directly or
/// through its external function). Expanding one function into another can
/// make the second recursive, so the order decides which calls stay calls:
/// with `f` and `g` calling each other, `f` is expanded into its callers and
/// `g`, and `g` then calls itself and is not. Lowering declares the
/// `always_inline` bodies in the order Clang creates them, which the LLVM
/// back end and this walk both follow
/// ([`Unit::declare_items`](crate::lowering)), so all three agree.
///
/// Only a module whose choice can differ runs the walk: it has an
/// `always_inline` copy with an external function, or a weak `always_inline`
/// definition, which LLVM expands although the linker may choose another
/// definition. A module with only plain inline copies just loses them.
/// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125 (a call may use the inline or
/// the external definition). GNU extension: GCC manual, "Common Function
/// Attributes", `always_inline`.
pub(crate) fn resolve_inline_copies(module: &mut Module<'_>, scratch: &Bump) {
    let ids = scratch.alloc_slice_fill_iter(module.functions().map(|(func, _)| func));
    let mut copies = ArenaVec::new_in(scratch);
    copies.resize(module.functions().count(), None);
    let mut expand = false;
    for (func, function) in module.functions() {
        if let Some(external) = module.inline_copy_target(func) {
            copies[external.index()] = Some(func);
            expand |= function.inlining == InlineControl::Always;
        }
        expand |= function.inlining == InlineControl::Always
            && function.linkage == Linkage::Weak
            && function.body.is_some();
    }
    if expand {
        let mut function_scratch = Bump::new();
        // The module order LLVM sees: a copy in its external function's place.
        let order =
            scratch.alloc_slice_fill_iter(module.functions().filter_map(|(func, _)| match copies
                [func.index()]
            {
                | Some(copy) => Some(copy),
                | None if module.inline_copy_target(func).is_some() => None,
                | None => Some(func),
            }));
        for &callee in &*order {
            let function = module.function(callee);
            if function.inlining != InlineControl::Always {
                continue;
            }
            let external = module.inline_copy_target(callee);
            let names_callee = |func: FuncId| func == callee || Some(func) == external;
            let Some(body) = &function.body else { continue };
            if !viable(module, callee, body, names_callee) {
                continue;
            }
            for &user in &*ids {
                if !names_callee(user) {
                    expand_calls(module, user, callee, names_callee, &function_scratch);
                    function_scratch.reset();
                }
            }
        }
    }
    let targets = scratch.alloc_slice_fill_iter(
        module
            .functions()
            .map(|(func, _)| module.inline_copy_target(func)),
    );
    for &func in &*ids {
        if let Some(body) = &mut module.function_mut(func).body {
            body.retarget_calls(|callee| targets[callee.index()]);
        }
    }
    for &func in &*ids {
        if targets[func.index()].is_some() {
            module.function_mut(func).body = None;
        }
    }
}

/// Whether `body` can be expanded into its callers: LLVM's inline viability
/// (`isInlineViable`). The body does not call itself, start a variadic
/// argument list, branch indirectly, or call a function that may return
/// twice unless it may itself. A variadic body without `va_start` and one
/// with dynamic allocation are viable, as in LLVM; [`expand_calls`] brackets
/// the second with a stack save and restore, as LLVM's inliner does.
fn viable(
    module: &Module<'_>,
    func: FuncId,
    body: &Body<'_>,
    names_callee: impl Fn(FuncId) -> bool,
) -> bool {
    if body.has_indirect_control() {
        return false;
    }
    let returns_twice = module.returns_twice(func);
    let indirect_returns_twice = module.returns_twice_escapes();
    body.blocks().all(|block| {
        body.block_insts(block)
            .iter()
            .all(|&inst| match *body.inst(inst) {
                | InstData::Call { func, .. } =>
                    !names_callee(func) && (returns_twice || !module.returns_twice(func)),
                | InstData::CallIndirect { .. } => returns_twice || !indirect_returns_twice,
                | ref data => data.opcode() != Opcode::VaStart,
            })
    })
}

/// Expands every call of `callee` in `user`'s body. A callee that allocates
/// dynamically gets its own stack region: its storage is released where
/// the call would have returned, so a call in a loop does not grow the
/// stack (LLVM's `InlineFunction` inserts `stacksave`/`stackrestore`).
/// C99: §6.2.4p6, p. 32; PDF p. 44.
fn expand_calls(
    module: &mut Module<'_>,
    user: FuncId,
    callee: FuncId,
    names_callee: impl Fn(FuncId) -> bool,
    scratch: &Bump,
) {
    let calls = module.function(user).body.as_ref().is_some_and(|body| {
        body.blocks().any(|block| {
            body.block_insts(block).iter().any(
                |&inst| matches!(*body.inst(inst), InstData::Call { func, .. } if names_callee(func)),
            )
        })
    });
    if !calls {
        return;
    }
    let Some(body) = module.function_mut(user).body.take() else {
        return;
    };
    let mut draft = Draft::lift(&body, scratch);
    let mut sites = ArenaVec::new_in(scratch);
    for block in draft.alive_blocks() {
        for &inst in &draft.block(block).insts {
            if !draft.is_removed(inst)
                && matches!(*draft.inst(inst), InstData::Call { func, .. } if names_callee(func))
            {
                sites.push(inst);
            }
        }
    }
    let callee_body = module
        .function(callee)
        .body
        .as_ref()
        .expect("a viable callee has a body");
    let dynamic = callee_body.blocks().any(|block| {
        callee_body
            .block_insts(block)
            .iter()
            .any(|&inst| callee_body.inst(inst).opcode() == Opcode::DynAlloca)
    });
    for call in sites {
        let location = draft.alive_blocks().find_map(|block| {
            draft
                .block(block)
                .insts
                .iter()
                .position(|&inst| inst == call)
                .map(|position| (block, position))
        });
        let Some((block, mut position)) = location else {
            continue;
        };
        let save = dynamic.then(|| {
            let save = draft.add_inst(
                block,
                position,
                InstData::Nullary {
                    opcode: Opcode::StackSave,
                    ty:     Type::Ptr,
                },
                Some(Type::Ptr),
            );
            position += 1;
            save.expect("a stack save has a result")
        });
        let continuation = inline::expand(&mut draft, callee_body, block, position, call);
        if let Some(save) = save {
            _ = draft.add_inst(
                continuation,
                0,
                InstData::Unary {
                    opcode: Opcode::StackRestore,
                    ty:     Type::Ptr,
                    arg:    save,
                },
                None,
            );
        }
    }
    draft.lower(module, user);
}
