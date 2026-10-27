; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @factorial(i32 noundef %v0) {
block0:
  %v2 = icmp sle i32 %v0, 1
  br i1 %v2, label %block1, label %block2

block1:                                           ; preds = %block0
  ret i32 1

block2:                                           ; preds = %block0
  %v3 = sub nsw i32 %v0, 1
  %v4 = call i32 @factorial(i32 noundef %v3)
  %v5 = mul nsw i32 %v0, %v4
  ret i32 %v5
}

define i32 @main() {
block0:
  %v1 = call i32 @factorial(i32 noundef 4)
  ret i32 %v1
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
