; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @bump(ptr noundef %v0) {
block0:
  %v1 = load i32, ptr %v0, align 4
  %v3 = add nsw i32 %v1, 1
  store i32 %v3, ptr %v0, align 4
  %v4 = load i32, ptr %v0, align 4
  ret i32 %v4
}

define i32 @main() {
block0:
  %slot0 = alloca [12 x i8], align 4
  store i32 2, ptr %slot0, align 4
  %v6 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 4, ptr %v6, align 4
  %v10 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 6, ptr %v10, align 4
  br label %block1

block1:                                           ; preds = %block3, %block0
  %v14 = phi i32 [ 0, %block0 ], [ %v25, %block3 ]
  %v15 = phi ptr [ %slot0, %block0 ], [ %v23, %block3 ]
  %v16 = phi i32 [ 0, %block0 ], [ %v20, %block3 ]
  %v18 = icmp slt i32 %v14, 3
  br i1 %v18, label %block2, label %block4

block2:                                           ; preds = %block1
  %v19 = call i32 @bump(ptr noundef %v15)
  %v20 = add nsw i32 %v16, %v19
  %v23 = getelementptr inbounds i8, ptr %v15, i64 4
  br label %block3

block3:                                           ; preds = %block2
  %v25 = add nsw i32 %v14, 1
  br label %block1

block4:                                           ; preds = %block1
  ret i32 %v16
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
