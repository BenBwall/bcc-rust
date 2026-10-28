//! Decoding and validation reject what the specification rejects, and the
//! listing shows one instruction per line.

use super::{
    Builder,
    builder::{
        leb,
        section,
    },
    program,
    raw,
};
use crate::{
    Program,
    ValType::{
        F32,
        I32,
        I64,
    },
};

fn rejects(bytes: &[u8], expected: &str) {
    match Program::load(bytes, None) {
        | Ok(_) => panic!("accepted a module that should fail with {expected:?}"),
        | Err(error) => assert!(error.contains(expected), "{error:?} lacks {expected:?}"),
    }
}

fn invalid(code: &str, expected: &str) {
    let mut module = Builder::new();
    module.memory(1, None);
    module.func("f", &[], &[I32], &[I32], code);
    rejects(&module.build(), expected);
}

#[test]
fn malformed_modules_are_rejected_with_their_offset() {
    rejects(b"\0asn\x01\0\0\0", "not a WebAssembly module");
    rejects(b"\0asm\x02\0\0\0", "version 1");
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    section(&mut bytes, 1, &[1, 0x60, 0, 0]);
    section(&mut bytes, 1, &[0]);
    rejects(&bytes, "out of order");
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    bytes.extend([1, 10]);
    rejects(&bytes, "past the end");
    // A function section without a code section.
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    section(&mut bytes, 1, &[1, 0x60, 0, 0]);
    section(&mut bytes, 3, &[1, 0]);
    rejects(&bytes, "disagree");
    // SIMD is named, not just unknown.
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    section(&mut bytes, 1, &[1, 0x60, 1, 0x7B, 0]);
    rejects(&bytes, "SIMD");
    let error = Program::load(&bytes, None).unwrap_err();
    assert!(error.contains("at byte 0xd"), "the v128 byte: {error}");
}

#[test]
fn ill_typed_code_is_rejected() {
    invalid("i64.const 1", "expected an i32 operand but found an i64");
    invalid("i32.add", "needs an operand");
    invalid("i32.const 1 i32.const 2", "extra values");
    invalid("br 1", "label 1");
    invalid("i32.const 0 local.get 3", "local 3 does not exist");
    invalid("call 9", "function 9 does not exist");
    invalid("i32.const 0 i32.load align=8", "alignment");
    invalid("i32.const 0 if i32 i32.const 1 end", "without an else");
    invalid(
        "f32.const 1 i32.const 2 i32.const 1 select",
        "different types",
    );
    invalid("global.get 0", "global 0 does not exist");
    let mut module = Builder::new();
    let constant = module.global("c", I32, false, "i32.const 1");
    module.func(
        "f",
        &[],
        &[],
        &[],
        &format!("i32.const 2 global.set {constant}"),
    );
    rejects(&module.build(), "immutable");
    let mut module = Builder::new();
    module.func("f", &[], &[I32], &[], "i32.const 0 i32.load");
    rejects(&module.build(), "no memory");
}

#[test]
fn code_after_an_unconditional_branch_is_polymorphic() {
    let mut module = Builder::new();
    module.func("f", &[], &[I32], &[], "unreachable i32.add");
    module.func(
        "g",
        &[],
        &[I64],
        &[],
        "block i64 i64.const 1 br 0 f32.const 1 drop end",
    );
    program(&module);
}

#[test]
fn the_listing_has_one_line_per_instruction() {
    let mut module = Builder::new();
    module.memory(1, None).data(Some(8), b"hi\n\"");
    let sp = module.global("__stack_pointer", I32, true, "i32.const 1024");
    module.func(
        "f",
        &[I32],
        &[I32],
        &[F32],
        &format!(
            "block i32 i32.const 1 local.get 0 br_if 0 drop global.get {sp} i32.const 4 i32.sub \
             i32.load offset=8 f32.const 1.5 local.set 1 end"
        ),
    );
    let program = program(&module);
    let text = &program.wat.text;
    let expected = [
        "  (func $f (type 0) (param i32) (result i32)",
        "    (local f32)",
        "    block (result i32)  ;; label = @1",
        "      i32.const 1",
        "      local.get 0",
        "      br_if 0 (;@1;)",
        "      drop",
        "      global.get $__stack_pointer",
        "      i32.const 4",
        "      i32.sub",
        "      i32.load offset=8",
        "      f32.const 1.5",
        "      local.set 1",
        "    end",
        "  )",
    ];
    let start = text
        .lines()
        .position(|line| line == expected[0])
        .unwrap_or_else(|| panic!("{text}"));
    let shown: Vec<&str> = text.lines().skip(start).take(expected.len()).collect();
    assert_eq!(shown, expected, "{text}");
    assert!(
        text.contains("(global $__stack_pointer (mut i32) (i32.const 1024))"),
        "{text}"
    );
    assert!(
        text.contains("(data (;0;) (i32.const 8) \"hi\\n\\\"\")"),
        "{text}"
    );
    // Each instruction line maps back to its instruction, and the closing
    // parenthesis to the body's final `end`.
    let func = program.module.export("f", crate::ExternKind::Func).unwrap();
    let body = program.module.body(func).unwrap();
    assert_eq!(program.wat.line_of(func, 0), Some(start + 3));
    assert_eq!(
        program.wat.line_of(func, body.code.len() as u32 - 1),
        Some(start + expected.len())
    );
    assert_eq!(program.wat.header_of(func), Some(start + 1));
}

