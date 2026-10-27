; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @main() {
block0:
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v4 = phi i32 [ 0, %block0 ], [ %v8, %block2 ]
  %v5 = phi i32 [ 2147483646, %block0 ], [ %v7, %block2 ]
  %v6 = icmp slt i32 %v4, 3
  br i1 %v6, label %block2, label %block3

block2:                                           ; preds = %block1
  %v7 = add nsw i32 %v5, 1
  %v8 = add nsw i32 %v4, 1
  br label %block1

block3:                                           ; preds = %block1
  ret i32 %v5
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
