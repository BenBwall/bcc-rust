; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [7 x i8] c"%d %d\0A\00", align 1

define i32 @twice(i32 noundef %v0) {
block0:
  %v2 = mul nsw i32 2, %v0
  ret i32 %v2
}

define i32 @square(i32 noundef %v0) {
block0:
  %v1 = mul nsw i32 %v0, %v0
  ret i32 %v1
}

define i32 @sum_of(ptr noundef %v0, i32 noundef %v1, ptr noundef %v2) {
block0:
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v6 = phi i32 [ 0, %block0 ], [ %v15, %block2 ]
  %v7 = phi i32 [ 0, %block0 ], [ %v14, %block2 ]
  %v8 = icmp slt i32 %v6, %v1
  br i1 %v8, label %block2, label %block3

block2:                                           ; preds = %block1
  %v9 = sext i32 %v6 to i64
  %v10 = mul nsw i64 %v9, 4
  %v11 = getelementptr inbounds i8, ptr %v0, i64 %v10
  %v12 = load i32, ptr %v11, align 4
  %v13 = call i32 %v2(i32 noundef %v12)
  %v14 = add nsw i32 %v7, %v13
  %v15 = add nsw i32 %v6, 1
  br label %block1

block3:                                           ; preds = %block1
  ret i32 %v7
}

define i32 @main() {
block0:
  %slot0 = alloca [8 x i8], align 4
  store i32 3, ptr %slot0, align 4
  %v4 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 4, ptr %v4, align 4
  br label %block2

block1:                                           ; preds = %block2
  %v9 = phi i32 [ %v12, %block2 ]
  br label %block5

block2:                                           ; preds = %block3, %block0
  %v11 = phi i32 [ 0, %block0 ], [ %v20, %block3 ]
  %v12 = phi i32 [ 0, %block0 ], [ %v19, %block3 ]
  %v13 = icmp slt i32 %v11, 2
  br i1 %v13, label %block3, label %block1

block3:                                           ; preds = %block2
  %v14 = sext i32 %v11 to i64
  %v15 = mul nsw i64 %v14, 4
  %v16 = getelementptr inbounds i8, ptr %slot0, i64 %v15
  %v17 = load i32, ptr %v16, align 4
  %v18 = call i32 @twice(i32 noundef %v17)
  %v19 = add nsw i32 %v12, %v18
  %v20 = add nsw i32 %v11, 1
  br label %block2

block4:                                           ; preds = %block5
  %v21 = phi i32 [ %v25, %block5 ]
  %v23 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef %v9, i32 noundef %v21)
  ret i32 0

block5:                                           ; preds = %block6, %block1
  %v24 = phi i32 [ 0, %block1 ], [ %v33, %block6 ]
  %v25 = phi i32 [ 0, %block1 ], [ %v32, %block6 ]
  %v26 = icmp slt i32 %v24, 2
  br i1 %v26, label %block6, label %block4

block6:                                           ; preds = %block5
  %v27 = sext i32 %v24 to i64
  %v28 = mul nsw i64 %v27, 4
  %v29 = getelementptr inbounds i8, ptr %slot0, i64 %v28
  %v30 = load i32, ptr %v29, align 4
  %v31 = call i32 @square(i32 noundef %v30)
  %v32 = add nsw i32 %v25, %v31
  %v33 = add nsw i32 %v24, 1
  br label %block5
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
