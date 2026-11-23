; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [10 x i8] c"%d %d %d\0A\00", align 1

define i32 @main() {
block0:
  br label %block1

block1:                                           ; preds = %block3, %block2, %block0
  %v4 = phi i32 [ 0, %block0 ], [ %v7, %block2 ], [ %v7, %block3 ]
  %v5 = phi i32 [ 0, %block0 ], [ %v5, %block2 ], [ %v9, %block3 ]
  %v6 = icmp slt i32 %v4, 3
  br i1 %v6, label %block2, label %block4

block2:                                           ; preds = %block1
  %v7 = add nsw i32 %v4, 1
  %v8 = icmp eq i32 %v7, 2
  br i1 %v8, label %block1, label %block3

block3:                                           ; preds = %block2
  %v9 = add nsw i32 %v5, %v7
  br label %block1

block4:                                           ; preds = %block4, %block1
  %v10 = phi i32 [ 0, %block1 ], [ %v12, %block4 ]
  %v11 = phi i32 [ 0, %block1 ], [ %v15, %block4 ]
  %v12 = add nsw i32 %v10, 1
  %v13 = icmp eq i32 %v12, 2
  %v14 = add i32 %v11, %v12
  %v15 = select i1 %v13, i32 %v11, i32 %v14
  %v16 = icmp slt i32 %v12, 3
  br i1 %v16, label %block4, label %block5

block5:                                           ; preds = %block6, %block4
  %v17 = phi i32 [ 1, %block4 ], [ %v23, %block6 ]
  %v18 = phi i32 [ 0, %block4 ], [ %v22, %block6 ]
  %v19 = icmp sle i32 %v17, 3
  br i1 %v19, label %block6, label %block7

block6:                                           ; preds = %block5
  %v20 = icmp eq i32 %v17, 2
  %v21 = add i32 %v18, %v17
  %v22 = select i1 %v20, i32 %v18, i32 %v21
  %v23 = add nsw i32 %v17, 1
  br label %block5

block7:                                           ; preds = %block5
  %v25 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef %v5, i32 noundef %v15, i32 noundef %v18)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
