; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [15 x i8] c"total is %.3d\0A\00", align 1

define i32 @main() {
block0:
  br label %block1

block1:                                           ; preds = %block3, %block0
  %v3 = phi i32 [ 0, %block0 ], [ %v9, %block3 ]
  %v4 = phi i32 [ 0, %block0 ], [ %v7, %block3 ]
  %v6 = icmp slt i32 %v3, 4
  br i1 %v6, label %block2, label %block4

block2:                                           ; preds = %block1
  %v7 = add nsw i32 %v4, 3
  br label %block3

block3:                                           ; preds = %block2
  %v9 = add nsw i32 %v3, 1
  br label %block1

block4:                                           ; preds = %block1
  %v12 = add nsw i32 %v4, 44
  %v14 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef %v12)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
