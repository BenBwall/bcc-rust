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

#[path = "build_support/llvm.rs"]
mod llvm;
#[path = "build_support/native.rs"]
mod native;

fn main() {
    println!("Running build script...");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=float_parsing.c");
    println!("cargo:rerun-if-changed=float_parsing.h");
    println!("cargo:rerun-if-changed=build_support/native.rs");
    println!("cargo:rerun-if-changed=build_support/llvm.rs");
    native::compile(&OUT_DIR);
    #[cfg(feature = "benchmarking-internals")]
    {
        gen_one_million();
        gen_parser_mix();
    }
}

/// Number of repeated units in the generated mixed parser workload.
#[cfg(feature = "benchmarking-internals")]
const PARSER_MIX_UNITS: usize = 20_000;

/// Generates a C99 translation unit that exercises typedef-sensitive
/// declarations, aggregates, enums, initializers, function definitions,
/// statements, and expressions of varying precedence.
#[cfg(feature = "benchmarking-internals")]
fn gen_parser_mix() {
    let out_dir = &*OUT_DIR;
    let mut f = BufWriter::new(std::fs::File::create(format!("{out_dir}/parser-mix.c")).unwrap());
    for i in 0..PARSER_MIX_UNITS {
        writeln!(
            f,
            "typedef unsigned long size{i}_t;
struct node{i} {{ int value; struct node{i} *next; size{i}_t length : 8; }};
enum color{i} {{ RED{i}, GREEN{i} = 4, BLUE{i} }};
static const int table{i}[4] = {{ 1, 2, [3] = 4 }};
struct node{i} root{i} = {{ .value = {i}, .next = 0 }};
int (*callback{i})(int, char *);
static int compute{i}(size{i}_t count, const int *restrict values, struct node{i} *head)
{{
    size{i}_t index;
    int total = 0;
    for (index = 0; index < count; ++index) {{
        total += values[index] * (int)index - (total >> 2) % 7;
        if (total > 1000 && head != 0 || !(total & 1))
            total = total ? total / 2 : -1;
        else
            continue;
    }}
    while (head) {{
        switch (head->value) {{
        case RED{i}: total ^= sizeof(struct node{i}); break;
        case GREEN{i}: total |= (int)sizeof total; /* fallthrough */
        default: total <<= 1;
        }}
        head = head->next;
    }}
    do {{ total--; }} while (total > 100);
    return total + table{i}[{i} % 4] + (int)(sizeof(size{i}_t) * 2u);
}}"
        )
        .unwrap();
    }
}

#[cfg(feature = "benchmarking-internals")]
fn gen_one_million() {
    let out_dir = &*OUT_DIR;
    let mut f =
        BufWriter::new(std::fs::File::create(format!("{out_dir}/one-million-lines.c")).unwrap());
    for i in 0..1_000_000 {
        writeln!(f, "int i{i} = {i};").unwrap();
    }
}
