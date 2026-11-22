; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [5 x i8] c"walk\00", align 1
@.str.1 = internal constant [17 x i8] c"%s has %d chars\0A\00", align 1

define i32 @length(ptr noundef %v0) {
block0:
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v3 = phi ptr [ %v0, %block0 ], [ %v6, %block2 ]
  %v4 = load i8, ptr %v3, align 1
  %v5 = icmp ne i8 %v4, 0
  br i1 %v5, label %block2, label %block3

block2:                                           ; preds = %block1
  %v6 = getelementptr inbounds i8, ptr %v3, i64 1
  br label %block1

block3:                                           ; preds = %block1
  %v7 = ptrtoint ptr %v3 to i64
  %v8 = ptrtoint ptr %v0 to i64
  %v9 = sub i64 %v7, %v8
  %v10 = trunc i64 %v9 to i32
  ret i32 %v10
}

define i32 @main() {
block0:
  %slot0 = alloca [8 x i8], align 1
  br label %block1

block1:                                           ; preds = %block1, %block0
  %v4 = phi ptr [ %slot0, %block0 ], [ %v6, %block1 ]
  %v5 = phi ptr [ @.str, %block0 ], [ %v7, %block1 ]
  %v6 = getelementptr inbounds i8, ptr %v4, i64 1
  %v7 = getelementptr inbounds i8, ptr %v5, i64 1
  %v8 = load i8, ptr %v5, align 1
  store i8 %v8, ptr %v4, align 1
  %v9 = sext i8 %v8 to i32
  %v10 = icmp ne i32 %v9, 0
  br i1 %v10, label %block1, label %block2

block2:                                           ; preds = %block1
  br label %block3

block3:                                           ; preds = %block4, %block2
  %v13 = phi ptr [ %slot0, %block2 ], [ %v16, %block4 ]
  %v14 = load i8, ptr %v13, align 1
  %v15 = icmp ne i8 %v14, 0
  br i1 %v15, label %block4, label %block5

block4:                                           ; preds = %block3
  %v16 = getelementptr inbounds i8, ptr %v13, i64 1
  br label %block3

block5:                                           ; preds = %block3
  %v17 = ptrtoint ptr %v13 to i64
  %v18 = ptrtoint ptr %slot0 to i64
  %v19 = sub i64 %v17, %v18
  %v20 = trunc i64 %v19 to i32
  %v21 = call i32 (ptr, ...) @printf(ptr noundef @.str.1, ptr noundef %slot0, i32 noundef %v20)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
