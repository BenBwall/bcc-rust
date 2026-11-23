; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [10 x i8] c"%d %d %d\0A\00", align 1

define i32 @main() #0 {
block0:
  br label %block1

block1:                                           ; preds = %block3, %block4, %block0
  %v2 = phi i32 [ 0, %block0 ], [ %v7, %block3 ], [ %v7, %block4 ]
  %v3 = phi i32 [ 0, %block0 ], [ %v3, %block3 ], [ %v10, %block4 ]
  %v5 = icmp slt i32 %v2, 3
  br i1 %v5, label %block2, label %block5

block2:                                           ; preds = %block1
  %v7 = add nsw i32 %v2, 1
  %v9 = icmp eq i32 %v7, 2
  br i1 %v9, label %block3, label %block4

block3:                                           ; preds = %block2
  br label %block1

block4:                                           ; preds = %block2
  %v10 = add nsw i32 %v3, %v7
  br label %block1

block5:                                           ; preds = %block1
  br label %block6

block6:                                           ; preds = %block9, %block5
  %v13 = phi i32 [ 0, %block5 ], [ %v16, %block9 ]
  %v14 = phi i32 [ 0, %block5 ], [ %v20, %block9 ]
  %v16 = add nsw i32 %v13, 1
  %v18 = icmp eq i32 %v16, 2
  br i1 %v18, label %block7, label %block8

block7:                                           ; preds = %block6
  br label %block9

block8:                                           ; preds = %block6
  %v19 = add nsw i32 %v14, %v16
  br label %block9

block9:                                           ; preds = %block7, %block8
  %v20 = phi i32 [ %v14, %block7 ], [ %v19, %block8 ]
  %v22 = icmp slt i32 %v16, 3
  br i1 %v22, label %block6, label %block10

block10:                                          ; preds = %block9
  br label %block11

block11:                                          ; preds = %block15, %block10
  %v25 = phi i32 [ 1, %block10 ], [ %v34, %block15 ]
  %v26 = phi i32 [ 0, %block10 ], [ %v32, %block15 ]
  %v28 = icmp sle i32 %v25, 3
  br i1 %v28, label %block12, label %block16

block12:                                          ; preds = %block11
  %v30 = icmp eq i32 %v25, 2
  br i1 %v30, label %block13, label %block14

block13:                                          ; preds = %block12
  br label %block15

block14:                                          ; preds = %block12
  %v31 = add nsw i32 %v26, %v25
  br label %block15

block15:                                          ; preds = %block13, %block14
  %v32 = phi i32 [ %v26, %block13 ], [ %v31, %block14 ]
  %v34 = add nsw i32 %v25, 1
  br label %block11

block16:                                          ; preds = %block11
  %v36 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef %v3, i32 noundef %v20, i32 noundef %v26)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

attributes #0 = { "frame-pointer"="all" }

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
