//! Build script for the BCC C compiler.

use std::env::var;

fn main() {
    println!("Running build script...");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=float_parsing.c");
    println!("cargo:rerun-if-changed=float_parsing.h");
    let out_dir = var("OUT_DIR").unwrap();
    cc::Build::new()
        .file("float_parsing.c")
        .opt_level(3)
        .debug(true)
        .out_dir(&out_dir)
        .compile("lexer");
    bindgen::Builder::default()
        .header("float_parsing.h")
        .allowlist_file("float_parsing.h")
        .allowlist_item("ERANGE")
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(format!("{out_dir}/bindings.rs"))
        .expect("Couldn't write bindings!");
}
