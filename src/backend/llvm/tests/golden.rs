//! Golden tests: textual bcc IR in, LLVM IR text out.

use super::*;

/// One golden module: its name, its bcc IR, the profile it verifies in, and
/// the LLVM IR it prints for x86-64 Linux.
pub(super) struct Golden {
    pub(super) name:     &'static str,
    pub(super) input:    &'static str,
    pub(super) profile:  Profile,
    pub(super) expected: &'static str,
}

impl Golden {
    pub(super) fn emit(&self) -> String {
        emit(self.input, self.profile, Target::LinuxGnu)
    }
}

pub(super) const GOLDENS: &[Golden] = &[ARITHMETIC, LOOP, MEMORY, CALLS, SWITCH, GLOBALS, VARARGS];

const ARITHMETIC: Golden = Golden {
    name:     "arithmetic",
    input:    "\
function @arith(i32, i32, i64, f64) -> i32 external {
block0(v0: i32, v1: i32, v2: i64, v3: f64):
    v4 = iadd.i32 nsw v0, v1
    v5 = isub.i32 nuw v4, v1
    v6 = imul.i32 nsw nuw v5, v0
    v7 = sdiv.i32 exact v6, v1
    v8 = udiv.i32 v7, v1
    v9 = srem.i32 v8, v1
    v10 = urem.i32 v9, v1
    v11 = shl.i32 nsw v10, v0
    v12 = lshr.i32 exact v11, v0
    v13 = ashr.i32 v12, v0
    v14 = iconst.i32 -7
    v15 = and.i32 v13, v14
    v16 = or.i32 v15, v0
    v17 = xor.i32 v16, v1
    v18 = icmp.i32 sle v17, v0
    v19 = select.i32 v18, v17, v14
    v20 = freeze.i32 v19
    v21 = sext.i64 v20
    v22 = iadd.i64 v21, v2
    v23 = sitofp.f64 v22
    v24 = fconst.f64 0x3FF8000000000000
    v25 = fmul.f64 v23, v24
    v26 = fneg.f64 v25
    v27 = fcmp.f64 ult v26, v3
    v28 = zext.i32 v27
    v29 = fptosi.i32 v26
    v30 = iadd.i32 v28, v29
    return v30
}

function @convert(i128, f32, ptr) -> f128 internal {
block0(v0: i128, v1: f32, v2: ptr):
    v3 = iconst.i128 -170141183460469231731687303715884105728
    v4 = iadd.i128 v0, v3
    v5 = trunc.i16 v4
    v6 = zext.i64 v5
    v7 = uitofp.f32 v6
    v8 = fconst.f32 0x3DCCCCCD
    v9 = fsub.f32 v7, v8
    v10 = fdiv.f32 v9, v1
    v11 = frem.f32 v10, v1
    v12 = fpext.f80 v11
    v13 = fconst.f80 0x3FFF8000000000000000
    v14 = fadd.f80 v12, v13
    v15 = fptrunc.f64 v14
    v16 = fptoui.i64 v15
    v17 = bitcast.f64 v16
    v18 = fcmp.f64 uno v17, v15
    v19 = ptrtoint.i64 v2
    v20 = inttoptr.ptr v19
    v21 = icmp.ptr eq v20, v2
    v22 = and.i1 v18, v21
    v23 = fconst.f128 0x3FFF8000000000000000000000000000
    v24 = poison.f128
    v25 = select.f128 v22, v23, v24
    return v25
}
",
    profile:  Profile::PostAbi,
    expected: r#"target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define dso_local i32 @arith(i32 %v0, i32 %v1, i64 %v2, double %v3) {
block0:
  %v4 = add nsw i32 %v0, %v1
  %v5 = sub nuw i32 %v4, %v1
  %v6 = mul nuw nsw i32 %v5, %v0
  %v7 = sdiv exact i32 %v6, %v1
  %v8 = udiv i32 %v7, %v1
  %v9 = srem i32 %v8, %v1
  %v10 = urem i32 %v9, %v1
  %v11 = shl nsw i32 %v10, %v0
  %v12 = lshr exact i32 %v11, %v0
  %v13 = ashr i32 %v12, %v0
  %v15 = and i32 %v13, -7
  %v16 = or i32 %v15, %v0
  %v17 = xor i32 %v16, %v1
  %v18 = icmp sle i32 %v17, %v0
  %v19 = select i1 %v18, i32 %v17, i32 -7
  %v20 = freeze i32 %v19
  %v21 = sext i32 %v20 to i64
  %v22 = add i64 %v21, %v2
  %v23 = sitofp i64 %v22 to double
  %v25 = fmul double %v23, 0x3FF8000000000000
  %v26 = fneg double %v25
  %v27 = fcmp ult double %v26, %v3
  %v28 = zext i1 %v27 to i32
  %v29 = fptosi double %v26 to i32
  %v30 = add i32 %v28, %v29
  ret i32 %v30
}

