//! Turns assembly text into an object or executable with the bundled clang,
//! which assembles with LLVM's integrated assembler and links with LLD as
//! the LLVM back end's executables are linked.

use std::{
    io::{
        self,
        Write as _,
    },
    path::Path,
};

use crate::{
    backend::llvm::{
        OutputKind,
        TemporaryObject,
        Toolchain,
    },
    target::Target,
    util::bump::Bump,
};

/// Writes `text` to a temporary `.s` file beside `out_path` and assembles
/// it, linking an executable unless `kind` asks for an object.
pub(crate) fn assemble(
    text: &str,
    out_path: &Path,
    target: Target,
    kind: OutputKind,
) -> io::Result<()> {
    let toolchain = Toolchain::bundled(target)?;
    let arena = Bump::new();
    let (assembly, mut file) = TemporaryObject::create(out_path, "s", &arena)?;
    file.write_all(text.as_bytes())?;
    drop(file);
    toolchain.assemble(assembly.path(), out_path, target, kind)
}
