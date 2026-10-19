//! Independent PE/COFF parser and equivalence oracle. Compare section order,
//! every characteristic and byte, symbol types/storage classes/aux records,
//! implicit addends and relocation targets. Only file placement, string-table
//! packing and symbol ordering are normalized.

use std::{
    io,
    process::Command,
};

use crate::{
    backend::llvm::{
        OutputKind,
        Toolchain,
    },
    target::Target,
    test_support::TempDir,
    util::bump::Bump,
};

pub(crate) fn equivalent(text: &str, target: Target) -> io::Result<()> {
    let arena = Bump::new();
    let direct = super::super::object::encode_coff(text, target, &arena)
        .unwrap_or_else(|e| panic!("{e}\n{text}"));
    let directory = TempDir::new("coff-equivalence");
    let assembly = directory.join("reference.s");
    let reference = directory.join("reference.obj");
    std::fs::write(&assembly, text)?;
    // Assembly and freestanding links need the bundled tools, not a CRT.
    // The MSVC selector avoids MinGW discovery on a Linux validation host.
    let toolchain = Toolchain::bundled(Target::WindowsMsvc)?;
    toolchain.assemble(&assembly, &reference, target, OutputKind::Object)?;
    let expected = std::fs::read(&reference)?;
    assert_eq!(
        inspect(&direct),
        inspect(&expected),
        "COFF differs:\n{text}"
    );
    let native = directory.join("native.obj");
    std::fs::write(&native, &direct)?;
    let mut disassemblies = Vec::new();
    for path in [&reference, &native] {
        let output =
            super::object::required_objdump(Command::new("objdump").arg("-drw").arg(path))?;
        assert!(
            output.status.success(),
            "COFF disassembly failed: {output:?}"
        );
        // Alias order can change objdump's choice of labels. The structured
        // view compares names; here compare instruction addresses/bytes.
        let listing = String::from_utf8_lossy(&output.stdout);
        disassemblies.push(
            listing
                .lines()
                .filter_map(|line| {
                    let (address, rest) = line.trim_start().split_once(':')?;
                    if address.is_empty() || !address.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return None;
                    }
                    let bytes = rest.split('\t').find(|part| !part.trim().is_empty())?;
                    Some((address.to_owned(), bytes.trim().to_owned()))
                })
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        disassemblies[0], disassemblies[1],
        "COFF disassembly differs:\n{text}"
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct View {
    header:      (u16, u16, u32, u16, u16),
    sections:    Vec<(String, u32, u32, Vec<u8>)>,
    symbols:     Vec<SymbolView>,
    relocations: Vec<(usize, u32, u16, String, i16)>,
    comdats:     Vec<(usize, Option<String>)>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolView {
    name:        String,
    value:       u32,
    section:     i16,
    kind:        u16,
    class:       u8,
    aux:         Vec<u8>,
    weak_target: String,
}

fn inspect(bytes: &[u8]) -> View {
    let count = usize::from(u16_at(bytes, 2));
    let symtab = u32_at(bytes, 8) as usize;
    let symbol_count = u32_at(bytes, 12) as usize;
    let strtab = symtab + symbol_count * 18;
    let string_size = u32_at(bytes, strtab) as usize;
    assert_eq!(
        strtab + string_size,
        bytes.len(),
        "COFF string table extent"
    );
    let name = |data: &[u8], section: bool| {
        let start = if data[..4] == [0; 4] {
            Some(u32_at(data, 4) as usize)
        } else if section && data[0] == b'/' {
            Some(
                std::str::from_utf8(data[1..].split(|&c| c == 0).next().unwrap())
                    .unwrap()
                    .parse::<usize>()
                    .unwrap(),
            )
        } else {
            None
        };
        let data = if let Some(start) = start {
            assert!((4..string_size).contains(&start));
            &bytes[strtab + start..strtab + string_size]
        } else {
            data
        };
        String::from_utf8(data.split(|&c| c == 0).next().unwrap().to_vec()).unwrap()
    };
    let mut names = vec![None; symbol_count];
    let mut index = 0;
    while index < symbol_count {
        let at = symtab + 18 * index;
        names[index] = Some((name(&bytes[at..at + 8], false), i16_at(bytes, at + 12)));
        index += 1 + usize::from(bytes[at + 17]);
    }
    let mut view = View {
        header:      (
            u16_at(bytes, 0),
            u16_at(bytes, 2),
            u32_at(bytes, 4),
            u16_at(bytes, 16),
            u16_at(bytes, 18),
        ),
        sections:    Vec::new(),
        symbols:     Vec::new(),
        relocations: Vec::new(),
        comdats:     Vec::new(),
    };
    for id in 0..count {
        let at = 20 + id * 40;
        assert_eq!(u32_at(bytes, at + 8), 0, "COFF virtual size");
        assert_eq!(u32_at(bytes, at + 12), 0, "COFF virtual address");
        assert_eq!(u32_at(bytes, at + 28), 0, "COFF line number pointer");
        assert_eq!(u16_at(bytes, at + 34), 0, "COFF line number count");
        let size = u32_at(bytes, at + 16);
        let raw = u32_at(bytes, at + 20) as usize;
        let flags = u32_at(bytes, at + 36);
        let data = if flags & 0x80 != 0 || size == 0 {
            Vec::new()
        } else {
            assert!(raw >= 20 + count * 40);
            bytes[raw..raw + size as usize].to_vec()
        };
        view.sections
            .push((name(&bytes[at..at + 8], true), size, flags, data));
        let relocs = u32_at(bytes, at + 24) as usize;
        for n in 0..usize::from(u16_at(bytes, at + 32)) {
            let at = relocs + n * 10;
            let (symbol, section) = names[u32_at(bytes, at + 4) as usize].as_ref().unwrap();
            view.relocations.push((
                id,
                u32_at(bytes, at),
                u16_at(bytes, at + 8),
                symbol.clone(),
                *section,
            ));
        }
    }
    index = 0;
    while index < symbol_count {
        let at = symtab + index * 18;
        let aux_count = usize::from(bytes[at + 17]);
        let mut aux = bytes[at + 18..at + 18 * (aux_count + 1)].to_vec();
        let mut target = String::new();
        if bytes[at + 16] == 105 {
            assert_eq!(aux_count, 1);
            target = format!("{:?}", names[u32_at(&aux, 0) as usize].as_ref().unwrap());
            aux[..4].fill(0);
        }
        view.symbols.push(SymbolView {
            name: name(&bytes[at..at + 8], false),
            value: u32_at(bytes, at + 8),
            section: i16_at(bytes, at + 12),
            kind: u16_at(bytes, at + 14),
            class: bytes[at + 16],
            aux,
            weak_target: target,
        });
        index += aux_count + 1;
    }
    for (id, section) in view.sections.iter().enumerate() {
        if section.2 & 0x1000 == 0 {
            continue;
        }
        let number = i16::try_from(id + 1).unwrap();
        let definition = view
            .symbols
            .iter()
            .find(|s| s.section == number && s.class == 3 && s.aux.len() == 18)
            .unwrap();
        if definition.aux[14] != 5 {
            // Symbol order may vary, but the first non-section symbol in a
            // COMDAT must still identify the same linker selection key.
            let key = view
                .symbols
                .iter()
                .find(|s| s.section == number && s.aux.is_empty());
            view.comdats.push((id, key.map(|s| s.name.clone())));
        }
    }
    view.symbols.sort();
    view
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[test]
fn coff_unwind_comdats_refptr_exports_and_tls_match_clang() {
    let text = ".text\n.def f;.scl 2;.type 32;.endef\n.globl f\nf:\n.seh_proc f\npushq \
                %rbx\n.seh_pushreg %rbx\nsubq $48,%rsp\n.seh_stackalloc 48\nmovaps \
                %xmm6,32(%rsp)\n.seh_savexmm %xmm6,32\n.seh_endprologue\nmovq \
                .refptr.ext(%rip),%rax\nmovl _tls_index(%rip),%ecx\nmovq %gs:88,%rax\nleaq \
                tls@SECREL32(%rax),%rdx\nmovl $42,ext(%rip)\nretq\n.seh_endproc\n.section \
                .rdata$.refptr.ext,\"dr\",discard,.refptr.ext\n.globl \
                .refptr.ext\n.refptr.ext:\n.quad ext\n.section \
                .data,\"dw\",discard,choice\n.globl choice\nchoice:\n.long 1\n.section \
                .tls$,\"dw\"\ntls:\n.long 3\n.section .drectve,\"yni\"\n.ascii \" \
                /EXPORT:f\"\n.section .ctors,\"dw\"\n.p2align 3\n.quad f\n";
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        equivalent(text, target).unwrap();
    }
}

#[test]
fn coff_symbol_resolution_and_all_relocation_widths_match_clang() {
    for text in [
        ".def f;.scl 3;.type 32;.endef\nf:\nretq\n.def alias;.scl 2;.type 32;.endef\n.weak \
         alias\nalias=f\n.quad alias\n",
        ".data\n.quad 0\n.La:\n.quad 0\n.secidx .La\n.secrel32 .La\n.rva .La\n.quad .La+3\n",
        ".def \"?overload@@YAMM@Z\";.scl 3;.type 32;.endef\n\"?overload@@YAMM@Z\":\nretq\ncallq \
         \"?overload@@YAMM@Z\"\nleaq \"?overload@@YAMM@Z\"(%rip),%rax\n",
        ".def f;.scl 2;.type 32;.endef\n.globl f\nf:\nretq\n.def alias;.scl 2;.type \
         32;.endef\n.weak alias\nalias = f\ncallq alias\n.quad alias\n",
        ".def z;.scl 2;.type 32;.endef\n.weak z\nz:\nretq\n.quad z\n",
        ".def f;.scl 2;.type 32;.endef\n.globl f\nf:\nretq\n.globl alias\nalias = f\ncallq \
         alias\n.quad alias\n",
        ".text\n.globl f\nf:\njmp f\nje f\ncallq f\nleaq f(%rip),%rax\nlocal:\ncallq local\njmp \
         local\n.data\na:\n.quad a\n.long a\n.rva a\n.secrel32 a\n.secidx a\n",
        ".def f;.scl 2;.type 32;.endef\n.globl f\nf:\njmp f\nje f\ncallq f\nleaq \
         f(%rip),%rax\n.data\na:\n.quad .La+3\n.La:\n.quad 1\n.text\nleaq .La(%rip),%rax\n",
        ".weak w\n.globl f\nf:\nretq\n.weak z\nz:\nretq\n.quad w\n",
        ".text\n.seh_proc f\nf:\n.seh_endprologue\nretq\n.seh_endproc\n.section \
         .text$g,\"xr\",discard,g\n.globl g\ng:\n.seh_proc \
         g\n.seh_endprologue\nretq\n.seh_endproc\n",
        ".section .text,\"xr\",discard,g\n.globl g\ng:\n.seh_proc \
         g\n.seh_endprologue\nretq\n.seh_endproc\n.section .rdata,\"dr\"\n.quad \
         g\n.text\nf:\n.seh_proc f\n.seh_endprologue\nretq\n.seh_endproc\n",
    ] {
        for target in [Target::WindowsGnu, Target::WindowsMsvc] {
            equivalent(text, target).unwrap();
        }
    }
}

#[test]
fn coff_multiple_comdat_unwind_sections_match_clang() {
    let text = ".section .text$x,\"xr\",discard,x\n.globl x\nx:\n.seh_proc \
                x\n.seh_endprologue\nretq\n.seh_endproc\n.section \
                .text$y,\"xr\",discard,y\n.globl y\ny:\n.seh_proc \
                y\n.seh_endprologue\nretq\n.seh_endproc\n";
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        equivalent(text, target).unwrap();
    }
}

#[test]
fn coff_comdat_and_ordinary_unwind_sections_match_clang() {
    let text = ".section .text$x,\"xr\",discard,x\n.globl x\nx:\n.seh_proc \
                x\n.seh_endprologue\nretq\n.seh_endproc\n.text\nf:\n.seh_proc \
                f\n.seh_endprologue\nretq\n.seh_endproc\n.section \
                .text$y,\"xr\",discard,y\n.globl y\ny:\n.seh_proc \
                y\n.seh_endprologue\nretq\n.seh_endproc\n.text\ng:\n.seh_proc \
                g\n.seh_endprologue\nretq\n.seh_endproc\n";
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        equivalent(text, target).unwrap();
    }
}

#[test]
fn coff_large_and_framed_unwind_match_clang() {
    for offset in [16, 524_272, 524_288, 1_048_560, 1_048_576] {
        let text = format!(
            "f:\n.seh_proc f\nmovq %rbx,{offset}(%rsp)\n.seh_savereg %rbx,{offset}\nmovaps \
             %xmm12,{offset}(%rsp)\n.seh_savexmm \
             %xmm12,{offset}\n.seh_endprologue\nretq\n.seh_endproc\n"
        );
        for target in [Target::WindowsGnu, Target::WindowsMsvc] {
            equivalent(&text, target).unwrap();
        }
    }
    for size in [8, 128, 136, 65_536, 524_280, 524_288] {
        let text = format!(
            "f:\n.seh_proc f\npushq %rbp\n.seh_pushreg %rbp\nsubq ${size},%rsp\n.seh_stackalloc \
             {size}\nmovq %rsp,%rbp\n.seh_setframe %rbp,0\nmovaps %xmm15,16(%rsp)\n.seh_savexmm \
             %xmm15,16\n.seh_endprologue\nretq\n.seh_endproc\n"
        );
        equivalent(&text, Target::WindowsMsvc).unwrap();
    }
}

#[test]
fn coff_rejects_unknown_or_unrepresentable_input_before_output() {
    for text in [
        ".seh_handler f,@except",
        ".seh_endproc",
        ".seh_proc f\nretq",
        ".def f",
        ".scl 2",
        ".section .pdata,\"dr\"",
        ".p2align 14",
        ".bss\n.zero 4294967296",
        ".data\n.zero 4294967296",
        ".byte ext",
        "movq ext@GOTPCREL(%rip),%rax",
        ".seh_proc f\n.seh_stackalloc 7\n.seh_endproc",
        ".section x,\"dw\",invalid,s",
        ".ascii \"\\q\"",
        ".section x,\"dw\",discard,s\n.globl s\ns:\n.section .data\ns=ext",
    ] {
        let arena = Bump::new();
        assert!(
            super::super::object::encode_coff(text, Target::WindowsMsvc, &arena).is_err(),
            "accepted {text}"
        );
    }
}

#[test]
fn coff_equivalence_detects_metadata_and_unwind_mutations() {
    let arena = Bump::new();
    let bytes = super::super::object::encode_coff(
        ".globl f\nf:\n.seh_proc f\n.seh_endprologue\nretq\n.seh_endproc\n.section \
         .data,\"dw\",discard,g\n.globl g\ng:\n.quad f\n",
        Target::WindowsMsvc,
        &arena,
    )
    .unwrap();
    let original = inspect(&bytes);
    let symtab = u32_at(&bytes, 8) as usize;
    let xdata = original
        .sections
        .iter()
        .position(|s| s.0 == ".xdata")
        .unwrap();
    let xraw = u32_at(&bytes, 20 + xdata * 40 + 20) as usize;
    for at in [
        0,
        4,
        18,
        20 + 36,
        symtab + 18 + 8,
        symtab + 18 + 14,
        xraw + 1,
    ] {
        let mut changed = bytes.to_vec();
        changed[at] ^= 1;
        assert_ne!(inspect(&changed), original, "mutation {at}");
    }
}

#[test]
fn coff_equivalence_detects_comdat_key_order_changes() {
    let arena = Bump::new();
    let bytes = super::super::object::encode_coff(
        ".section .data,\"dw\",discard,key\n.globl key\nkey:\n.long 1\n.globl \
         other\nother:\n.long 2\n",
        Target::WindowsMsvc,
        &arena,
    )
    .unwrap();
    let original = inspect(&bytes);
    let symbols = u32_at(&bytes, 8) as usize;
    let count = usize::from(u16_at(&bytes, 2));
    let first = symbols + count * 2 * 18;
    let second = first + 18;
    let mut changed = bytes.to_vec();
    changed[first..second].copy_from_slice(&bytes[second..second + 18]);
    changed[second..second + 18].copy_from_slice(&bytes[first..second]);
    let reordered = inspect(&changed);
    assert_eq!(reordered.symbols, original.symbols);
    assert_ne!(reordered.comdats, original.comdats);
}

#[test]
fn coff_programs_match_both_targets_at_b0_and_b1() {
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        for optimize in [false, true] {
            for program in super::programs::PROGRAMS
                .iter()
                .chain(super::programs::WIDE_FLOAT_PROGRAMS)
                .chain([&super::programs::SSE_UNSIGNED_CONVERSIONS])
            {
                super::end_to_end::with_module(program.source, target, optimize, |module| {
                    let mut text = String::new();
                    super::super::emit_assembly(module, target, &mut text).unwrap();
                    equivalent(&text, target).unwrap();
                });
            }
        }
    }
}

