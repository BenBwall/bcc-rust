; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@counter = global [4 x i8] zeroinitializer, align 4
@.str = internal constant [10 x i8] c"SQUARE(3)\00", align 1

define i32 @main() {
block0:
  %v3 = icmp sgt i32 9, 1073741823
  br i1 %v3, label %block1, label %block5

block1:                                           ; preds = %block0
  br label %block2

block2:                                           ; preds = %block1
  br label %block3

block3:                                           ; preds = %block2
  br label %block4

block4:                                           ; preds = %block3
  br label %block5

block5:                                           ; preds = %block4, %block0
  %v6 = phi i32 [ 9, %block0 ], [ 1073741823, %block4 ]
  %v9 = load i32, ptr @counter, align 4
  %v11 = add nsw i32 %v9, 1
  store i32 %v11, ptr @counter, align 4
  %v13 = load i32, ptr @counter, align 4
  %v14 = add nsw i32 %v6, %v13
  %v16 = load i8, ptr @.str, align 1
  %v17 = sext i8 %v16 to i32
  %v19 = icmp eq i32 %v17, 83
  %v20 = zext i1 %v19 to i32
  %v21 = add nsw i32 %v14, %v20
  ret i32 %v21
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