define internal fp128 @convert(i128 %v0, float %v1, ptr %v2) {
block0:
  %v4 = add i128 %v0, -170141183460469231731687303715884105728
  %v5 = trunc i128 %v4 to i16
  %v6 = zext i16 %v5 to i64
  %v7 = uitofp i64 %v6 to float
  %v9 = fsub float %v7, 0x3FB99999A0000000
  %v10 = fdiv float %v9, %v1
  %v11 = frem float %v10, %v1
  %v12 = fpext float %v11 to x86_fp80
  %v14 = fadd x86_fp80 %v12, 0xK3FFF8000000000000000
  %v15 = fptrunc x86_fp80 %v14 to double
  %v16 = fptoui double %v15 to i64
  %v17 = bitcast i64 %v16 to double
  %v18 = fcmp uno double %v17, %v15
  %v19 = ptrtoint ptr %v2 to i64
  %v20 = inttoptr i64 %v19 to ptr
  %v21 = icmp eq ptr %v20, %v2
  %v22 = and i1 %v18, %v21
  %v25 = select i1 %v22, fp128 0xL00000000000000003FFF800000000000, fp128 poison
  ret fp128 %v25
}
"#,
};

const LOOP: Golden = Golden {
    name:     "loop",
    input:    "\
function @count(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 slt v2, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 nsw v3, v2
    v6 = iconst.i32 1
    v7 = iadd.i32 nsw v2, v6
    jump block1(v7, v5)
block3:
    return v3
}

function @twice(i1, i32) -> i32 external {
block0(v0: i1, v1: i32):
    brif v0, block1(v1), block1(v1)
block1(v2: i32):
    return v2
block2(v3: i32):
    jump block1(v3)
}
",
    profile:  Profile::PostAbi,
    expected: r#"target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define dso_local i32 @count(i32 %v0) {
block0:
  br label %block1
block1:
  %v2 = phi i32 [ 0, %block0 ], [ %v7, %block2 ]
  %v3 = phi i32 [ 0, %block0 ], [ %v5, %block2 ]
  %v4 = icmp slt i32 %v2, %v0
  br i1 %v4, label %block2, label %block3
block2:
  %v5 = add nsw i32 %v3, %v2
  %v7 = add nsw i32 %v2, 1
  br label %block1
block3:
  ret i32 %v3
}

define dso_local i32 @twice(i1 %v0, i32 %v1) {
block0:
  br i1 %v0, label %block1, label %block1
block1:
  %v2 = phi i32 [ %v1, %block0 ], [ %v1, %block0 ]
  ret i32 %v2
}
"#,
};

const MEMORY: Golden = Golden {
    name:     "memory",
    input:    "\
global @counter internal size 4, align 4 = zero

function @memory(ptr, i64) -> i32 external {
    slot0 = stack_slot 16, align 8
    slot1 = stack_slot 4, align 4
block0(v0: ptr, v1: i64):
    v2 = stack_addr slot0
    v3 = stack_addr slot1
    v4 = global_addr @counter
    v5 = load.i32 volatile v4, align 4
    store.i32 v5, v3, align 4
    v6 = ptr_add inbounds v2, v1
    v7 = ptr_add v0, v1
    store.i64 volatile v1, v6, align 8
    v8 = iconst.i64 16
    copy v2, v0, v8, align 8
    copy volatile may_overlap v7, v0, v8, align 1
    v9 = iconst.i8 0
    fill v2, v9, v8, align 8
    v10 = null
    store.ptr v10, v2, align 8
    v11 = load.i32 v3, align 4
    return v11
}
",
    profile:  Profile::PostAbi,
    expected: r#"target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@counter = internal global [4 x i8] zeroinitializer, align 4

define dso_local i32 @memory(ptr %v0, i64 %v1) {
block0:
  %slot0 = alloca [16 x i8], align 8
  %slot1 = alloca [4 x i8], align 4
  %v5 = load volatile i32, ptr @counter, align 4
  store i32 %v5, ptr %slot1, align 4
  %v6 = getelementptr inbounds i8, ptr %slot0, i64 %v1
  %v7 = getelementptr i8, ptr %v0, i64 %v1
  store volatile i64 %v1, ptr %v6, align 8
  call void @llvm.memcpy.p0.p0.i64(ptr align 8 %slot0, ptr align 8 %v0, i64 16, i1 false)
  call void @llvm.memmove.p0.p0.i64(ptr align 1 %v7, ptr align 1 %v0, i64 16, i1 true)
  call void @llvm.memset.p0.i64(ptr align 8 %slot0, i8 0, i64 16, i1 false)
  store ptr null, ptr %slot0, align 8
  %v11 = load i32, ptr %slot1, align 4
  ret i32 %v11
}

declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1 immarg)
declare void @llvm.memmove.p0.p0.i64(ptr, ptr, i64, i1 immarg)
declare void @llvm.memset.p0.i64(ptr, i8, i64, i1 immarg)
"#,
};

