; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @bump(ptr noundef %v0) {
block0:
  %v1 = load i32, ptr %v0, align 4
  %v3 = add nsw i32 %v1, 1
  store i32 %v3, ptr %v0, align 4
  ret i32 %v3
}

define i32 @main() {
block0:
  %slot0 = alloca [12 x i8], align 4
  store i32 2, ptr %slot0, align 4
  %v5 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 4, ptr %v5, align 4
  %v8 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 6, ptr %v8, align 4
  br label %block1

block1:                                           ; preds = %block3, %block0
  %v11 = phi i32 [ 0, %block0 ], [ %v19, %block3 ]
  %v12 = phi ptr [ %slot0, %block0 ], [ %v18, %block3 ]
  %v13 = phi i32 [ 0, %block0 ], [ %v17, %block3 ]
  %v14 = icmp slt i32 %v11, 3
  br i1 %v14, label %block3, label %block2

block2:                                           ; preds = %block1
  ret i32 %v13

block3:                                           ; preds = %block1
  %v15 = load i32, ptr %v12, align 4
  %v16 = add nsw i32 %v15, 1
  store i32 %v16, ptr %v12, align 4
  %v17 = add nsw i32 %v13, %v16
  %v18 = getelementptr inbounds i8, ptr %v12, i64 4
  %v19 = add nsw i32 %v11, 1
  br label %block1
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
