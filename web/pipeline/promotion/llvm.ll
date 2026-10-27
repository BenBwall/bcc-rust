; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @main() {
block0:
  %v4 = add nsw i32 100, 100
  %v7 = add nsw i32 100, 100
  %v8 = trunc i32 %v7 to i8
  %v10 = add i8 %v8, 1
  %v11 = sext i8 %v10 to i32
  %v12 = sub nsw i32 %v4, %v11
  ret i32 %v12
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