const CALLS: Golden = Golden {
    name:     "calls",
    input:    "\
function @add(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 nsw v0, v1
    return v2
}

function @sink(ptr) external

function @calls(i32, ptr) -> i32 internal {
block0(v0: i32, v1: ptr):
    v2 = call @add(v0, v0)
    call @sink(v1)
    v3 = func_addr @add
    v4 = call_indirect v3(v2, v0) : (i32, i32) -> i32
    v5 = call_indirect v1(v4) : (i32) -> i32
    call_indirect v1() : ()
    return v5
}
",
    profile:  Profile::PostAbi,
    expected: r#"target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define dso_local i32 @add(i32 %v0, i32 %v1) {
block0:
  %v2 = add nsw i32 %v0, %v1
  ret i32 %v2
}

declare void @sink(ptr)

define internal i32 @calls(i32 %v0, ptr %v1) {
block0:
  %v2 = call i32 @add(i32 %v0, i32 %v0)
  call void @sink(ptr %v1)
  %v4 = call i32 (i32, i32) @add(i32 %v2, i32 %v0)
  %v5 = call i32 (i32) %v1(i32 %v4)
  call void () %v1()
  ret i32 %v5
}
"#,
};

const SWITCH: Golden = Golden {
    name:     "switch",
    input:    "\
function @classify(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 10
    v2 = iconst.i32 20
    switch v0, block3(v2), [-1: block1, 0: block2(v1), 7: block2(v1), 9: block3(v2)]
block1:
    unreachable
block2(v3: i32):
    jump block3(v3)
block3(v4: i32):
    return v4
}
",
    profile:  Profile::PostAbi,
    expected: r#"target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define dso_local i32 @classify(i32 %v0) {
block0:
  switch i32 %v0, label %block3 [
    i32 -1, label %block1
    i32 0, label %block2
    i32 7, label %block2
    i32 9, label %block3
  ]
block1:
  unreachable
block2:
  %v3 = phi i32 [ 10, %block0 ], [ 10, %block0 ]
  br label %block3
block3:
  %v4 = phi i32 [ 20, %block0 ], [ 20, %block0 ], [ %v3, %block2 ]
  ret i32 %v4
}
"#,
};

const GLOBALS: Golden = Golden {
    name:     "globals",
    input:    "\
global @zeroed internal size 8, align 8 = zero
global @message external constant size 6, align 1 = bytes \"68692022215c\"
global @blank internal size 3, align 1 = bytes \"000000\"
global @table internal constant size 28, align 8 = bytes \
               \"0102030405060708000000000000000000000000000000000a0b0c0d\" relocs [16: @message \
               + 2, 8: @answer]
global @back external size 8, align 8 = bytes \"0000000000000000\" relocs [0: @table - 8]
global @errno external size 4, align 4
global @limits external constant size 0, align 1

function @answer() -> i32 external {
block0:
    v0 = iconst.i32 42
    return v0
}
",
    profile:  Profile::PostAbi,
    expected: r#"target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@zeroed = internal global [8 x i8] zeroinitializer, align 8
@message = dso_local constant [6 x i8] c"hi \22!\5C", align 1
@blank = internal global [3 x i8] zeroinitializer, align 1
@table = internal constant <{ [8 x i8], ptr, ptr, [4 x i8] }> <{ [8 x i8] c"\01\02\03\04\05\06\07\08", ptr @answer, ptr getelementptr (i8, ptr @message, i64 2), [4 x i8] c"\0A\0B\0C\0D" }>, align 8
@back = dso_local global <{ ptr }> <{ ptr getelementptr (i8, ptr @table, i64 -8) }>, align 8
@errno = external global [4 x i8], align 4
@limits = external constant [0 x i8], align 1

define dso_local i32 @answer() {
block0:
  ret i32 42
}
"#,
};

