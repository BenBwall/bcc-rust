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
    #[cfg(test)]
    if matches!(target, Target::LinuxGnu | Target::LinuxMusl) {
        super::tests::object::equivalent(text)?;
        return build_object(text, out_path, target, kind);
    }
    let toolchain = Toolchain::bundled(target)?;
    let arena = Bump::new();
    let (assembly, mut file) = TemporaryObject::create(out_path, "s", &arena)?;
    file.write_all(text.as_bytes())?;
    drop(file);
    toolchain.assemble(assembly.path(), out_path, target, kind)
}

/// Builds direct ELF bytes, retaining the text assembler for COFF targets.
/// All encoding succeeds before the output file is opened.
pub(crate) fn build_object(
    text: &str,
    out_path: &Path,
    target: Target,
    kind: OutputKind,
) -> io::Result<()> {
    if !matches!(target, Target::LinuxGnu | Target::LinuxMusl) {
        return assemble(text, out_path, target, kind);
    }
    if kind == OutputKind::SharedLibrary {
        return Err(io::Error::other(super::NativeError::SharedLibraryCode));
    }
    let arena = Bump::new();
    let bytes = super::object::encode(text, &arena).map_err(io::Error::other)?;
    if kind == OutputKind::Object {
        return std::fs::write(out_path, &bytes);
    }
    let (object, mut file) = TemporaryObject::create(out_path, "o", &arena)?;
    file.write_all(&bytes)?;
    drop(file);
    Toolchain::bundled(target)?.assemble(object.path(), out_path, target, kind)
}
