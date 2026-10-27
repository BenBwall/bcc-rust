; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @sum(ptr noundef %v0, ptr noundef %v1) {
block0:
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v3 = phi ptr [ %v0, %block0 ], [ %v10, %block2 ]
  %v4 = phi i32 [ 0, %block0 ], [ %v7, %block2 ]
  %v5 = icmp ne ptr %v3, %v1
  br i1 %v5, label %block2, label %block3

block2:                                           ; preds = %block1
  %v6 = load i32, ptr %v3, align 4
  %v7 = add nsw i32 %v4, %v6
  %v10 = getelementptr inbounds i8, ptr %v3, i64 4
  br label %block1

block3:                                           ; preds = %block1
  ret i32 %v4
}

define i32 @main() {
block0:
  %slot0 = alloca [16 x i8], align 16
  store i32 2, ptr %slot0, align 16
  %v5 = getelementptr inbounds i8, ptr %slot0, i64 4
  store i32 3, ptr %v5, align 4
  %v9 = getelementptr inbounds i8, ptr %slot0, i64 8
  store i32 5, ptr %v9, align 8
  %v13 = getelementptr inbounds i8, ptr %slot0, i64 12
  store i32 7, ptr %v13, align 4
  %v18 = getelementptr inbounds i8, ptr %slot0, i64 16
  %v19 = call i32 @sum(ptr noundef %slot0, ptr noundef %v18)
  ret i32 %v19
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
