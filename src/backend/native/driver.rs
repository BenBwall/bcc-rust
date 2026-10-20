//! Turns the native listing into an object or executable: [`object_writer`]
//! chooses between encoding the object in process ([`build_object`]) and
//! assembling the listing with the bundled clang ([`assemble`]), whose
//! integrated assembler is LLVM's. Executables of both paths link with LLD
//! through the bundled clang, as the LLVM back end's executables are linked.

use std::{
    io::{
        self,
        Write as _,
    },
    path::Path,
};

use super::NativeError;
use crate::{
    backend::llvm::{
        OutputKind,
        TemporaryObject,
        Toolchain,
    },
    ir::Module,
    target::Target,
    util::bump::Bump,
};

/// How `-o` turns the native listing into an object.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ObjectWriter {
    /// ELF or COFF bytes encoded in process ([`build_object`]).
    Direct,
    /// The listing assembled by the bundled clang ([`assemble`]).
    Assembler,
}

/// The object writer for `module` on `target`: the one `requested` names
/// (`--native-object` or `--no-native-object`), else direct COFF objects on
/// both Windows targets and the assembler on Linux, where direct ELF objects
/// stay opt-in. The encoder reads only the emitter's own vocabulary, so a
/// module with file-scope assembly, which is arbitrary assembler input, takes
/// the assembler by default and is refused when direct objects are requested.
pub(crate) fn object_writer(
    module: &Module<'_>,
    target: Target,
    requested: Option<ObjectWriter>,
) -> Result<ObjectWriter, NativeError<'static>> {
    let encodable = module.module_asm().is_empty();
    match requested {
        | Some(ObjectWriter::Direct) if !encodable => Err(NativeError::ObjectModuleAsm),
        | Some(writer) => Ok(writer),
        | None if encodable && matches!(target, Target::WindowsGnu | Target::WindowsMsvc) =>
            Ok(ObjectWriter::Direct),
        | None => Ok(ObjectWriter::Assembler),
    }
}

/// Writes `text` to a temporary `.s` file beside `out_path` and assembles
/// it, linking an executable unless `kind` asks for an object.
pub(crate) fn assemble(
    text: &str,
    out_path: &Path,
    target: Target,
    kind: OutputKind,
) -> io::Result<()> {
    // The direct encoder refuses file-scope assembly, so such listings are
    // only assembled.
    #[cfg(test)]
    if matches!(target, Target::LinuxGnu | Target::LinuxMusl)
        && !text.contains(super::emit::MODULE_ASM_START)
    {
        super::tests::object::equivalent(text)?;
        return build_object(text, out_path, target, kind);
    }
    #[cfg(test)]
    if matches!(target, Target::WindowsGnu | Target::WindowsMsvc)
        && !text.contains(super::emit::MODULE_ASM_START)
    {
        super::tests::object_coff::equivalent(text, target)?;
        return build_object(text, out_path, target, kind);
    }
    let toolchain = Toolchain::bundled(target)?;
    let arena = Bump::new();
    let (assembly, mut file) = TemporaryObject::create(out_path, "s", &arena)?;
    file.write_all(text.as_bytes())?;
    drop(file);
    toolchain.assemble(assembly.path(), out_path, target, kind)
}

/// Builds direct ELF or COFF bytes from the private emitter listing.
/// All encoding succeeds before the output file is opened.
pub(crate) fn build_object(
    text: &str,
    out_path: &Path,
    target: Target,
    kind: OutputKind,
) -> io::Result<()> {
    if kind == OutputKind::SharedLibrary {
        return Err(io::Error::other(NativeError::SharedLibraryCode));
    }
    let arena = Bump::new();
    let bytes = match target {
        | Target::LinuxGnu | Target::LinuxMusl => super::object::encode(text, &arena),
        | Target::WindowsGnu | Target::WindowsMsvc =>
            super::object::encode_coff(text, target, &arena),
    }
    .map_err(io::Error::other)?;
    if kind == OutputKind::Object {
        return std::fs::write(out_path, &bytes);
    }
    let (object, mut file) = TemporaryObject::create(out_path, "o", &arena)?;
    file.write_all(&bytes)?;
    drop(file);
    Toolchain::bundled(target)?.assemble(object.path(), out_path, target, kind)
}
