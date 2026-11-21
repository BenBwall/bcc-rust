; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [28 x i8] c"mode %x: read %d, write %d\0A\00", align 1

define i32 @main() {
block0:
  %v4 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef 4, i32 noundef 1, i32 noundef 0)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