#[test]
fn padded_leb_immediates_decode() {
    // Linkers leave 5-byte relocatable forms in calls and global indices.
    let mut module = Builder::new();
    module.func("target", &[], &[I32], &[], "i32.const 9");
    let mut bytes = module.build();
    // Rebuild main's body by hand: call 0 with a 5-byte index.
    let mut body = vec![0];
    body.extend([0x10, 0x80, 0x80, 0x80, 0x80, 0x00, 0x0B]);
    let mut code = vec![1];
    leb(&mut code, body.len() as u64);
    code.extend(body);
    let mut rebuilt = b"\0asm\x01\0\0\0".to_vec();
    section(&mut rebuilt, 1, &[1, 0x60, 0, 1, 0x7F]);
    section(&mut rebuilt, 3, &[1, 0]);
    section(&mut rebuilt, 10, &code);
    let program = Program::load(&rebuilt, None).unwrap();
    assert_eq!(
        program.module.body(0).unwrap().code[0],
        crate::Instr::Call(0)
    );
    bytes.clear();
}

#[test]
fn the_2_0_module_rules_are_enforced() {
    // Two exports with one name.
    let mut module = Builder::new();
    module.func("f", &[], &[], &[], "");
    module.func("f", &[], &[], &[], "");
    rejects(&module.build(), "two exports are named \"f\"");
    // A table whose maximum is below its minimum.
    rejects(
        &raw(&[(4, &[1, 0x70, 1, 2, 1])]),
        "maximum is below its minimum",
    );
    // ref.func of a function nothing declares; exporting it declares it.
    let types: &[u8] = &[1, 0x60, 0, 1, 0x70];
    let code: &[u8] = &[1, 4, 0, 0xD2, 0, 0x0B];
    rejects(
        &raw(&[(1, types), (3, &[1, 0]), (10, code)]),
        "no element segment, export",
    );
    let export: &[u8] = &[1, 1, b'f', 0, 0];
    Program::load(
        &raw(&[(1, types), (3, &[1, 0]), (7, export), (10, code)]),
        None,
    )
    .unwrap();
    // A global initializer may read only imported globals.
    let mut module = Builder::new();
    module.global("a", I32, false, "i32.const 1");
    module.global("b", I32, false, "global.get 0");
    module.func("f", &[], &[], &[], "");
    rejects(&module.build(), "may read only imported globals");
    // memory.init needs a DataCount section.
    let types: &[u8] = &[1, 0x60, 0, 0];
    let body: &[u8] = &[1, 12, 0, 0x41, 0, 0x41, 0, 0x41, 0, 0xFC, 8, 0, 0, 0x0B];
    rejects(
        &raw(&[
            (1, types),
            (3, &[1, 0]),
            (5, &[1, 0, 1]),
            (10, body),
            (11, &[1, 1, 0]),
        ]),
        "need a data count section",
    );
    // A block type index is an s33: at most five bytes.
    let block = |index: &[u8]| {
        let mut body = vec![0, 0x02];
        body.extend_from_slice(index);
        body.extend([0x0B, 0x0B]);
        let mut code = vec![1];
        leb(&mut code, body.len() as u64);
        code.extend(body);
        raw(&[(1, types), (3, &[1, 0]), (10, &code)])
    };
    Program::load(&block(&[0x80, 0x80, 0x80, 0x80, 0x00]), None).unwrap();
    rejects(&block(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00]), "too large");
    // Extended constants stay accepted.
    let mut module = Builder::new();
    module.global("a", I32, false, "i32.const 1 i32.const 2 i32.add");
    module.func("f", &[], &[], &[], "");
    program(&module);
}
