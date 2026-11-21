; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [10 x i8] c"found a 5\00", align 1
@.str.1 = internal constant [19 x i8] c"%s after %d tries\0A\00", align 1
@.str.2 = internal constant [6 x i8] c"found\00", align 1
@.str.3 = internal constant [10 x i8] c"not found\00", align 1

define i32 @main() #0 {
block0:
  %slot0 = alloca [12 x i8], align 4
  store i32 7, ptr %slot0, align 4
  %v5 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 9, ptr %v5, align 4
  %v9 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 4, ptr %v9, align 4
  br label %block1

block1:                                           ; preds = %block3, %block0
  %v11 = phi i32 [ 0, %block0 ], [ %v23, %block3 ]
  %v13 = icmp slt i32 %v11, 3
  br i1 %v13, label %block2, label %block4

block2:                                           ; preds = %block1
  %v15 = sext i32 %v11 to i64
  %v17 = mul nsw i64 %v15, 4
  %v18 = getelementptr inbounds i8, ptr %slot0, i64 %v17
  %v19 = load i32, ptr %v18, align 4
  %v21 = icmp ne i32 %v19, 5
  br i1 %v21, label %block3, label %block4

block3:                                           ; preds = %block2
  %v23 = add nsw i32 %v11, 1
  br label %block1

block4:                                           ; preds = %block2, %block1
  %v25 = icmp slt i32 %v11, 3
  br i1 %v25, label %block5, label %block6

block5:                                           ; preds = %block4
  %v27 = sext i32 %v11 to i64
  %v29 = mul nsw i64 %v27, 4
  %v30 = getelementptr inbounds i8, ptr %slot0, i64 %v29
  br label %block7

block6:                                           ; preds = %block4
  br label %block7

block7:                                           ; preds = %block5, %block6
  %v33 = phi ptr [ %v30, %block5 ], [ null, %block6 ]
  %v36 = icmp ne ptr %v33, null
  br i1 %v36, label %block8, label %block10

block8:                                           ; preds = %block7
  %v37 = load i32, ptr %v33, align 4
  %v39 = icmp eq i32 %v37, 5
  br i1 %v39, label %block9, label %block10

block9:                                           ; preds = %block8
  %v41 = call i32 @puts(ptr noundef @.str)
  br label %block10

block10:                                          ; preds = %block9, %block8, %block7
  %v44 = icmp ne ptr %v33, null
  br i1 %v44, label %block11, label %block12

block11:                                          ; preds = %block10
  br label %block13

block12:                                          ; preds = %block10
  br label %block13

block13:                                          ; preds = %block11, %block12
  %v47 = phi ptr [ @.str.2, %block11 ], [ @.str.3, %block12 ]
  %v48 = call i32 (ptr, ...) @printf(ptr noundef @.str.1, ptr noundef %v47, i32 noundef %v11)
  ret i32 0
}

declare i32 @puts(ptr noundef)

declare i32 @printf(ptr noundef, ...)

attributes #0 = { "frame-pointer"="all" }

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
