; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @main() {
block0:
  %slot0 = alloca [12 x i8], align 4
  store i32 2, ptr %slot0, align 4
  %v5 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 4, ptr %v5, align 4
  %v9 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 6, ptr %v9, align 4
  %v13 = getelementptr inbounds i8, ptr %slot0, i64 12
  %v14 = load i32, ptr %v13, align 4
  ret i32 %v14
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
