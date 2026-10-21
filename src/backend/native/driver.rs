//! Turns the native listing into an object or executable: [`object_writer`]
//! chooses between encoding the object in process ([`build_object`]) and
//! assembling the listing with the bundled clang ([`assemble`]), whose
//! integrated assembler is LLVM's, and [`build`] runs that choice. Executables
//! of both paths link with LLD through the bundled clang, as the LLVM back
//! end's executables are linked.

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

/// How `-o` turns the native listing into an object, as [`object_writer`]
/// chose it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ObjectWriter {
    /// ELF or COFF bytes encoded in process ([`build_object`]); an encoder
    /// refusal is an error.
    Direct,
    /// Bytes encoded in process, except that a listing the encoder refuses
    /// is assembled.
    DirectOrAssembler,
    /// The listing assembled by the bundled clang ([`assemble`]).
    Assembler,
}

/// The object writer `--native-object` (`Direct`) or `--no-native-object`
/// (`Assembler`) asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RequestedWriter {
    Direct,
    Assembler,
}

/// The object writer for `module` on `target`. A `requested` writer is used
/// as asked, and only it makes an encoder refusal final. Otherwise both
/// Windows targets encode COFF objects directly and assemble a listing the
/// encoder refuses, as every Windows listing was assembled before direct COFF
/// objects became the default, and Linux assembles, since direct ELF objects
/// stay opt-in. The encoder reads only the emitter's own vocabulary, so a
/// module with file-scope assembly, which is arbitrary assembler input, takes
/// the assembler by default and is refused when direct objects are requested.
pub(crate) fn object_writer(
    module: &Module<'_>,
    target: Target,
    requested: Option<RequestedWriter>,
) -> Result<ObjectWriter, NativeError<'static>> {
    let encodable = module.module_asm().is_empty();
    match requested {
        | Some(RequestedWriter::Direct) if !encodable => Err(NativeError::ObjectModuleAsm),
        | Some(RequestedWriter::Direct) => Ok(ObjectWriter::Direct),
        | None if encodable && matches!(target, Target::WindowsGnu | Target::WindowsMsvc) =>
            Ok(ObjectWriter::DirectOrAssembler),
        | Some(RequestedWriter::Assembler) | None => Ok(ObjectWriter::Assembler),
    }
}

/// Builds `out_path` from the native listing `text` with `writer`. The
/// assembler behind [`ObjectWriter::DirectOrAssembler`] accepts forms the
/// encoder lacks, such as a user section name outside `[A-Za-z0-9_.$-]`, a
/// user `.pdata` section, a non-ASCII export name or more sections than a
/// regular COFF object holds. Encoding finishes before any output is opened,
/// so a refusal leaves nothing to undo.
pub(crate) fn build(
    text: &str,
    out_path: &Path,
    target: Target,
    kind: OutputKind,
    writer: ObjectWriter,
) -> io::Result<()> {
    match writer {
        | ObjectWriter::Direct => build_object(text, out_path, target, kind),
        | ObjectWriter::DirectOrAssembler => encode_object(text, out_path, target, kind, |_| {
            assemble(text, out_path, target, kind)
        }),
        | ObjectWriter::Assembler => assemble(text, out_path, target, kind),
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
    encode_object(text, out_path, target, kind, |error| {
        Err(io::Error::other(error))
    })
}

/// [`build_object`], with `refused` deciding what an encoder refusal, which
/// opened no output, returns.
fn encode_object(
    text: &str,
    out_path: &Path,
    target: Target,
    kind: OutputKind,
    refused: impl FnOnce(super::object::Error) -> io::Result<()>,
) -> io::Result<()> {
    if kind == OutputKind::SharedLibrary {
        return Err(io::Error::other(NativeError::SharedLibraryCode));
    }
    let arena = Bump::new();
    let encoded = match target {
        | Target::LinuxGnu | Target::LinuxMusl => super::object::encode(text, &arena),
        | Target::WindowsGnu | Target::WindowsMsvc =>
            super::object::encode_coff(text, target, &arena),
    };
    let bytes = match encoded {
        | Ok(bytes) => bytes,
        | Err(error) => return refused(error),
    };
    if kind == OutputKind::Object {
        return std::fs::write(out_path, &bytes);
    }
    let (object, mut file) = TemporaryObject::create(out_path, "o", &arena)?;
    file.write_all(&bytes)?;
    drop(file);
    Toolchain::bundled(target)?.assemble(object.path(), out_path, target, kind)
}
