; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [14 x i8] c"2u - 3 is %u\0A\00", align 1
@.str.1 = internal constant [27 x i8] c"the countdown ended at %u\0A\00", align 1
@.str.2 = internal constant [42 x i8] c"%zu bytes, sum %u, -1 < 1u is %d, %u, %u\0A\00", align 1

define i32 @main() {
block0:
  %slot0 = alloca [12 x i8], align 4
  %v2 = sub i32 2, 3
  %v4 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef %v2)
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v6 = phi i32 [ 2, %block0 ], [ %v10, %block2 ]
  %v8 = icmp ult i32 %v6, 3
  br i1 %v8, label %block2, label %block3

block2:                                           ; preds = %block1
  %v10 = sub i32 %v6, 1
  br label %block1

block3:                                           ; preds = %block1
  %v12 = call i32 (ptr, ...) @printf(ptr noundef @.str.1, i32 noundef %v6)
  store i32 4, ptr %slot0, align 4
  %v18 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 5, ptr %v18, align 4
  %v22 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 6, ptr %v22, align 4
  br label %block4

block4:                                           ; preds = %block6, %block3
  %v25 = phi i32 [ 3, %block3 ], [ %v28, %block6 ]
  %v26 = phi i32 [ 0, %block3 ], [ %v37, %block6 ]
  %v28 = sub i32 %v25, 1
  %v30 = icmp ugt i32 %v25, 0
  br i1 %v30, label %block5, label %block7

block5:                                           ; preds = %block4
  %v32 = zext i32 %v28 to i64
  %v34 = mul nsw i64 %v32, 4
  %v35 = getelementptr inbounds i8, ptr %slot0, i64 %v34
  %v36 = load i32, ptr %v35, align 4
  %v37 = add i32 %v26, %v36
  br label %block6

block6:                                           ; preds = %block5
  br label %block4

block7:                                           ; preds = %block4
  %v42 = shl i32 -2147483648, 1
  %v44 = call i32 (ptr, ...) @printf(ptr noundef @.str.2, i64 noundef 12, i32 noundef %v26, i32 noundef 0, i32 noundef -2147483648, i32 noundef %v42)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
