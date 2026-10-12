//! Transparent slice inputs, names and independently owned printed text.

use super::*;

#[test]
fn borrowed_lists_and_printed_text() {
    let arena = Bump::new();
    let printed = Session::with(&arena, Target::WindowsGnu, |api| {
        let empty = api.const_structure(&[]);
        let global = api.global(api.name("empty"), api.structure(&[], true));
        api.initializer(global, empty);

        let fields = [api.ty(Type::I8), api.ty(Type::I64)];
        let values = [api.bits(Type::I8, 7), api.bits(Type::I64, 42)];
        let global = api.global(api.name("fält"), api.structure(&fields, true));
        api.initializer(global, api.const_structure(&values));
        let bytes = [0, 0x80, 0xFF];
        let global = api.global(api.name("bytes"), api.array(api.ty(Type::I8), 3));
        api.initializer(global, api.bytes(&bytes));

        let signature = api.signature(api.ty(Type::I64), &[api.ty(Type::I64)], false);
        let callee = api.function(api.name("callee"), signature);
        let function = api.function(
            api.name("caller"),
            api.signature(api.ty(Type::I64), &[], false),
        );
        let entry = api.block(function, api.name("entry"));
        let join = api.block(function, api.name("join"));
        api.position(entry);
        let call = api.call(signature, callee, &[values[1]]);
        _ = api.jump(join);
        api.position(join);
        let phi = api.phi(api.ty(Type::I64));
        api.incoming(phi, call, entry);
        _ = api.ret(phi);
        let (trap, ty) = api.intrinsic("llvm.trap", &[]);
        let function = api.function(api.name("trap"), api.signature(api.void(), &[], false));
        api.position(api.block(function, api.name("entry")));
        _ = api.call(ty, trap, &[]);
        _ = api.ret_void();
        // Exercise a nonempty overload list as well as the empty intrinsic
        // list.
        _ = api.intrinsic("llvm.ctpop", &[api.ty(Type::I64)]);
        api.verify().unwrap();
        assert_eq!(api.name("fält").to_bytes(), "fält".as_bytes());
        assert_eq!(api.name("").to_bytes(), b"");
        api.print()
    });
    let text = printed.text();
    assert!(text.is_ascii(), "LLVM escapes non-ASCII names and bytes");
    assert!(text.contains("i8 7, i64 42"), "{text}");
    assert!(text.contains("call i64 @callee(i64 42)"), "{text}");
    assert!(text.contains("phi i64"), "{text}");
    assert!(text.contains("@llvm.ctpop.i64"), "{text}");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "IR names contain no NUL")]
fn nul_in_name_panics() {
    let arena = Bump::new();
    Session::with(&arena, Target::WindowsGnu, |api| {
        _ = api.name("bad\0name");
    });
}
