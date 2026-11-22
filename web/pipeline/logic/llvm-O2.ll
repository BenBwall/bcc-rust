; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [10 x i8] c"found a 5\00", align 1
@.str.1 = internal constant [19 x i8] c"%s after %d tries\0A\00", align 1
@.str.2 = internal constant [6 x i8] c"found\00", align 1
@.str.3 = internal constant [10 x i8] c"not found\00", align 1

define i32 @main() {
block0:
  %slot0 = alloca [12 x i8], align 4
  store i32 7, ptr %slot0, align 4
  %v4 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 9, ptr %v4, align 4
  %v7 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 4, ptr %v7, align 4
  br label %block1

block1:                                           ; preds = %block3, %block0
  %v12 = phi i32 [ 0, %block0 ], [ %v19, %block3 ]
  %v13 = icmp slt i32 %v12, 3
  br i1 %v13, label %block2, label %block4

block2:                                           ; preds = %block1
  %v14 = sext i32 %v12 to i64
  %v15 = mul nsw i64 %v14, 4
  %v16 = getelementptr inbounds i8, ptr %slot0, i64 %v15
  %v17 = load i32, ptr %v16, align 4
  %v18 = icmp ne i32 %v17, 5
  br i1 %v18, label %block3, label %block4

block3:                                           ; preds = %block2
  %v19 = add nsw i32 %v12, 1
  br label %block1

block4:                                           ; preds = %block2, %block1
  %v20 = sext i32 %v12 to i64
  %v21 = mul i64 %v20, 4
  %v22 = getelementptr i8, ptr %slot0, i64 %v21
  %v24 = select i1 %v13, ptr %v22, ptr null
  %v25 = icmp ne ptr %v24, null
  br i1 %v25, label %block5, label %block7

block5:                                           ; preds = %block4
  %v26 = load i32, ptr %v24, align 4
  %v27 = icmp eq i32 %v26, 5
  br i1 %v27, label %block6, label %block7

block6:                                           ; preds = %block5
  %v29 = call i32 @puts(ptr noundef @.str)
  br label %block7

block7:                                           ; preds = %block6, %block5, %block4
  %v33 = select i1 %v25, ptr @.str.2, ptr @.str.3
  %v34 = call i32 (ptr, ...) @printf(ptr noundef @.str.1, ptr noundef %v33, i32 noundef %v12)
  ret i32 0
}

declare i32 @puts(ptr noundef)

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
