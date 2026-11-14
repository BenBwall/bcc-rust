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
  %v4 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef -1)
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v6 = phi i32 [ 2, %block0 ], [ %v8, %block2 ]
  %v7 = icmp ult i32 %v6, 3
  br i1 %v7, label %block2, label %block3

block2:                                           ; preds = %block1
  %v8 = sub i32 %v6, 1
  br label %block1

block3:                                           ; preds = %block1
  %v10 = call i32 (ptr, ...) @printf(ptr noundef @.str.1, i32 noundef %v6)
  store i32 4, ptr %slot0, align 4
  %v15 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 5, ptr %v15, align 4
  %v18 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 6, ptr %v18, align 4
  br label %block4

block4:                                           ; preds = %block5, %block3
  %v20 = phi i32 [ 3, %block3 ], [ %v22, %block5 ]
  %v21 = phi i32 [ 0, %block3 ], [ %v28, %block5 ]
  %v22 = sub i32 %v20, 1
  %v23 = icmp ugt i32 %v20, 0
  br i1 %v23, label %block5, label %block6

block5:                                           ; preds = %block4
  %v24 = zext i32 %v22 to i64
  %v25 = mul nsw i64 %v24, 4
  %v26 = getelementptr inbounds i8, ptr %slot0, i64 %v25
  %v27 = load i32, ptr %v26, align 4
  %v28 = add i32 %v21, %v27
  br label %block4

block6:                                           ; preds = %block4
  %v32 = call i32 (ptr, ...) @printf(ptr noundef @.str.2, i64 noundef 12, i32 noundef %v21, i32 noundef 0, i32 noundef -2147483648, i32 noundef 0)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
