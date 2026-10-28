//! The integration tests, one module per subject, in one test binary.
//!
//! Most of them drive the compiled `bcc-rust` binary and the pinned
//! toolchain. One binary links once instead of once per subject, and its
//! tests all run in parallel. Select a subject by its module name, as in
//! `cargo test --test integration diagnostics_golden`.
//!
//! `tests/allocation_count.rs` stays a target of its own: it installs a
//! counting global allocator and needs `--features benchmarking-internals`
//! for its measurements.

#![expect(
    unused_crate_dependencies,
    reason = "Integration tests inherit package dependencies but exercise the compiled binary."
)]

#[path = "../support/native.rs"]
mod native;

mod atomic_compound;
mod bench_corpus;
mod binary128;
mod call_diagnostics;
mod cli;
mod code_generation_cli;
mod diagnostic_parity;
mod diagnostics_golden;
mod driver_cli;
mod explorer;
mod file_scope_asm;
mod freestanding_cli;
mod half_precision;
mod hosted_cli;
mod identity_programs;
mod inline_asm;
mod language_cli;
mod late_builtins;
mod link_temporary_object;
mod llvm_build_cache;
mod local_statics;
mod math_builtins_cli;
mod mingw_target_macros;
mod native_runner;
mod native_section_parity;
mod optimization_cli;
mod overloadable;
mod overloadable_abi;
mod personality_cli;
mod pointer_diagnostics;
mod preprocessing_cli;
mod reachable_functions;
mod semantic_cli;
mod static_compound_literals;
mod static_function_designators;
mod static_relocations;
mod target_resource_headers;
mod target_selection;
mod thread_local_linkage_cli;
mod torture_regressions;
mod windows_dll_flow;
mod windows_dll_interop;
