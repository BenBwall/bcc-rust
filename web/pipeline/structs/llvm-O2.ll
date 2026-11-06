; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@calls = global [4 x i8] zeroinitializer, align 4
@origin = global [8 x i8] c"\01\00\00\00\02\00\00\00", align 4
@moved.total = internal global [4 x i8] zeroinitializer, align 4

define i64 @moved(i64 %v13, i32 noundef %v2) {
block0:
  %slot0 = alloca [8 x i8], align 8
  %v0 = alloca [16 x i8], align 8
  store i64 %v13, ptr %slot0, align 8
  %v4 = load i32, ptr @moved.total, align 4
  %v5 = add nsw i32 %v4, %v2
  store i32 %v5, ptr @moved.total, align 4
  %v7 = load i32, ptr @calls, align 4
  %v9 = add nsw i32 %v7, 1
  store i32 %v9, ptr @calls, align 4
  %v10 = load i32, ptr %slot0, align 4
  %v11 = add nsw i32 %v10, %v2
  store i32 %v11, ptr %slot0, align 4
  call void @llvm.memcpy.p0.p0.i64(ptr align 4 %v0, ptr align 4 %slot0, i64 8, i1 false)
  %0 = load <8 x i8>, ptr %v0, align 8
  %1 = freeze <8 x i8> %0
  %inst16.ret = bitcast <8 x i8> %1 to i64
  ret i64 %inst16.ret
}

define i32 @main() {
block0:
  %v4 = load i32, ptr @origin, align 4
  %v5 = getelementptr inbounds i8, ptr @origin, i64 4
  %v6 = load i32, ptr %v5, align 4
  %v9 = load i32, ptr @moved.total, align 4
  %v10 = add nsw i32 %v9, 3
  store i32 %v10, ptr @moved.total, align 4
  %v12 = load i32, ptr @calls, align 4
  %v14 = add nsw i32 %v12, 1
  store i32 %v14, ptr @calls, align 4
  %v15 = add nsw i32 %v4, 3
  %v17 = add nsw i32 %v6, 10
  %v18 = mul nsw i32 %v15, 2
  %v21 = add i32 %v9, 7
  store i32 %v21, ptr @moved.total, align 4
  %v22 = add i32 %v12, 2
  store i32 %v22, ptr @calls, align 4
  %v23 = add nsw i32 %v18, 4
  %v24 = add nsw i32 %v23, %v17
  %v25 = add nsw i32 %v24, %v22
  %v26 = add nsw i32 %v25, 5
  ret i32 %v26
}

; Function Attrs: nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias writeonly captures(none), ptr noalias readonly captures(none), i64, i1 immarg) #0

attributes #0 = { nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
