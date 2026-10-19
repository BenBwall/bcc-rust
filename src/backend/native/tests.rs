//! Tests of the native back end: selection and allocation on textual IR,
//! and end-to-end runs of C programs compared with the interpreter.

mod allocation;
mod end_to_end;
mod programs;
mod selection;

use super::{
    conv::CallConv,
    emit_assembly,
    isel,
    mir::MFunction,
    regalloc::{
        self,
        Allocation,
        CheckError,
    },
};
use crate::{
    ir::{
        Module,
        parse_module,
    },
    target::Target,
    util::bump::Bump,
};

/// The assembly of a textual post-ABI module for `target`.
fn assembly(ir: &str, target: Target) -> String {
    let arena = Bump::new();
    let module = parse_module(&arena, ir).unwrap_or_else(|error| panic!("{error}\n{ir}"));
    let mut text = String::new();
    emit_assembly(&module, target, &mut text).unwrap_or_else(|error| panic!("{error}\n{ir}"));
    text
}

/// The refusal of a textual post-ABI module for `target`.
fn refusal(ir: &str, target: Target) -> String {
    let arena = Bump::new();
    let module = parse_module(&arena, ir).unwrap_or_else(|error| panic!("{error}\n{ir}"));
    let mut text = String::new();
    match emit_assembly(&module, target, &mut text) {
        | Ok(()) => panic!("the module was accepted:\n{text}"),
        | Err(error) => error.to_string(),
    }
}

/// The text of one function in an assembly listing, from its label to the
/// end of its unwind information.
fn function_text<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("\n{name}:\n"))
        .unwrap_or_else(|| panic!("no function {name} in\n{text}"));
    let rest = &text[start..];
    let end = rest
        .find(".cfi_endproc")
        .or_else(|| rest.find(".seh_endproc"))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// Selects and allocates `function` of a textual module and hands the
/// allocated machine function to `then`.
fn with_allocation<R>(
    ir: &str,
    function: &str,
    target: Target,
    then: impl FnOnce(&Module<'_>, &MFunction<'_>, &Allocation<'_>) -> R,
) -> R {
    let arena = Bump::new();
    let module = parse_module(&arena, ir).unwrap_or_else(|error| panic!("{error}\n{ir}"));
    let Some(crate::ir::Symbol::Function(id)) = module.symbol(function) else {
        panic!("no function {function}");
    };
    let definition = module.function(id);
    let body = definition.body.as_ref().expect("a defined function");
    let scratch = Bump::new();
    let mut func = isel::select(
        &module,
        definition,
        body,
        target,
        crate::target::TlsModel::default(),
        &scratch,
    )
    .unwrap();
    let allocation = regalloc::allocate(&mut func, CallConv::of(target), &scratch);
    then(&module, &func, &allocation)
}

/// Selects and allocates `function`, lets `corrupt` change the allocation,
/// then rewrites the function and runs the allocation checker on it.
fn check_corrupted(
    ir: &str,
    function: &str,
    target: Target,
    corrupt: impl FnOnce(&MFunction<'_>, &mut Allocation<'_>),
) -> Result<(), Box<CheckError>> {
    let arena = Bump::new();
    let module = parse_module(&arena, ir).unwrap_or_else(|error| panic!("{error}\n{ir}"));
    let Some(crate::ir::Symbol::Function(id)) = module.symbol(function) else {
        panic!("no function {function}");
    };
    let definition = module.function(id);
    let body = definition.body.as_ref().expect("a defined function");
    let scratch = Bump::new();
    let mut func = isel::select(
        &module,
        definition,
        body,
        target,
        crate::target::TlsModel::default(),
        &scratch,
    )
    .unwrap();
    let mut allocation = regalloc::allocate(&mut func, CallConv::of(target), &scratch);
    corrupt(&func, &mut allocation);
    regalloc::rewrite_checked(&mut func, &allocation, &scratch).map_err(|error| Box::new(*error))
}
