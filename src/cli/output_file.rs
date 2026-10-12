//! Protects the source from conflicting file output before code generation.
//! File identities include hard links and followed symbolic links, unlike
//! path spelling comparisons. This is a host CLI policy, not a C constraint.

use std::{
    fs::File,
    io,
    path::Path,
};

/// Whether an existing output identifies the input. A new output is allowed;
/// other inspection errors stop compilation rather than risk truncating input.
pub(super) fn is_input(input: &Path, output: &Path) -> io::Result<bool> {
    let output_file = match File::open(output) {
        | Ok(file) => file,
        | Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        | Err(error) => return Err(error),
    };
    let input_file = File::open(input)?;
    #[cfg(any(unix, windows))]
    {
        Ok(identity(&input_file)? == identity(&output_file)?)
    }
    #[cfg(not(any(unix, windows)))]
    {
        drop((input_file, output_file));
        Ok(input.canonicalize()? == output.canonicalize()?)
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

/// Unix identifies an open file by its device and inode.
#[cfg(unix)]
fn identity(file: &File) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}
