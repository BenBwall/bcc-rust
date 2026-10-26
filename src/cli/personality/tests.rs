use std::{
    ffi::OsString,
    path::Path,
};

use super::*;

#[test]
fn executable_names_select_personalities() {
    let named = |name: &str| Personality::from_program(Path::new(name));
    assert_eq!(named("gcc"), Some((Personality::Gcc, None)));
    assert_eq!(named("/usr/bin/cc"), Some((Personality::Gcc, None)));
    assert_eq!(named("gcc-14"), Some((Personality::Gcc, None)));
    assert_eq!(named("bcc-gcc"), Some((Personality::Gcc, None)));
    assert_eq!(named("clang-18.1"), Some((Personality::Clang, None)));
    assert_eq!(named("CL.EXE"), Some((Personality::Msvc, None)));
    assert_eq!(named("clang-cl"), Some((Personality::ClangCl, None)));
    assert_eq!(named("bcc-clang-cl"), Some((Personality::ClangCl, None)));
    assert_eq!(named("tcc"), Some((Personality::Tcc, None)));
    assert_eq!(
        named("x86_64-w64-mingw32-gcc"),
        Some((Personality::Gcc, Some(Target::WindowsGnu)))
    );
    assert_eq!(
        named("x86_64-linux-musl-gcc"),
        Some((Personality::Gcc, Some(Target::LinuxMusl)))
    );
    assert_eq!(named("bcc-rust"), None);
    assert_eq!(named("bcc"), None);
}

#[test]
fn mode_flags_select_personalities_anywhere() {
    let mut arguments: Vec<OsString> = ["bcc-rust", "-c", "--msvc-mode", "x.c"]
        .map(Into::into)
        .into();
    assert_eq!(select(&mut arguments), Some(Personality::Msvc));
    assert_eq!(arguments, ["bcc-rust", "-c", "x.c"].map(OsString::from));
    let mut arguments: Vec<OsString> = ["bcc-rust", "--driver-mode=clang-cl"]
        .map(Into::into)
        .into();
    assert_eq!(select(&mut arguments), Some(Personality::ClangCl));
    let mut arguments: Vec<OsString> = ["bcc-rust", "x.c"].map(Into::into).into();
    assert_eq!(select(&mut arguments), None);
}
