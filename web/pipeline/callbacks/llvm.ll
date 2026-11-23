; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [7 x i8] c"%d %d\0A\00", align 1

define i32 @twice(i32 noundef %v0) #0 {
block0:
  %v2 = mul nsw i32 2, %v0
  ret i32 %v2
}

define i32 @square(i32 noundef %v0) #0 {
block0:
  %v1 = mul nsw i32 %v0, %v0
  ret i32 %v1
}

define i32 @sum_of(ptr noundef %v0, i32 noundef %v1, ptr noundef %v2) #0 {
block0:
  br label %block1

block1:                                           ; preds = %block3, %block0
  %v5 = phi i32 [ 0, %block0 ], [ %v16, %block3 ]
  %v6 = phi i32 [ 0, %block0 ], [ %v14, %block3 ]
  %v7 = icmp slt i32 %v5, %v1
  br i1 %v7, label %block2, label %block4

block2:                                           ; preds = %block1
  %v8 = sext i32 %v5 to i64
  %v10 = mul nsw i64 %v8, 4
  %v11 = getelementptr inbounds i8, ptr %v0, i64 %v10
  %v12 = load i32, ptr %v11, align 4
  %v13 = call i32 %v2(i32 noundef %v12)
  %v14 = add nsw i32 %v6, %v13
  br label %block3

block3:                                           ; preds = %block2
  %v16 = add nsw i32 %v5, 1
  br label %block1

block4:                                           ; preds = %block1
  ret i32 %v6
}

define i32 @main() #0 {
block0:
  %slot0 = alloca [8 x i8], align 4
  store i32 3, ptr %slot0, align 4
  %v5 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 4, ptr %v5, align 4
  %v9 = call i32 @sum_of(ptr noundef %slot0, i32 noundef 2, ptr noundef @twice)
  %v13 = call i32 @sum_of(ptr noundef %slot0, i32 noundef 2, ptr noundef @square)
  %v15 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef %v9, i32 noundef %v13)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

attributes #0 = { "frame-pointer"="all" }

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