#[test]
fn coff_direct_objects_link_with_both_bundled_windows_linkers() {
    let directory = TempDir::new("coff-link");
    let text = ".section .text$entry,\"xr\",discard,entry\n.def entry;.scl 2;.type \
                32;.endef\n.globl entry\nentry:\n.seh_proc entry\nsubq $40,%rsp\n.seh_stackalloc \
                40\n.seh_endprologue\ncallq helper\naddq $40,%rsp\nretq\n.seh_endproc\n.section \
                .text$helper,\"xr\",discard,helper\n.def helper;.scl 2;.type 32;.endef\n.globl \
                helper\nhelper:\n.seh_proc helper\n.seh_endprologue\nmovl \
                choice(%rip),%eax\nretq\n.seh_endproc\n.section \
                .data,\"dw\",discard,choice\n.globl choice\nchoice:\n.long 42\n";
    let second = ".section .data,\"dw\",discard,choice\n.globl choice\nchoice:\n.long 99\n";
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        equivalent(text, target).unwrap();
        let arena = Bump::new();
        let first = directory.join("first.obj");
        let other = directory.join("other.obj");
        let executable = directory.join("linked.exe");
        std::fs::write(
            &first,
            super::super::object::encode_coff(text, target, &arena).unwrap(),
        )
        .unwrap();
        std::fs::write(
            &other,
            super::super::object::encode_coff(second, target, &arena).unwrap(),
        )
        .unwrap();
        let toolchain = Toolchain::bundled(Target::WindowsMsvc).unwrap();
        let gnu = target == Target::WindowsGnu;
        let name = match (gnu, cfg!(windows)) {
            | (true, false) => "ld.lld",
            | (true, true) => "ld.lld.exe",
            | (false, false) => "lld-link",
            | (false, true) => "lld-link.exe",
        };
        let mut command = Command::new(toolchain.clang().with_file_name(name));
        if gnu {
            _ = command
                .args([
                    "-m",
                    "i386pep",
                    "--entry",
                    "entry",
                    "--subsystem",
                    "console",
                    "--no-insert-timestamp",
                    "-o",
                ])
                .arg(&executable);
        } else {
            _ = command
                .args([
                    "/entry:entry",
                    "/subsystem:console",
                    "/nodefaultlib",
                    "/timestamp:0",
                ])
                .arg(format!("/out:{}", executable.display()));
        }
        let result = command.arg(&first).arg(&other).output().unwrap();
        assert!(result.status.success(), "{target:?}: {result:?}");
        let image = std::fs::read(&executable).unwrap();
        assert_eq!(&image[..2], b"MZ");
        // The first selectany definition survives into the linked image.
        let pe = u32_at(&image, 0x3C) as usize;
        // Both COMDAT procedures retain a RUNTIME_FUNCTION record in the
        // linked image's exception directory (PE32+ data directory 3).
        assert_eq!(u32_at(&image, pe + 24 + 112 + 3 * 8 + 4), 24);
        let count = usize::from(u16_at(&image, pe + 6));
        let sections = pe + 24 + usize::from(u16_at(&image, pe + 20));
        let data = (0..count)
            .map(|n| sections + 40 * n)
            .find(|&at| image[at..at + 8].starts_with(b".data\0"))
            .unwrap();
        let raw = u32_at(&image, data + 20) as usize;
        assert_eq!(u32_at(&image, raw), 42);
    }
}

#[test]
fn coff_quoted_exports_and_constructor_sections_match_clang() {
    let text = r#".globl "f=bar;baz,qux"
.def "f=bar;baz,qux"; .scl 2; .type 32; .endef
"f=bar;baz,qux":
retq
.section .drectve,"yni"
.ascii " /EXPORT:\"f=bar;baz,qux\""
.section .CRT$XCU,"dr"
.p2align 3
.quad "f=bar;baz,qux"
.section .dtors.00101,"dw"
.p2align 3
.quad "f=bar;baz,qux"
"#;
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        equivalent(text, target).unwrap();
    }
}