const VARARGS: Golden = Golden {
    name:     "varargs",
    input:    "\
global @format internal constant size 4, align 1 = bytes \"25640a00\"

function @printf(ptr, ...) -> i32 external

function @anything(...) external

function @sum(i32, ...) -> i32 external {
    slot0 = stack_slot 24, align 8
    slot1 = stack_slot 24, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    va_start v1
    va_copy v2, v1
    v3 = va_arg.i32 v1
    va_end v2
    va_end v1
    v4 = iadd.i32 nsw v0, v3
    v5 = global_addr @format
    v6 = call @printf(v5, v4)
    v7 = iconst.i64 3
    call @anything(v7, v5)
    return v4
}
",
    profile:  Profile::PreAbi,
    expected: r#"target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@format = internal constant [4 x i8] c"%d\0A\00", align 1

declare i32 @printf(ptr, ...)

declare void @anything(...)

define dso_local i32 @sum(i32 %v0, ...) {
block0:
  %slot0 = alloca [24 x i8], align 8
  %slot1 = alloca [24 x i8], align 8
  call void @llvm.va_start.p0(ptr %slot0)
  call void @llvm.va_copy.p0(ptr %slot1, ptr %slot0)
  %v3 = va_arg ptr %slot0, i32
  call void @llvm.va_end.p0(ptr %slot1)
  call void @llvm.va_end.p0(ptr %slot0)
  %v4 = add nsw i32 %v0, %v3
  %v6 = call i32 (ptr, ...) @printf(ptr @format, i32 %v4)
  call void (...) @anything(i64 3, ptr @format)
  ret i32 %v4
}

declare void @llvm.va_start.p0(ptr)
declare void @llvm.va_copy.p0(ptr, ptr)
declare void @llvm.va_end.p0(ptr)
"#,
};

#[test]
fn modules_print_their_golden_llvm_ir() {
    for golden in GOLDENS {
        pretty_assertions::assert_eq!(golden.emit(), golden.expected, "{}", golden.name);
    }
}

#[test]
fn modules_for_another_target_are_refused() {
    let arena = Bump::new();
    let module = parse_module(
        &arena,
        "target triple = \"x86_64-unknown-linux-gnu\"
target datalayout = \"\"
",
    )
    .unwrap();
    let mut out = String::new();
    assert_eq!(
        emit_module(&module, Target::WindowsMsvc, &mut out),
        Err(EmitError::TargetMismatch {
            module:    "x86_64-unknown-linux-gnu",
            requested: "x86_64-pc-windows-msvc",
        })
    );
}

#[test]
fn overlapping_relocations_are_refused() {
    let arena = Bump::new();
    let module = parse_module(
        &arena,
        "global @g external size 16, align 8 = bytes \"00000000000000000000000000000000\"          relocs [0: @g, 4: @g]
",
    )
    .unwrap();
    let mut out = String::new();
    assert_eq!(
        emit_module(&module, Target::LinuxGnu, &mut out),
        Err(EmitError::OverlappingRelocations {
            global: "g",
            first:  0,
            second: 4,
        })
    );
}

#[test]
fn names_and_constants_are_spelled_for_llvm() {
    let names: Vec<String> = ["main", "$x.y-z", "9lives", "a b\"c\\", ""]
        .iter()
        .map(|name| syntax::symbol(name).to_string())
        .collect();
    assert_eq!(
        names,
        [
            "@main",
            "@$x.y-z",
            "@\"9lives\"",
            r#"@"a b\22c\5C""#,
            "@\"\""
        ]
    );
    let float = |bits, ty| syntax::float(bits, ty).to_string();
    // A float NaN keeps its payload in the double form LLVM reads.
    assert_eq!(float(0x7FC0_0001, Type::F32), "0x7FF8000020000000");
    assert_eq!(float(0xFF80_0000, Type::F32), "0xFFF0000000000000");
    assert_eq!(float(0x8000_0001, Type::F32), "0xB6A0000000000000");
    let int = |bits, ty| syntax::int(bits, ty).to_string();
    assert_eq!(
        [
            int(1, Type::I1),
            int(0, Type::I1),
            int(0xFF, Type::I8),
            int(u128::MAX, Type::I128)
        ],
        ["true", "false", "-1", "-1"]
    );
}
