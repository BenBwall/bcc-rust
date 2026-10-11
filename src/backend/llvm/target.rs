//! Each target's LLVM data layout, as the pinned clang emits it.

use crate::target::Target;

/// The `target datalayout` string the pinned clang 23.1.1 writes for an empty
/// C file with `--target=<triple> -S -emit-llvm`, recorded on 2026-10-11.
/// Only the symbol mangling differs: ELF (`m:e`) on Linux and COFF (`m:w`) on
/// Windows. Every target is little-endian with 64-bit pointers, 128-bit
/// aligned `i128` and `x86_fp80`, and a 16-byte stack alignment.
///
/// A test runs clang again and compares, so an LLVM upgrade that changes a
/// layout fails loudly instead of feeding LLVM a stale one, which it would
/// accept silently.
pub(crate) const fn data_layout(target: Target) -> &'static str {
    match target {
        | Target::LinuxGnu | Target::LinuxMusl =>
            "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
        | Target::WindowsGnu | Target::WindowsMsvc =>
            "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
    }
}
