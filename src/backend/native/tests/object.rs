//! Byte-level ELF equivalence against the pinned clang assembler. Every
//! native assembly/run helper and the native difftest oracle uses this check.
//! Section header order and metadata, contents, symbol properties, COMDAT
//! membership and RELA tuples must agree. File offsets, string-table packing
//! and symbol order within the local/global partitions may differ.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
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

pub(crate) fn equivalent(text: &str) -> io::Result<()> {
    let arena = Bump::new();
    let direct = super::super::object::encode(text, &arena)
        .unwrap_or_else(|error| panic!("{error}\n{text}"));
    let directory = TempDir::new("native-object-equivalence");
    let assembly = directory.path().join("reference.s");
    let reference = directory.path().join("reference.o");
    let native = directory.path().join("native.o");
    std::fs::write(&assembly, text)?;
    let toolchain = Toolchain::bundled(Target::LinuxGnu)?;
    toolchain.assemble(&assembly, &reference, Target::LinuxGnu, OutputKind::Object)?;
    let reference_bytes = std::fs::read(&reference)?;
    let expected = inspect(&reference_bytes);
    let actual = inspect(&direct);
    assert_equivalent(&actual, &expected, text);
    // Raw executable-section equality above also pins every encoded
    // instruction. Require objdump's independent disassembly as well.
    std::fs::write(&native, &direct)?;
    let llvm = toolchain.clang().with_file_name(if cfg!(windows) {
        "llvm-objdump.exe"
    } else {
        "llvm-objdump"
    });
    let objdump = if llvm.is_file() {
        llvm
    } else {
        "objdump".into()
    };
    let expected = required_objdump(Command::new(&objdump).arg("-dr").arg(&reference))?;
    {
        assert!(
            expected.status.success(),
            "reference disassembly failed: {expected:?}"
        );
        let actual = required_objdump(Command::new(&objdump).arg("-dr").arg(&native))?;
        assert!(
            actual.status.success(),
            "direct disassembly failed: {actual:?}"
        );
        let normalize = |bytes: &[u8]| {
            String::from_utf8_lossy(bytes)
                .lines()
                .filter(|line| !line.contains("file format"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(
            normalize(&actual.stdout),
            normalize(&expected.stdout),
            "disassembly differs:\n{text}"
        );
    }
    Ok(())
}

fn assert_equivalent(actual: &View, expected: &View, text: &str) {
    assert_eq!(
        actual.elf_header, expected.elf_header,
        "ELF headers differ:\n{text}"
    );
    assert_eq!(
        actual.headers, expected.headers,
        "section headers differ:\n{text}"
    );
    assert_eq!(
        actual.symbols, expected.symbols,
        "symbol tables differ:\n{text}"
    );
    assert_eq!(
        actual.relocations, expected.relocations,
        "relocations differ:\n{text}"
    );
    assert_eq!(
        actual.groups, expected.groups,
        "COMDAT groups differ:\n{text}"
    );
    assert_eq!(
        actual.sections.keys().collect::<Vec<_>>(),
        expected.sections.keys().collect::<Vec<_>>(),
        "section sets differ:\n{text}"
    );
    for (name, actual) in &actual.sections {
        let expected = &expected.sections[name];
        assert_eq!(
            (actual.0, actual.1, actual.2, actual.4),
            (expected.0, expected.1, expected.2, expected.4),
            "section metadata differs for {name}:\n{text}"
        );
        let difference = actual.3.iter().zip(&expected.3).position(|(a, b)| a != b);
        assert!(
            difference.is_none() && actual.3.len() == expected.3.len(),
            "section bytes differ for {name} at {difference:?}: direct={:?} reference={:?}\n{text}",
            difference.map(|at| &actual.3[at.saturating_sub(8)..(at + 16).min(actual.3.len())]),
            difference.map(|at| &expected.3[at.saturating_sub(8)..(at + 16).min(expected.3.len())])
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
struct View {
    elf_header:  [u8; 64],
    headers:     Vec<SectionHeader>,
    sections:    BTreeMap<String, (u32, u64, u64, Vec<u8>, u64)>,
    symbols:     Vec<(String, u8, u8, String, u64, u64)>,
    relocations: Vec<(String, u64, u32, String, i64)>,
    groups:      Vec<(String, Vec<String>)>,
}

#[derive(Debug, PartialEq, Eq)]
struct SectionHeader {
    name:       String,
    kind:       u32,
    flags:      u64,
    address:    u64,
    size:       u64,
    link:       String,
    info:       String,
    align:      u64,
    entry_size: u64,
}

fn inspect(bytes: &[u8]) -> View {
    assert_eq!(&bytes[..7], b"\x7fELF\x02\x01\x01");
    let mut elf_header: [u8; 64] = bytes[..64].try_into().unwrap();
    // String-table packing and section placement can change e_shoff. All
    // other header fields, including sizes, counts and e_shstrndx, must match.
    elf_header[40..48].fill(0);
    let at = usize::try_from(u64_at(bytes, 40)).unwrap();
    let count = usize::from(u16_at(bytes, 60));
    let headers = (0..count)
        .map(|i| &bytes[at + i * 64..at + (i + 1) * 64])
        .collect::<Vec<_>>();
    let shstrings = section_data(bytes, headers[usize::from(u16_at(bytes, 62))]);
    let mut names = headers
        .iter()
        .map(|h| cstr(shstrings, usize::try_from(u32_at(h, 0)).unwrap()).to_owned())
        .collect::<Vec<_>>();
    let mut view = View {
        elf_header,
        headers: Vec::new(),
        sections: BTreeMap::new(),
        symbols: Vec::new(),
        relocations: Vec::new(),
        groups: Vec::new(),
    };
    let symtab = headers.iter().position(|h| u32_at(h, 4) == 2).unwrap();
    let strings = section_data(
        bytes,
        headers[usize::try_from(u32_at(headers[symtab], 40)).unwrap()],
    );
    let symbols = section_data(bytes, headers[symtab])
        .as_chunks::<24>()
        .0
        .iter()
        .collect::<Vec<_>>();
    // Equal section names in different COMDAT groups are distinct sections.
    for (group_index, header) in headers.iter().enumerate() {
        if u32_at(header, 4) != 17 {
            continue;
        }
        let signature = symbols[usize::try_from(u32_at(header, 44)).unwrap()];
        let signature = cstr(strings, usize::try_from(u32_at(signature, 0)).unwrap());
        for member in section_data(bytes, header)[4..].as_chunks::<4>().0 {
            let index = usize::try_from(u32_at(member, 0)).unwrap();
            assert!(group_index < index, "ELF group must precede every member");
            write!(&mut names[index], " [{signature}]").unwrap();
        }
    }
    let symbol_name = |id: usize| {
        let s = symbols[id];
        if s[4] & 15 == 3 {
            names[usize::from(u16_at(s, 6))].clone()
        } else {
            cstr(strings, usize::try_from(u32_at(s, 0)).unwrap()).to_owned()
        }
    };
    let first_global = usize::try_from(u32_at(headers[symtab], 44)).unwrap();
    assert!(
        first_global > 0 && first_global <= symbols.len(),
        "invalid symtab sh_info"
    );
    for (i, s) in symbols.iter().enumerate() {
        assert_eq!(
            s[4] >> 4 == 0,
            i < first_global,
            "symtab sh_info must separate local and global symbols"
        );
    }
    for (i, &h) in headers.iter().enumerate() {
        let kind = u32_at(h, 4);
        let link = usize::try_from(u32_at(h, 40)).unwrap();
        let info = usize::try_from(u32_at(h, 44)).unwrap();
        view.headers.push(SectionHeader {
            name: if kind == 17 {
                format!(".group [{}]", symbol_name(info))
            } else {
                names[i].clone()
            },
            kind,
            flags: u64_at(h, 8),
            address: u64_at(h, 16),
            // Both string tables contain all compared names. Suffix sharing
            // changes their byte size without changing any ELF meaning.
            size: if kind == 3 { 0 } else { u64_at(h, 32) },
            link: if link == 0 {
                String::new()
            } else {
                names[link].clone()
            },
            info: match kind {
                | 4 => names[info].clone(),
                | 17 => symbol_name(info),
                | _ => info.to_string(),
            },
            align: u64_at(h, 48),
            entry_size: u64_at(h, 56),
        });
    }
    for (i, &h) in headers.iter().enumerate().skip(1) {
        let kind = u32_at(h, 4);
        match kind {
            | 1 | 7 | 8 | 14 | 15 | 16 | 0x7000_0001 => {
                // clang may keep empty standard sections; they have no
                // observables.
                let size = u64_at(h, 32);
                if size != 0 || names[i] == ".note.GNU-stack" {
                    assert!(
                        view.sections
                            .insert(
                                names[i].clone(),
                                (
                                    kind,
                                    u64_at(h, 8),
                                    u64_at(h, 48),
                                    section_data(bytes, h).to_vec(),
                                    size
                                )
                            )
                            .is_none(),
                        "duplicate named section requires a group-aware key"
                    );
                }
            },
            | 4 => {
                let target = &names[usize::try_from(u32_at(h, 44)).unwrap()];
                for r in section_data(bytes, h).as_chunks::<24>().0 {
                    let info = u64_at(r, 8);
                    view.relocations.push((
                        (*target).clone(),
                        u64_at(r, 0),
                        u32::try_from(info & 0xFFFF_FFFF).unwrap(),
                        symbol_name(usize::try_from(info >> 32).unwrap()),
                        u64_at(r, 16) as i64,
                    ));
                }
            },
            | 17 => {
                let data = section_data(bytes, h);
                assert_eq!(u32_at(data, 0), 1);
                let mut members = data[4..]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|n| names[usize::try_from(u32_at(n, 0)).unwrap()].clone())
                    .collect::<Vec<_>>();
                members.sort();
                view.groups.push((
                    symbol_name(usize::try_from(u32_at(h, 44)).unwrap()),
                    members,
                ));
            },
            | _ => {},
        }
    }
    for &s in symbols.iter().skip(1) {
        if s[4] & 15 == 3 {
            continue;
        }
        let name = cstr(strings, usize::try_from(u32_at(s, 0)).unwrap());
        let section = u16_at(s, 6);
        view.symbols.push((
            name.to_owned(),
            s[4],
            s[5],
            if section == 0 {
                "UND".to_owned()
            } else if section >= 0xFF00 {
                // clang -c adds an STT_FILE symbol with SHN_ABS.
                format!("SPECIAL:{section}")
            } else {
                names[usize::from(section)].clone()
            },
            u64_at(s, 8),
            u64_at(s, 16),
        ));
    }
    view.symbols.sort();
    view.relocations.sort();
    view.groups.sort();
    view
}

fn section_data<'a>(bytes: &'a [u8], h: &[u8]) -> &'a [u8] {
    if u32_at(h, 4) == 8 {
        return &[];
    }
    let start = usize::try_from(u64_at(h, 24)).unwrap();
    let size = usize::try_from(u64_at(h, 32)).unwrap();
    &bytes[start..start + size]
}
fn cstr(bytes: &[u8], at: usize) -> &str {
    let end = bytes[at..].iter().position(|&b| b == 0).unwrap() + at;
    std::str::from_utf8(&bytes[at..end]).unwrap()
}
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

#[test]
fn elf_attributes_tls_groups_aliases_and_initializers() {
    equivalent(
        ".text\n.globl f\n.type f,@function\nf:\n.cfi_startproc\npushq %rbx\n.cfi_def_cfa_offset \
         16\n.cfi_offset %rbx,-16\nsubq $256,%rsp\n.cfi_def_cfa_offset 272\nmovq %fs:0,%rax\nleaq \
         t@TPOFF(%rax),%r8\naddq ext@GOTTPOFF(%rip),%r8\nmovq \
         weak@GOTPCREL(%rip),%rsi\n.cfi_remember_state\naddq $256,%rsp\n.cfi_def_cfa_offset \
         16\npopq %rbx\n.cfi_def_cfa_offset 8\nretq\n.cfi_restore_state\n.cfi_endproc\n.size \
         f,.-f\n.weak weak\n.section .tdata,\"awT\",@progbits\n.p2align 3\n.type \
         t,@object\nt:\n.quad 42\n.size t,8\n.section .bss.g,\"awG\",@nobits,g,comdat\n.weak \
         g\n.hidden g\n.p2align 4\n.type g,@object\ng:\n.zero 16\n.size g,16\n.globl alias\nalias \
         = g\n.section .data.rel.ro,\"aw\",@progbits\n.quad f+8\n.quad t\n.section \
         .init_array.101,\"aw\",@init_array\n.p2align 3\n.quad f\n.section \
         .note.GNU-stack,\"\",@progbits\n",
    )
    .unwrap();
}

#[test]
fn branch_relaxation_boundaries_and_cascades() {
    for gap in [0, 1, 126, 127, 128, 129, 255, 256] {
        let mut text =
            format!(".text\nf:\njmp .Lend\n.Lstart:\n.zero {gap}\njne .Lstart\n.Lend:\nretq\n");
        equivalent(&text).unwrap();
        text.push_str(
            ".p2align 4,0x90\ng:\nje .La\njne .Lb\n.zero 123\n.La:\n.zero 128\n.Lb:\nretq\n",
        );
        equivalent(&text).unwrap();
    }
}

#[test]
fn encodings_at_register_and_immediate_boundaries() {
    let mut text = String::from(".text\n");
    for (a, b) in [("%rax", "%r8"), ("%r13", "%r12"), ("%rsp", "%rbp")] {
        for n in [
            -2_147_483_648,
            -129,
            -128,
            -1,
            0,
            1,
            127,
            128,
            2_147_483_647,
        ] {
            write!(
                &mut text,
                "movq ${n},{a}\naddq ${n},{a}\ncmpq ${n},{b}\nimulq ${n},{a},{b}\nmovq \
                 {a},{n}({b})\n"
            )
            .unwrap();
        }
    }
    for gap in 0..16 {
        write!(&mut text, ".p2align 4,0x90\n.zero {gap}\n").unwrap();
    }
    equivalent(&text).unwrap();
}

#[test]
fn unknown_encoding_fails_before_overwriting_output() {
    let directory = TempDir::new("native-object-refusal");
    let path = directory.path().join("keep.o");
    std::fs::write(&path, b"keep").unwrap();
    let error = super::super::build_object(
        ".text\nunimplemented %rax\n",
        &path,
        Target::LinuxGnu,
        OutputKind::Object,
    )
    .unwrap_err();
    assert!(error.to_string().contains("native object emission"));
    assert_eq!(std::fs::read(path).unwrap(), b"keep");
}

#[test]
fn integer_register_widths_and_displacements() {
    let rows = [
        (
            "b",
            [
                "al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b",
                "r12b", "r13b", "r14b", "r15b",
            ],
        ),
        (
            "w",
            [
                "ax", "cx", "dx", "bx", "sp", "bp", "si", "di", "r8w", "r9w", "r10w", "r11w",
                "r12w", "r13w", "r14w", "r15w",
            ],
        ),
        (
            "l",
            [
                "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d",
                "r11d", "r12d", "r13d", "r14d", "r15d",
            ],
        ),
        (
            "q",
            [
                "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11",
                "r12", "r13", "r14", "r15",
            ],
        ),
    ];
    let mut text = String::from(".text\n");
    for (width, registers) in rows {
        for a in registers {
            for b in registers {
                for op in [
                    "mov", "add", "sub", "adc", "sbb", "and", "or", "xor", "cmp", "test",
                ] {
                    writeln!(&mut text, "{op}{width} %{a},%{b}").unwrap();
                }
            }
        }
        for a in registers {
            for imm in [-128, 0, 1, 127, 128, 255] {
                write!(
                    &mut text,
                    "mov{width} ${imm},%{a}\nadd{width} ${imm},%{a}\ncmp{width} ${imm},%{a}\n"
                )
                .unwrap();
            }
        }
    }
    for base in ["rax", "rsp", "rbp", "r8", "r12", "r13", "r15"] {
        for disp in [-2_147_483_648, -129, -128, 0, 127, 128, 2_147_483_647] {
            write!(
                &mut text,
                "movq {disp}(%{base}),%r9\nmovb %sil,{disp}(%{base})\nmovq $7,{disp}(%{base})\n"
            )
            .unwrap();
        }
    }
    equivalent(&text).unwrap();
}

#[test]
fn sse_x87_and_conversion_encodings() {
    let mut text = String::from(".text\n");
    for a in 0..16 {
        for b in 0..16 {
            for op in [
                "movaps", "movups", "movss", "movsd", "xorps", "addss", "addsd", "subss", "subsd",
                "mulss", "mulsd", "divss", "divsd", "ucomiss", "ucomisd", "cvtss2sd", "cvtsd2ss",
            ] {
                // The emitter uses movss/movsd and movups for memory transfers.
                if matches!(op, "movss" | "movsd" | "movups") {
                    continue;
                }
                writeln!(&mut text, "{op} %xmm{a},%xmm{b}").unwrap();
            }
        }
    }
    for xmm in 0..16 {
        for gpr in ["rax", "r9", "r15"] {
            write!(
                &mut text,
                "movq %{gpr},%xmm{xmm}\nmovq %xmm{xmm},%{gpr}\ncvtsi2ssq \
                 %{gpr},%xmm{xmm}\ncvtsi2sdq %{gpr},%xmm{xmm}\ncvttss2si \
                 %xmm{xmm},%{gpr}\ncvttsd2si %xmm{xmm},%{gpr}\n"
            )
            .unwrap();
        }
        for op in ["movaps", "movups", "movss", "movsd"] {
            write!(
                &mut text,
                "{op} 128(%r13),%xmm{xmm}\n{op} %xmm{xmm},(%r12)\n"
            )
            .unwrap();
        }
    }
    for op in [
        "flds", "fldl", "fldt", "filds", "fildl", "fildll", "fstps", "fstpl", "fstpt", "fistps",
        "fistpl", "fistpll", "fnstcw", "fldcw",
    ] {
        writeln!(&mut text, "{op} 256(%r13)").unwrap();
    }
    for op in ["fadd", "fsub", "fmul", "fdiv"] {
        writeln!(&mut text, "{op} %st(1),%st").unwrap();
    }
    text.push_str("fstp %st(0)\nfucomip %st(1),%st\n");
    equivalent(&text).unwrap();
}

#[test]
#[ignore = "Explicit compile-time measurement against the clang text path."]
fn benchmark_native_object_emission() {
    use std::time::Instant;

    use crate::{
        backend::native::emit_assembly,
        util::bump::ArenaString,
    };
    let directory = TempDir::new("native-object-timing");
    let assembly = directory.join("text.s");
    let reference = directory.join("text.o");
    let native = directory.join("native.o");
    let toolchain = Toolchain::bundled(Target::LinuxGnu).unwrap();
    for name in [
        "widths",
        "division_and_shifts",
        "pressure_and_calls",
        "float_comparisons_and_conversions",
    ] {
        let Some(program) = super::programs::PROGRAMS.iter().find(|p| p.name == name) else {
            continue;
        };
        for optimize in [false, true] {
            super::end_to_end::with_module(program.source, Target::LinuxGnu, optimize, |module| {
                let mut times = [Vec::new(), Vec::new()];
                for repeat in 0..21 {
                    for arm in [repeat % 2, 1 - repeat % 2] {
                        let arena = Bump::new();
                        let start = Instant::now();
                        let mut text = ArenaString::new_in(&arena);
                        emit_assembly(module, Target::LinuxGnu, &mut text).unwrap();
                        if arm == 0 {
                            std::fs::write(&assembly, text.as_bytes()).unwrap();
                            toolchain
                                .assemble(
                                    &assembly,
                                    &reference,
                                    Target::LinuxGnu,
                                    OutputKind::Object,
                                )
                                .unwrap();
                        } else {
                            let bytes =
                                super::super::object::encode(text.into_str(), &arena).unwrap();
                            std::fs::write(&native, &bytes).unwrap();
                        }
                        let elapsed = start.elapsed();
                        if repeat > 0 {
                            times[arm].push(elapsed.as_secs_f64() * 1000.0);
                        }
                    }
                }
                for values in &mut times {
                    values.sort_by(f64::total_cmp);
                }
                eprintln!(
                    "native object timing {name} optimize={optimize}: text={:.3} ms (min {:.3}) \
                     direct={:.3} ms (min {:.3}) ratio={:.3} (20 interleaved samples; post-ABI IR \
                     to object, debug build)",
                    times[0][10],
                    times[0][0],
                    times[1][10],
                    times[1][0],
                    times[1][10] / times[0][10]
                );
            });
        }
    }
}

#[test]
fn coff_option_preserves_unwind_comdats_refptr_and_exports() {
    let text =
        ".text\n.def f;\n.scl 2;\n.type 32;\n.endef\n.globl f\nf:\n.seh_proc f\npushq \
         %rbx\n.seh_pushreg %rbx\nsubq $48,%rsp\n.seh_stackalloc 48\nmovaps \
         %xmm6,32(%rsp)\n.seh_savexmm %xmm6,32\n.seh_endprologue\nmovq \
         .refptr.ext(%rip),%rax\nmovaps 32(%rsp),%xmm6\naddq $48,%rsp\npopq \
         %rbx\nretq\n.seh_endproc\n.section .rdata$.refptr.ext,\"dr\",discard,.refptr.ext\n.globl \
         .refptr.ext\n.refptr.ext:\n.quad ext\n.section .data,\"dw\",discard,choice\n.globl \
         choice\nchoice:\n.long 1\n.section .drectve,\"yni\"\n.ascii \" /EXPORT:f\"\n";
    let directory = TempDir::new("native-coff-object");
    let assembly = directory.join("reference.s");
    let reference = directory.join("reference.obj");
    let native = directory.join("native.obj");
    std::fs::write(&assembly, text).unwrap();
    Toolchain::bundled(Target::WindowsMsvc)
        .unwrap()
        .assemble(
            &assembly,
            &reference,
            Target::WindowsMsvc,
            OutputKind::Object,
        )
        .unwrap();
    super::super::build_object(text, &native, Target::WindowsMsvc, OutputKind::Object).unwrap();
    super::object_coff::equivalent(text, Target::WindowsMsvc).unwrap();
    let dump = required_objdump(Command::new("objdump").args(["-x", "-dr"]).arg(&native)).unwrap();
    {
        assert!(dump.status.success(), "{dump:?}");
        let dump = String::from_utf8_lossy(&dump.stdout);
        for section in [".pdata", ".xdata", ".drectve", ".refptr.ext"] {
            assert!(dump.contains(section), "missing {section}: {dump}");
        }
    }
}

#[test]
fn quoted_names_label_differences_and_instruction_end_biases() {
    equivalent(
        ".text\n.globl \"f(foo) = bar\"\n.type \"f(foo) = bar\",@function\n\"f(foo) = \
         bar\":\n.Lfirst:\ncallq \"f(foo) = bar\"\nmovq $7,\"data(foo)\"+8(%rip)\nmovb \
         $1,\"data(foo)\"+2(%rip)\nleaq \"data(foo)\"-3(%rip),%r13\n.Lsecond:\nretq\n.size \
         \"f(foo) = bar\",.-\"f(foo) = bar\"\n.data\n.type \
         \"data(foo)\",@object\n\"data(foo)\":\n.quad .Lfirst+3\n.long .Lsecond-.Lfirst+4\n.short \
         .Lfirst-.Lsecond-2\n.byte .Lsecond-.Lfirst+1\n.zero 8\n.size \"data(foo)\",23\n",
    )
    .unwrap();
}

#[test]
fn multiple_comdats_can_share_a_section_name() {
    equivalent(
        ".section code,\"axG\",@progbits,a,comdat\n.weak a\n.type \
         a,@function\na:\n.cfi_startproc\ncallq external@PLT\nretq\n.cfi_endproc\n.size \
         a,.-a\n.section code,\"axG\",@progbits,b,comdat\n.weak b\n.type \
         b,@function\nb:\n.cfi_startproc\nmovl $2,%eax\nretq\n.cfi_endproc\n.size b,.-b\n",
    )
    .unwrap();
}

#[test]
fn implicit_code_alignment_uses_canonical_nops() {
    let mut text = String::from(".text\n");
    for gap in 0..32 {
        write!(&mut text, ".p2align 5\n.zero {gap}\n").unwrap();
    }
    equivalent(&text).unwrap();
}

#[test]
fn alignment_maximums_match_clang() {
    // Code alignment pads with `nop`s, a fill with that byte, and either
    // emits nothing when it would take more than the maximum; a maximum of
    // at least the alignment has no effect.
    for max in [1, 2, 3, 7, 10, 11, 14, 15, 16, 40] {
        let mut text = String::from(".text\n");
        for gap in 0..17 {
            write!(
                &mut text,
                ".zero {gap}\n.p2align 4,,{max}\nretq\n.p2align 5,0x90,{max}\nretq\n"
            )
            .unwrap();
        }
        text.push_str(".data\n");
        for gap in 0..9 {
            write!(&mut text, ".zero {gap}\n.p2align 3,0x2a,{max}\n.byte 1\n").unwrap();
        }
        text.push_str(".bss\n.zero 3\n.p2align 4,0,8\n.zero 1\n");
        equivalent(&text).unwrap();
    }
}

/// Alignment padding shrinks when branches before it grow, so whether a
/// branch over it is relaxed depends on the order in which MC lays out and
/// relaxes items. Random listings of jumps, spacers and alignments near
/// the rel8 boundary pin that order against the pinned assembler: in six of
/// these 150, promoting every out-of-range branch of a layout at once, as
/// the encoder first did, gives other sizes than MC's fused pass.
#[test]
fn branch_relaxation_across_alignment_matches_mc() {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    for _ in 0..150 {
        let labels = 24 + next(8);
        let mut text = String::from(".text\nf:\n");
        for label in 0..labels {
            for _ in 0..=next(3) {
                match next(7) {
                    | 0 | 1 => {
                        let target = next(labels);
                        let branch = if next(2) == 0 { "jmp" } else { "jne" };
                        writeln!(&mut text, "{branch} .L{target}").unwrap();
                    },
                    | 2 => writeln!(&mut text, ".p2align 4").unwrap(),
                    | 3 => writeln!(&mut text, ".p2align 4,,{}", 1 + next(14)).unwrap(),
                    | 4 => writeln!(&mut text, ".p2align 5,,{}", 1 + next(30)).unwrap(),
                    | _ => writeln!(&mut text, ".zero {}", next(32)).unwrap(),
                }
            }
            writeln!(&mut text, ".L{label}:").unwrap();
        }
        text.push_str("retq\n");
        equivalent(&text).unwrap();
    }
}

fn refuses(text: &str, diagnostic: &str) {
    let arena = Bump::new();
    let error = match super::super::object::encode(text, &arena) {
        | Ok(_) => panic!("listing was silently accepted: {text}"),
        | Err(error) => error,
    };
    assert!(error.to_string().contains(diagnostic), "{error}: {text}");
}

#[test]
fn x87_register_indices_and_directions_are_preserved() {
    let text = ".text\nfstp %st(1)\nfucomip %st(2),%st\nfadd %st,%st(1)\n";
    let arena = Bump::new();
    let bytes = super::super::object::encode(text, &arena).unwrap();
    assert_eq!(
        inspect(&bytes).sections[".text"].3,
        [0xDD, 0xD9, 0xDF, 0xEA, 0xDC, 0xC1]
    );
    equivalent(text).unwrap();
    for i in 0..8 {
        equivalent(&format!(
            "fstp %st({i})\nfucomip %st({i}),%st\nfadd %st({i}),%st\nfmul %st,%st({i})\n"
        ))
        .unwrap();
    }
    for text in [
        "fstp %st(8)",
        "fucomip %st(2)",
        "fucomip %st(2),%st(1)",
        "fadd %st(1),%st(2)",
        "fsub %st,%st(1)",
        "fstp %rax",
    ] {
        refuses(text, "native object emission");
    }
}

#[test]
fn operand_counts_classes_and_widths_are_checked() {
    equivalent("retq $8\nshldl $3,%eax,%ebx\nshldw %cl,%ax,%bx\nshrdq %cl,%rax,%rbx\n").unwrap();
    let arena = Bump::new();
    let bytes = super::super::object::encode("retq $8\nshldl $3,%eax,%ebx\n", &arena).unwrap();
    assert_eq!(
        inspect(&bytes).sections[".text"].3,
        [0xC2, 8, 0, 0x0F, 0xA4, 0xC3, 3]
    );
    for text in [
        "negq %rax,%rbx",
        "cmové %rax,%rbx",
        "shlq %rdx,%rax",
        "shldl $3,%rax,%rbx",
        "movq %eax,%rbx",
        "movsd %rax,%xmm0",
        "leaq %rax,%rbx",
        "fldt %rax",
        "callq",
        "movq %rax,%rbx,%rcx",
        "movq *%rax,%rbx",
        "movq (%eax),%rbx",
        "movq (%xmm1),%rax",
        "movq (%rax,%rcx,4,8),%rax",
        "movq (%rax,,8),%rax",
        "movq %rax,(%rax,%xmm1)",
        "retq $65536",
        "shlb $256,%al",
        "bswapw %ax",
        "cmoveb %al,%bl",
    ] {
        refuses(text, "native object emission");
    }
    // Each opcode family must reject both missing and surplus operands.
    for text in [
        "retq",
        "ud2",
        "int3",
        "cltd",
        "cqto",
        "rep movsb",
        "callq f",
        "jmpq *%rax",
        "pushq %rax",
        "popq %rbx",
        "movabsq $1,%rax",
        "movq %rax,%xmm0",
        "movd %eax,%xmm0",
        "movb %al,%bl",
        "leaq (%rax),%rbx",
        "movzbl %al,%ebx",
        "movaps %xmm0,%xmm1",
        "addsd %xmm0,%xmm1",
        "ucomiss %xmm0,%xmm1",
        "cvtsi2ssl %eax,%xmm0",
        "cvttsd2si %xmm0,%eax",
        "cvtss2sd %xmm0,%xmm1",
        "flds (%rax)",
        "fstp %st(0)",
        "fucomip %st(1),%st",
        "fadd %st(1),%st",
        "sete %al",
        "cmoveq %rax,%rbx",
        "imulq $3,%rax,%rbx",
        "shldq $3,%rax,%rbx",
        "bswapq %rax",
        "bsrq %rax,%rbx",
        "shlq %cl,%rax",
        "negq %rax",
        "addq %rax,%rbx",
    ] {
        refuses(&format!("{text},%rax"), "native object emission");
        let mnemonic = text.split_whitespace().next().unwrap();
        if !matches!(mnemonic, "retq" | "ud2" | "int3" | "cltd" | "cqto") {
            refuses(mnemonic, "native object emission");
        }
    }
}

/// The allocator's operands: `disp(base,index,scale)` for every base and
/// index whose low three bits need a SIB byte or a displacement (`%rsp`,
/// `%rbp`, `%r12`, `%r13`), as loads, stores, ALU and SSE sources, x87
/// operands and `lea`, including the `lea` into `%r11` that addresses a
/// store whose base, index and value are all spilled. Also the `cmov` of
/// every condition from fused compares and the shifts and masks of a
/// division by a power of two.
#[test]
fn indexed_operands_and_allocator_forms_match_clang() {
    let mut text = String::from(".text\n");
    for base in ["rax", "rsp", "rbp", "r11", "r12", "r13"] {
        for index in ["rcx", "rbp", "r10", "r12", "r13", "r15"] {
            for scale in [1, 2, 4, 8] {
                for disp in ["", "-8", "127", "128", "-2147483648"] {
                    let m = format!("{disp}(%{base},%{index},{scale})");
                    write!(
                        &mut text,
                        "movq {m},%r9\nmovl %esi,{m}\nmovb %sil,{m}\nmovq $-1,{m}\nleaq \
                         {m},%r11\naddq {m},%rdx\ncmpl {m},%r8d\nmovzbl {m},%eax\nmovslq \
                         {m},%r14\nmovsd {m},%xmm9\nmovss %xmm2,{m}\naddsd {m},%xmm1\nfldt \
                         {m}\nfstpt {m}\n"
                    )
                    .unwrap();
                }
            }
        }
    }
    text.push_str(
        "leaq (%r11,%r10,8),%r11\nmovq %r10,(%r11)\nleaq 16(%rsp,%r10,4),%r11\nleaq \
         (%rax,%rax,2),%rcx\nleaq (%r13,%r13,8),%r13\nleaq 3(%rbp,%rbp,4),%rbp\n",
    );
    for cond in [
        "o", "no", "b", "ae", "e", "ne", "be", "a", "s", "ns", "p", "np", "l", "ge", "le", "g",
    ] {
        write!(
            &mut text,
            "cmov{cond}l %ecx,%r9d\ncmov{cond}q %r10,%rax\ncmov{cond}q \
             8(%rsp,%rcx,8),%rdx\nset{cond} %r11b\n"
        )
        .unwrap();
    }
    text.push_str(
        "sarl $31,%ecx\nshrl $30,%ecx\naddl %eax,%ecx\nandl $-4,%ecx\nsarl $2,%eax\nandl \
         $3,%edx\nsarq $63,%r9\nshrq $62,%r9\nandq $-4,%r9\nshrl $2,%r15d\nandq $7,%r14\n",
    );
    equivalent(&text).unwrap();
    for text in [
        "movq (%rax,%rsp,8),%rcx",
        "movq (%rax,%rcx,3),%rdx",
        "movq (%rax,%rcx,16),%rdx",
        "movq x(%rip,%rcx,8),%rdx",
        "movq (%rax,%ecx,8),%rdx",
        "movq (%eax,%rcx,8),%rdx",
        "leal (%rax,%rcx,4),%edx",
        "leaq (%rax,%rcx,4),%edx",
        "leaq (%rax,%rcx,4)",
    ] {
        refuses(text, "native object emission");
    }
}

#[test]
fn oversized_immediates_and_displacements_are_refused() {
    for text in [
        "movq $2147483648,%rax",
        "addq $2147483648,%rax",
        "imulq $2147483648,%rax,%rbx",
        "movq 4294967296(%rax),%rax",
        "movq -2147483649(%rax),%rax",
        "movb $256,%al",
        "movw $65536,%ax",
        "movl $4294967296,%eax",
        "movq $-18446744073709551615,%rax",
    ] {
        refuses(text, "native object emission");
    }
    equivalent("movabsq $2147483648,%rax\nmovl $4294967295,%eax\nmovw $65535,%ax\nmovb $255,%al\n")
        .unwrap();
}

#[test]
fn unknown_relocation_suffixes_are_refused() {
    for text in [
        "leaq x@TLSGD(%rip),%rdi",
        "movq x@DTPOFF(%rax),%rax",
        ".quad y@DTPOFF",
        ".long f@PLT",
        "callq f@TLSGD",
        "movq x@PLT(%rip),%rax",
    ] {
        refuses(text, "native object emission");
    }
    equivalent(
        "sf:\nretq\ncallq sf@PLT\ncallq external@PLT\nmovl x@GOTPCREL(%rip),%r8d\nleaq \
         x@GOTPCREL(%rip),%rax\naddq x@GOTPCREL(%rip),%rax\n",
    )
    .unwrap();
}

#[test]
fn narrow_data_relocations_use_matching_widths() {
    equivalent(".data\n.byte ext\n.byte 42\n.short ext+1\n.short 42\n.long ext+2\n.quad ext+3\n")
        .unwrap();
}

#[test]
fn section_names_infer_flags_and_types() {
    for section in [
        ".data.rel.ro.local",
        ".bss.x",
        ".tbss.x",
        ".tdata.x",
        ".rodata.x",
        ".text.x",
        ".init_array.101",
        ".fini_array.101",
        ".preinit_array.x",
        "custom",
        ".init",
        ".fini",
        ".eh_frame",
        ".gcc_except_table",
        ".gcc_except_table.foo",
        ".data1",
        ".rodata1",
        ".note.foo",
        ".notefoo",
        ".note.GNU-stack",
        ".ctors",
        ".bssfoo",
    ] {
        for flags in ["", ",\"\"", ",\"aw\"", ",\"awx\""] {
            let text = format!(".section {section}{flags}\n.zero 8\n");
            if matches!(section, ".eh_frame" | ".gcc_except_table") && flags.contains('w') {
                refuses(&text, "section flags");
            } else {
                equivalent(&text).unwrap();
            }
        }
    }
}

#[test]
fn explicit_section_types_override_name_defaults() {
    for section in [
        ".bss.x",
        ".tbss.x",
        ".init_array",
        ".fini_array.101",
        ".preinit_array",
    ] {
        equivalent(&format!(
            ".section {section},\"aw\",@progbits\n.zero 8\n.section {section}\n.zero 8\n"
        ))
        .unwrap();
        refuses(
            &format!(".section {section},\"aw\",@progbits\n.section {section},\"aw\",@nobits"),
            "inconsistent section attributes",
        );
    }
}

#[test]
fn named_sections_from_c_match_clang_types() {
    let directory = TempDir::new("native-c-section-types");
    let source = directory.join("sections.c");
    let reference = directory.join("sections.o");
    let toolchain = Toolchain::bundled(Target::LinuxGnu).unwrap();
    for section in [
        ".init_array",
        ".fini_array.101",
        ".preinit_array",
        ".bss.foo",
    ] {
        let declaration = if section == ".bss.foo" {
            "int p;"
        } else {
            "void (*p)(void) = f;"
        };
        let text = format!(
            "static void f(void) {{}}\n__attribute__((section(\"{section}\"), used)) \
             {declaration}\n"
        );
        std::fs::write(&source, &text).unwrap();
        let result = Command::new(toolchain.clang())
            .args(["--target=x86_64-unknown-linux-gnu", "-c"])
            .arg(&source)
            .arg("-o")
            .arg(&reference)
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        let expected = inspect(&std::fs::read(&reference).unwrap());
        for optimize in [false, true] {
            super::end_to_end::with_module(&text, Target::LinuxGnu, optimize, |module| {
                let mut assembly = String::new();
                super::super::emit_assembly(module, Target::LinuxGnu, &mut assembly).unwrap();
                equivalent(&assembly).unwrap();
                let arena = Bump::new();
                let bytes = super::super::object::encode(&assembly, &arena).unwrap();
                let actual = inspect(&bytes);
                let find = |view: &View| {
                    let h = view.headers.iter().find(|h| h.name == section).unwrap();
                    (h.kind, h.flags, h.size, h.align, h.entry_size)
                };
                assert_eq!(find(&actual), find(&expected), "{section}: {assembly}");
            });
        }
    }
}

#[test]
fn direct_control_transfers_with_addends_are_refused() {
    for instruction in [
        "callq ext+4",
        "callq ext-4",
        "jmp ext+4",
        "jmpq ext+4",
        "jne ext+4",
    ] {
        refuses(instruction, "native object emission");
    }
    equivalent("callq ext+0\n").unwrap();
}

#[test]
fn unsigned_immediates_select_sign_extended_short_forms() {
    let text = "addl $4294967295,%eax\n";
    let arena = Bump::new();
    let bytes = super::super::object::encode(text, &arena).unwrap();
    assert_eq!(inspect(&bytes).sections[".text"].3, [0x83, 0xC0, 0xFF]);
    for (suffix, register, values) in [
        ("w", "%ax", [65_407_i64, 65_408, 65_535, 127, 128]),
        (
            "l",
            "%eax",
            [4_294_967_167, 4_294_967_168, 4_294_967_295, 127, 128],
        ),
    ] {
        for value in values {
            for mnemonic in ["add", "sub", "and", "or", "xor", "cmp", "adc", "sbb"] {
                equivalent(&format!("{mnemonic}{suffix} ${value},{register}\n")).unwrap();
                equivalent(&format!("{mnemonic}{suffix} ${value},(%rax)\n")).unwrap();
            }
            equivalent(&format!("imul{suffix} ${value},{register},{register}\n")).unwrap();
        }
    }
    for text in [
        "addl $8589934591,%eax",
        "addw $131071,%ax",
        "imull $8589934591,%eax,%eax",
    ] {
        refuses(text, "operand width");
    }
}

#[test]
fn equivalence_rejects_elf_header_mutations() {
    let arena = Bump::new();
    let bytes = super::super::object::encode("retq\n", &arena).unwrap();
    let base = inspect(&bytes);
    // OSABI, ABI version, padding, type, machine, version, entry, program
    // header offset, flags, and header/entry sizes are all significant.
    for field in [7, 8, 9, 16, 18, 20, 24, 32, 48, 52, 54, 56, 58] {
        let mut mutated = bytes.to_vec();
        mutated[field] ^= 1;
        assert!(
            std::panic::catch_unwind(|| {
                assert_equivalent(&inspect(&mutated), &base, "ELF header mutation");
            })
            .is_err(),
            "ELF header mutation {field} escaped comparison"
        );
    }
}

#[test]
fn invalid_directives_and_overflowing_differences_are_refused() {
    for (text, diagnostic) in [
        ("a:\na:\n", "duplicate"),
        (".weak w\n.globl w\n", "binding"),
        (".p2align 4,0x90,3,1", "directive operands"),
        (".p2align 4,,0", "unsatisfiable alignment maximum"),
        (".p2align 4,", "missing alignment fill"),
        (".p2align 4,,", "invalid integer"),
        ("f:\ng:\n.size f,.-g", "size expression"),
        (".size f,.-f\n.zero 8\nf:", "size expression"),
        (
            ".La:\n.zero 300\n.Lb:\n.byte .Lb-.La",
            "exceeds field width",
        ),
        (
            ".La:\n.zero 129\n.Lb:\n.byte .La-.Lb",
            "exceeds field width",
        ),
        (
            ".La:\n.zero 65536\n.Lb:\n.short .Lb-.La",
            "exceeds field width",
        ),
        (".byte 256", "exceeds field width"),
        (
            ".La:\n.zero 300\n.Lb:\n.quad .La-.Lb-9223372036854775808",
            "exceeds field width",
        ),
        ("callq \"escaped\\\\symbol\"", "symbol expression"),
        (
            ".globl .Lglobal\n.Lglobal:\nretq",
            "temporary symbol attributes",
        ),
        (
            "local:\nretq\nmovq local@GOTPCREL(%rip),%rax",
            "GOT reference to local",
        ),
        (
            "local:\nretq\nalias = local\njmp alias",
            "branch to local alias",
        ),
        (".hidden f\n.protected f", "conflicting symbol visibility"),
        (
            ".type f,@object\n.type f,@function",
            "conflicting symbol type",
        ),
        (
            ".section x,\"awG\",@progbits,g@TLSGD,comdat",
            "symbol expression",
        ),
        (".cfi_unknown", "unknown CFI"),
        (".cfi_def_cfa_offset 8", "outside procedure"),
        (
            ".cfi_startproc\n.cfi_restore_state\n.cfi_endproc",
            "without remembered state",
        ),
        (".text extra", "directive operands"),
        (".section x,\"a\",@progbits,ignored", "section arguments"),
    ] {
        refuses(text, diagnostic);
    }
    equivalent(".La:\n.zero 255\n.Lb:\n.byte .Lb-.La\n.short .La-.Lb\n").unwrap();
}

#[test]
fn global_branches_calls_and_data_remain_interposable() {
    for binding in [
        ".globl gf",
        ".weak gf",
        ".globl gf\n.hidden gf",
        ".globl gf\n.protected gf",
    ] {
        equivalent(&format!(
            "{binding}\ngf:\njmp gf\nje gf\ncallq gf\nleaq gf(%rip),%rax\n.Lbase:\n.long \
             gf-.Lbase\n"
        ))
        .unwrap();
    }
    equivalent(
        "local:\njmp local\ncallq local\n.text\njmp external\nje external\ncallq \
         external\n.data\nother:\n.quad 0\n.text\njmp other\ncallq other\ncallq other@PLT\n",
    )
    .unwrap();
}

#[test]
fn equivalence_rejects_section_header_mutations() {
    let arena = Bump::new();
    let direct = super::super::object::encode(
        "local:\nretq\n.globl f\nf:\ncallq ext\n.data\n.quad f\n.section \
         group,\"awG\",@progbits,g,comdat\n.weak g\ng:\n.quad f\n",
        &arena,
    )
    .unwrap();
    let base = inspect(&direct);
    let shoff = usize::try_from(u64_at(&direct, 40)).unwrap();
    let count = usize::from(u16_at(&direct, 60));
    let header = |kind| {
        (0..count)
            .find(|&i| u32_at(&direct, shoff + i * 64 + 4) == kind)
            .unwrap()
    };
    for (index, field, value) in [
        (header(2), 44, 1_u32), // the reviewer's previously undetected sh_info mutation
        (header(2), 40, 0),     // sh_link
        (header(4), 44, 0),     // relocation sh_info
        (header(4), 56, 8),     // sh_entsize
        (header(17), 40, 0),
    ] {
        let mut mutated = direct.to_vec();
        let at = shoff + index * 64 + field;
        mutated[at..at + 4].copy_from_slice(&value.to_le_bytes());
        let detected = std::panic::catch_unwind(|| {
            assert_equivalent(&inspect(&mutated), &base, "header mutation");
        });
        assert!(
            detected.is_err(),
            "header mutation {index}:{field} escaped comparison"
        );
    }
    let mut reordered = direct.to_vec();
    let first_index = 2;
    let second_index = 4;
    let first = shoff + first_index * 64;
    let second = shoff + second_index * 64;
    for n in 0..64 {
        reordered.swap(first + n, second + n);
    }
    let remap = |index| {
        if index == first_index {
            second_index
        } else if index == second_index {
            first_index
        } else {
            index
        }
    };
    // Preserve every reference while swapping headers: the mutation changes
    // order alone, so a section-set comparison would still accept this object.
    for index in 0..count {
        let at = shoff + index * 64;
        let link = remap(usize::try_from(u32_at(&reordered, at + 40)).unwrap());
        reordered[at + 40..at + 44].copy_from_slice(&u32::try_from(link).unwrap().to_le_bytes());
        let kind = u32_at(&reordered, at + 4);
        if kind == 4 {
            let info = remap(usize::try_from(u32_at(&reordered, at + 44)).unwrap());
            reordered[at + 44..at + 48]
                .copy_from_slice(&u32::try_from(info).unwrap().to_le_bytes());
        } else if kind == 2 {
            let offset = usize::try_from(u64_at(&reordered, at + 24)).unwrap();
            let size = usize::try_from(u64_at(&reordered, at + 32)).unwrap();
            for symbol in (offset..offset + size).step_by(24) {
                let section = remap(usize::from(u16_at(&reordered, symbol + 6)));
                reordered[symbol + 6..symbol + 8]
                    .copy_from_slice(&u16::try_from(section).unwrap().to_le_bytes());
            }
        } else if kind == 17 {
            let offset = usize::try_from(u64_at(&reordered, at + 24)).unwrap();
            let size = usize::try_from(u64_at(&reordered, at + 32)).unwrap();
            for member in (offset + 4..offset + size).step_by(4) {
                let section = remap(usize::try_from(u32_at(&reordered, member)).unwrap());
                reordered[member..member + 4]
                    .copy_from_slice(&u32::try_from(section).unwrap().to_le_bytes());
            }
        }
    }
    let reordered = inspect(&reordered);
    assert_eq!(reordered.sections, base.sections);
    assert_eq!(reordered.symbols, base.symbols);
    assert_eq!(reordered.relocations, base.relocations);
    assert_eq!(reordered.groups, base.groups);
    assert!(
        std::panic::catch_unwind(|| assert_equivalent(&reordered, &base, "section order mutation"))
            .is_err(),
        "section order mutation escaped comparison"
    );
}

pub(super) fn required_objdump(command: &mut Command) -> io::Result<std::process::Output> {
    command.output().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "required objdump {:?} could not run: {error}; install llvm-objdump or GNU \
                 binutils (COFF checks require GNU objdump)",
                command.get_program()
            ),
        )
    })
}

#[test]
fn missing_objdump_is_an_explicit_failure() {
    let directory = TempDir::new("missing-objdump");
    let error = required_objdump(&mut Command::new(directory.join("absent-objdump"))).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(error.to_string().contains("required objdump"));
    assert!(error.to_string().contains("install"));
}
