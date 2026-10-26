; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @sum(ptr noundef %v0, ptr noundef %v1) {
block0:
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v4 = phi ptr [ %v0, %block0 ], [ %v9, %block2 ]
  %v5 = phi i32 [ 0, %block0 ], [ %v8, %block2 ]
  %v6 = icmp ne ptr %v4, %v1
  br i1 %v6, label %block2, label %block3

block2:                                           ; preds = %block1
  %v7 = load i32, ptr %v4, align 4
  %v8 = add nsw i32 %v5, %v7
  %v9 = getelementptr inbounds i8, ptr %v4, i64 4
  br label %block1

block3:                                           ; preds = %block1
  ret i32 %v5
}

define i32 @main() {
block0:
  %slot0 = alloca [16 x i8], align 16
  store i32 2, ptr %slot0, align 16
  %v4 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 3, ptr %v4, align 4
  %v7 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 5, ptr %v7, align 8
  %v10 = getelementptr inbounds i8, ptr %slot0, i64 12
  store i32 7, ptr %v10, align 4
  %v12 = getelementptr inbounds i8, ptr %slot0, i64 16
  br label %block2

block1:                                           ; preds = %block2
  %v14 = phi i32 [ %v16, %block2 ]
  ret i32 %v14

block2:                                           ; preds = %block3, %block0
  %v15 = phi ptr [ %slot0, %block0 ], [ %v20, %block3 ]
  %v16 = phi i32 [ 0, %block0 ], [ %v19, %block3 ]
  %v17 = icmp ne ptr %v15, %v12
  br i1 %v17, label %block3, label %block1

block3:                                           ; preds = %block2
  %v18 = load i32, ptr %v15, align 4
  %v19 = add nsw i32 %v16, %v18
  %v20 = getelementptr inbounds i8, ptr %v15, i64 4
  br label %block2
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
