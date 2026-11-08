; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [14 x i8] c"Hello, world!\00", align 1
@.str.1 = internal constant [4 x i8] c"Ada\00", align 1
@.str.2 = internal constant [19 x i8] c"%s has %d letters\0A\00", align 1

define i32 @main() {
block0:
  %slot0 = alloca [4 x i8], align 1
  %v1 = call i32 @puts(ptr noundef @.str)
  call void @llvm.memcpy.p0.p0.i64(ptr align 1 %slot0, ptr align 1 @.str.1, i64 4, i1 false)
  %v6 = call i32 @length(ptr noundef %slot0)
  %v9 = call i32 (ptr, ...) @printf(ptr noundef @.str.2, ptr noundef %slot0, i32 noundef %v6)
  ret i32 0
}

define i32 @length(ptr noundef %v0) {
block0:
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v2 = phi i32 [ 0, %block0 ], [ %v10, %block2 ]
  %v3 = sext i32 %v2 to i64
  %v4 = getelementptr inbounds i8, ptr %v0, i64 %v3
  %v5 = load i8, ptr %v4, align 1
  %v6 = sext i8 %v5 to i32
  %v8 = icmp ne i32 %v6, 0
  br i1 %v8, label %block2, label %block3

block2:                                           ; preds = %block1
  %v10 = add nsw i32 %v2, 1
  br label %block1

block3:                                           ; preds = %block1
  ret i32 %v2
}

declare i32 @puts(ptr noundef)

declare i32 @printf(ptr noundef, ...)

; Function Attrs: nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias writeonly captures(none), ptr noalias readonly captures(none), i64, i1 immarg) #0

attributes #0 = { nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
