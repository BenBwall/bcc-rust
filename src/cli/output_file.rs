//! Protects the source from conflicting file output before code generation.
//! File identities include hard links and followed symbolic links, unlike
//! path spelling comparisons. This is a host CLI policy, not a C constraint.

#[cfg(windows)]
use std::fs::File;
use std::{
    fs,
    io,
    path::Path,
};

/// Whether an existing output identifies the input. A new output is allowed;
/// other inspection errors stop compilation rather than risk truncating input.
/// Only regular files can be compared: opening a FIFO or device to read its
/// identity could block, and writing to one cannot truncate the source.
pub(super) fn is_input(input: &Path, output: &Path) -> io::Result<bool> {
    let output_metadata = match fs::metadata(output) {
        | Ok(metadata) => metadata,
        | Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        | Err(error) => return Err(error),
    };
    let input_metadata = fs::metadata(input)?;
    if !output_metadata.is_file() || !input_metadata.is_file() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        Ok((input_metadata.dev(), input_metadata.ino())
            == (output_metadata.dev(), output_metadata.ino()))
    }
    #[cfg(windows)]
    {
        Ok(identity(&File::open(input)?)? == identity(&File::open(output)?)?)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(input.canonicalize()? == output.canonicalize()?)
    }
}

/// A failed object compile removes stale output too, after `is_input` has
/// protected the source and its aliases. Devices, directories and FIFOs are
/// not object files; inspecting them never opens a potentially blocking file.
/// This is the CLI compile-job policy, not a backend API ownership rule.
pub(super) fn remove_failed_object(output: &Path, arena: &crate::util::bump::Bump) {
    let result = fs::metadata(output).and_then(|metadata| {
        if metadata.is_file() && can_write(output, &metadata, arena) {
            fs::remove_file(output)
        } else {
            Ok(())
        }
    });
    if let Err(error) = result
        && error.kind() != io::ErrorKind::NotFound
    {
        drop(io::Write::write_fmt(
            &mut io::stderr().lock(),
            format_args!(
                "error: cannot remove failed output '{}': {error}\n",
                output.display()
            ),
        ));
    }
}

/// Checks writability before unlinking, following LLVM output cleanup even
/// when compilation failed before opening its output. POSIX access follows
/// symlinks and checks the caller's permissions (including ACLs), rather than
/// mode bits alone.
fn can_write(output: &Path, metadata: &fs::Metadata, arena: &crate::util::bump::Bump) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        let _ = metadata;
        let bytes = output.as_os_str().as_bytes();
        if bytes.contains(&0) {
            return false;
        }
        let path = arena.alloc_slice_fill_iter(bytes.iter().copied().chain([0]));
        // SAFETY: path is NUL terminated and stays alive throughout access.
        unsafe { libc::access(path.as_ptr().cast(), libc::W_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = (output, arena);
        !metadata.permissions().readonly()
    }
}

/// Windows' stable Rust metadata API does not expose file identity yet.
/// Read the volume serial and 64-bit file index from the open handle instead.
#[cfg(windows)]
fn identity(file: &File) -> io::Result<(u64, u64)> {
    use std::{
        mem::MaybeUninit,
        os::windows::io::AsRawHandle as _,
    };

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION,
        GetFileInformationByHandle,
    };

    let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: `file` owns a live handle for the duration of the call, and
    // `info` is writable storage of the API's exact output type.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful call initializes the entire information structure.
    let info = unsafe { info.assume_init() };
    Ok((
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}
