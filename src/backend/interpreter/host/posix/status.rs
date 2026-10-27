//! `struct stat` as glibc lays it out on x86-64 (`sysdeps/unix/sysv/linux/
//! x86/bits/struct_stat.h`, 144 bytes, the same as `struct stat64`), filled
//! from a host file's metadata or for the console, which the interpreter
//! presents as a pipe (or a terminal for `--interpret-terminal-stdout`).

use std::fs::Metadata;

use crate::{
    backend::interpreter::{
        memory::Memory,
        trap::Fault,
        value::RuntimeValue,
    },
    ir::{
        Align,
        Type,
    },
};

/// The fields of a `struct stat`.
#[derive(Clone, Copy, Default, Debug)]
pub(in crate::backend::interpreter::host) struct Status {
    pub(in crate::backend::interpreter::host) device:     u64,
    pub(in crate::backend::interpreter::host) inode:      u64,
    pub(in crate::backend::interpreter::host) links:      u64,
    pub(in crate::backend::interpreter::host) mode:       u32,
    pub(in crate::backend::interpreter::host) user:       u32,
    pub(in crate::backend::interpreter::host) group:      u32,
    pub(in crate::backend::interpreter::host) rdev:       u64,
    pub(in crate::backend::interpreter::host) size:       i64,
    pub(in crate::backend::interpreter::host) block_size: i64,
    pub(in crate::backend::interpreter::host) blocks:     i64,
    /// Access, modification and status change times: seconds and
    /// nanoseconds.
    pub(in crate::backend::interpreter::host) times:      [(i64, i64); 3],
}

/// `S_IFIFO` and `S_IFCHR`.
pub(in crate::backend::interpreter::host) const FIFO: u32 = 0o010_000;
pub(in crate::backend::interpreter::host) const CHARACTER_DEVICE: u32 = 0o020_000;

impl Status {
    /// Writes the structure to `buffer`, with glibc's padding (`__pad0`
    /// and `__glibc_reserved`) zero.
    pub(in crate::backend::interpreter::host) fn store(
        &self,
        memory: &mut Memory<'_>,
        buffer: RuntimeValue,
    ) -> Result<(), Fault> {
        let fields: [(u64, Type, u128); 19] = [
            (0, Type::I64, self.device.into()),
            (8, Type::I64, self.inode.into()),
            (16, Type::I64, self.links.into()),
            (24, Type::I32, self.mode.into()),
            (28, Type::I32, self.user.into()),
            (32, Type::I32, self.group.into()),
            (36, Type::I32, 0),
            (40, Type::I64, self.rdev.into()),
            (48, Type::I64, signed(self.size)),
            (56, Type::I64, signed(self.block_size)),
            (64, Type::I64, signed(self.blocks)),
            (72, Type::I64, signed(self.times[0].0)),
            (80, Type::I64, signed(self.times[0].1)),
            (88, Type::I64, signed(self.times[1].0)),
            (96, Type::I64, signed(self.times[1].1)),
            (104, Type::I64, signed(self.times[2].0)),
            (112, Type::I64, signed(self.times[2].1)),
            (120, Type::I64, 0),
            (128, Type::I64, 0),
        ];
        for (offset, ty, value) in fields.into_iter().chain([(136, Type::I64, 0)]) {
            let at = memory.ptr_add(buffer, RuntimeValue::Int(offset.into()), true);
            memory.store(ty, RuntimeValue::int(ty, value), at, Align::BYTE)?;
        }
        Ok(())
    }

    /// The status of a host file.
    pub(in crate::backend::interpreter::host) fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Self {
                device:     metadata.dev(),
                inode:      metadata.ino(),
                links:      metadata.nlink(),
                mode:       metadata.mode(),
                user:       metadata.uid(),
                group:      metadata.gid(),
                rdev:       metadata.rdev(),
                size:       metadata.size().cast_signed(),
                block_size: metadata.blksize().cast_signed(),
                blocks:     metadata.blocks().cast_signed(),
                times:      [
                    (metadata.atime(), metadata.atime_nsec()),
                    (metadata.mtime(), metadata.mtime_nsec()),
                    (metadata.ctime(), metadata.ctime_nsec()),
                ],
            }
        }
        #[cfg(not(unix))]
        {
            // Windows keeps no POSIX modes: a regular file or a directory
            // with the permissions a umask of 022 leaves.
            let mode = if metadata.is_dir() {
                0o040_755
            } else {
                0o100_644
            };
            Self {
                links: 1,
                mode,
                size: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                block_size: 4096,
                ..Self::default()
            }
        }
    }

    /// The status the host's `fstatat` reported.
    #[cfg(target_os = "linux")]
    #[cfg_attr(
        target_arch = "x86_64",
        expect(
            clippy::useless_conversion,
            reason = "`st_nlink` and `st_blksize` are narrower on other Linux architectures."
        )
    )]
    pub(in crate::backend::interpreter::host) fn of_host(raw: &libc::stat) -> Self {
        Self {
            device:     raw.st_dev,
            inode:      raw.st_ino,
            links:      u64::from(raw.st_nlink),
            mode:       raw.st_mode,
            user:       raw.st_uid,
            group:      raw.st_gid,
            rdev:       raw.st_rdev,
            size:       raw.st_size,
            block_size: i64::from(raw.st_blksize),
            blocks:     raw.st_blocks,
            times:      [
                (raw.st_atime, raw.st_atime_nsec),
                (raw.st_mtime, raw.st_mtime_nsec),
                (raw.st_ctime, raw.st_ctime_nsec),
            ],
        }
    }

    /// The status of the console's output or error: a pipe of the buffer
    /// size glibc would allocate for it, or a terminal, owned by the user
    /// the interpreter runs as.
    pub(in crate::backend::interpreter::host) fn console(
        terminal: bool,
        block_size: usize,
    ) -> Self {
        let (mode, rdev) = if terminal {
            (CHARACTER_DEVICE | 0o620, 0x8800)
        } else {
            (FIFO | 0o600, 0)
        };
        let (user, group) = user_and_group();
        Self {
            links: 1,
            mode,
            user,
            group,
            rdev,
            block_size: i64::try_from(block_size).unwrap_or(i64::MAX),
            ..Self::default()
        }
    }
}

/// A signed field's two's-complement bits.
fn signed(value: i64) -> u128 {
    u128::from(value.cast_unsigned())
}

/// The real user and group the interpreter runs as.
fn user_and_group() -> (u32, u32) {
    #[cfg(unix)]
    {
        // SAFETY: `getuid` always succeeds.
        let user = unsafe { libc::getuid() };
        // SAFETY: `getgid` always succeeds.
        let group = unsafe { libc::getgid() };
        (user, group)
    }
    #[cfg(not(unix))]
    {
        (0, 0)
    }
}
