; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [15 x i8] c"total is %.3d\0A\00", align 1

define i32 @main() {
block0:
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v4 = phi i32 [ 0, %block0 ], [ %v8, %block2 ]
  %v5 = phi i32 [ 0, %block0 ], [ %v7, %block2 ]
  %v6 = icmp slt i32 %v4, 4
  br i1 %v6, label %block2, label %block3

block2:                                           ; preds = %block1
  %v7 = add nsw i32 %v5, 3
  %v8 = add nsw i32 %v4, 1
  br label %block1

block3:                                           ; preds = %block1
  %v10 = add nsw i32 %v5, 44
  %v12 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef %v10)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
