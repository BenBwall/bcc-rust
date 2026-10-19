//! Byte-level ELF equivalence against the pinned clang assembler. Every
//! native assembly/run helper and the native difftest oracle uses this check.
//! Section and symbol ordering is immaterial; contents, flags, alignments,
//! symbol properties, COMDAT membership and RELA tuples must agree exactly.

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
    // Raw executable-section equality above also pins every encoded
    // instruction. When objdump is installed, check its independent
    // disassembly as well.
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
    if let Ok(expected) = Command::new(&objdump)
        .args(["-dr"])
        .arg(&reference)
        .output()
    {
        assert!(
            expected.status.success(),
            "reference disassembly failed: {expected:?}"
        );
        let actual = Command::new(&objdump).arg("-dr").arg(&native).output()?;
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

#[derive(Debug, PartialEq, Eq)]
struct View {
    sections:    BTreeMap<String, (u32, u64, u64, Vec<u8>, u64)>,
    symbols:     Vec<(String, u8, u8, String, u64, u64)>,
    relocations: Vec<(String, u64, u32, String, i64)>,
    groups:      Vec<(String, Vec<String>)>,
}

fn inspect(bytes: &[u8]) -> View {
    assert_eq!(&bytes[..7], b"\x7fELF\x02\x01\x01");
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
        sections:    BTreeMap::new(),
        symbols:     Vec::new(),
        relocations: Vec::new(),
        groups:      Vec::new(),
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
    for header in &headers {
        if u32_at(header, 4) != 17 {
            continue;
        }
        let signature = symbols[usize::try_from(u32_at(header, 44)).unwrap()];
        let signature = cstr(strings, usize::try_from(u32_at(signature, 0)).unwrap());
        for member in section_data(bytes, header)[4..].as_chunks::<4>().0 {
            let index = usize::try_from(u32_at(member, 0)).unwrap();
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
    for (i, &h) in headers.iter().enumerate().skip(1) {
        let kind = u32_at(h, 4);
        match kind {
            | 1 | 8 | 14 | 15 | 0x7000_0001 => {
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
fn coff_option_keeps_text_unwind_comdats_refptr_and_exports() {
    let text =
        ".text\n.def f;\n.scl 2;\n.type 32;\n.endef\n.globl f\nf:\n.seh_proc f\npushq \
         %rbx\n.seh_pushreg %rbx\nsubq $48,%rsp\n.seh_stackalloc 48\nmovaps \
         %xmm6,32(%rsp)\n.seh_savexmm %xmm6,32\n.seh_endprologue\nmovq \
         .refptr.ext(%rip),%rax\nmovaps 32(%rsp),%xmm6\naddq $48,%rsp\npopq \
         %rbx\nretq\n.seh_endproc\n.section .rdata$.refptr.ext,\"dr\",discard,.refptr.ext\n.globl \
         .refptr.ext\n.refptr.ext:\n.quad ext\n.section .data,\"dw\",discard,choice\n.globl \
         choice\nchoice:\n.long 1\n.section .drectve,\"yni\"\n.ascii \" /EXPORT:f\"\n";
    let directory = TempDir::new("native-coff-fallback");
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
    assert_eq!(
        std::fs::read(&native).unwrap(),
        std::fs::read(&reference).unwrap()
    );
    let dump = Command::new("objdump")
        .args(["-x", "-dr"])
        .arg(&native)
        .output();
    if let Ok(dump) = dump {
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
