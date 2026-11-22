; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [5 x i8] c"walk\00", align 1
@.str.1 = internal constant [17 x i8] c"%s has %d chars\0A\00", align 1

define i32 @length(ptr noundef %v0) #0 {
block0:
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v1 = phi ptr [ %v0, %block0 ], [ %v6, %block2 ]
  %v2 = load i8, ptr %v1, align 1
  %v4 = icmp ne i8 %v2, 0
  br i1 %v4, label %block2, label %block3

block2:                                           ; preds = %block1
  %v6 = getelementptr inbounds i8, ptr %v1, i64 1
  br label %block1

block3:                                           ; preds = %block1
  %v7 = ptrtoint ptr %v1 to i64
  %v8 = ptrtoint ptr %v0 to i64
  %v9 = sub i64 %v7, %v8
  %v10 = trunc i64 %v9 to i32
  ret i32 %v10
}

define i32 @main() #0 {
block0:
  %slot0 = alloca [8 x i8], align 1
  br label %block1

block1:                                           ; preds = %block2, %block0
  %v2 = phi ptr [ %slot0, %block0 ], [ %v5, %block2 ]
  %v3 = phi ptr [ @.str, %block0 ], [ %v7, %block2 ]
  %v5 = getelementptr inbounds i8, ptr %v2, i64 1
  %v7 = getelementptr inbounds i8, ptr %v3, i64 1
  %v8 = load i8, ptr %v3, align 1
  store i8 %v8, ptr %v2, align 1
  %v9 = sext i8 %v8 to i32
  %v11 = icmp ne i32 %v9, 0
  br i1 %v11, label %block2, label %block3

block2:                                           ; preds = %block1
  br label %block1

block3:                                           ; preds = %block1
  %v15 = call i32 @length(ptr noundef %slot0)
  %v16 = call i32 (ptr, ...) @printf(ptr noundef @.str.1, ptr noundef %slot0, i32 noundef %v15)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

attributes #0 = { "frame-pointer"="all" }

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
