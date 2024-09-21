#![expect(
    missing_docs,
    reason = "We don't have a doc string here because it's obvious what a build script does."
)]

#[cfg(feature = "benchmarking-internals")]
use std::io::{
    BufWriter,
    Write,
};
use std::{
    env::var,
    sync::LazyLock,
};

static OUT_DIR: LazyLock<String> = LazyLock::new(|| var("OUT_DIR").unwrap());

fn main() {
    println!("Running build script...");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=float_parsing.c");
    println!("cargo:rerun-if-changed=float_parsing.h");
    cc::Build::new()
        .file("float_parsing.c")
        .opt_level(3)
        .debug(true)
        .out_dir(&*OUT_DIR)
        .compile("float_parsing");
    bindgen::Builder::default()
        .header("float_parsing.h")
        .allowlist_file("float_parsing.h")
        .allowlist_item("ERANGE")
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(format!("{}/bindings.rs", &*OUT_DIR))
        .expect("Couldn't write bindings!");
    #[cfg(feature = "benchmarking-internals")]
    {
        gen_one_million();
    }
}

#[cfg(feature = "benchmarking-internals")]
fn gen_one_million() {
    let out_dir = out_dir();
    let mut f =
        BufWriter::new(std::fs::File::create(format!("{out_dir}/one-million-lines.c")).unwrap());
    for i in 0..1_000_000 {
        writeln!(f, "int i{i} = {i};").unwrap();
    }
}
