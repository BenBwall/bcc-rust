//! `bcc --msvc-mode` against the real Microsoft toolchain on a Windows host.
//!
//! These tests run only on Windows with Visual Studio or its Build Tools
//! installed; elsewhere, and on a Windows host without them, each says why it
//! skipped and passes. `toolchain` finds the installation with `vswhere` and
//! takes the environment its `vcvars64.bat` sets (`INCLUDE`, `LIB`, `PATH`),
//! which `cl.exe` needs as much as bcc does. Its ignored `probe` reports what
//! the host has: run it with
//! `cargo test --test integration msvc::toolchain::probe -- --ignored
//! --nocapture`.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests own fixtures, environments and captured process output."
)]

#[cfg(test)]
mod cli;
#[cfg(test)]
mod crt;
#[cfg(test)]
mod interop;
#[cfg(test)]
mod macros;
#[cfg(test)]
mod toolchain;
