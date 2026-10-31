; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

define i32 @square(i32 noundef %v0) {
block0:
  %v1 = mul nsw i32 %v0, %v0
  ret i32 %v1
}

define i32 @main() {
block0:
  %v2 = add nsw i32 3, 1
  %v3 = call i32 @square(i32 noundef %v2)
  ret i32 %v3
}

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
