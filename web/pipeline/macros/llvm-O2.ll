; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@counter = global [4 x i8] zeroinitializer, align 4
@.str = internal constant [10 x i8] c"SQUARE(3)\00", align 1

define i32 @main() {
block0:
  %v3 = load i32, ptr @counter, align 4
  %v5 = add nsw i32 %v3, 1
  store i32 %v5, ptr @counter, align 4
  %v6 = add nsw i32 9, %v5
  %v7 = load i8, ptr @.str, align 1
  %v8 = sext i8 %v7 to i32
  %v10 = icmp eq i32 %v8, 83
  %v11 = zext i1 %v10 to i32
  %v12 = add nsw i32 %v6, %v11
  ret i32 %v12
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
