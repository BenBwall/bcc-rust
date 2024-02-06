//! Build script for the BCC C compiler.

use std::{
    env::var,
    io::{
        BufWriter,
        Write,
    },
    sync::OnceLock,
};

static OUT_DIR: OnceLock<String> = OnceLock::new();

fn out_dir() -> &'static str {
    OUT_DIR.get_or_init(|| var("OUT_DIR").unwrap()).as_str()
}

fn main() {
    println!("Running build script...");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=float_parsing.c");
    println!("cargo:rerun-if-changed=float_parsing.h");
    let out_dir = out_dir();
    cc::Build::new()
        .file("float_parsing.c")
        .opt_level(3)
        .debug(true)
        .out_dir(out_dir)
        .compile("float_parsing");
    bindgen::Builder::default()
        .header("float_parsing.h")
        .allowlist_file("float_parsing.h")
        .allowlist_item("ERANGE")
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(format!("{out_dir}/bindings.rs"))
        .expect("Couldn't write bindings!");
    gen_one_million();
}

fn gen_one_million() {
    let out_dir = out_dir();
    let mut f =
        BufWriter::new(std::fs::File::create(format!("{out_dir}/one-million-lines.c")).unwrap());
    for i in 0..1_000_000 {
        writeln!(f, "int i = {i}").unwrap();
    }
}
