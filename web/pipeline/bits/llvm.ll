; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [28 x i8] c"mode %x: read %d, write %d\0A\00", align 1

define i32 @main() #0 {
block0:
  %v2 = or i32 0, 4
  %v4 = or i32 %v2, 3
  %v6 = and i32 %v4, -3
  %v8 = xor i32 %v6, 1
  %v10 = and i32 %v8, 4
  %v12 = icmp ne i32 %v10, 0
  %v13 = zext i1 %v12 to i32
  %v16 = and i32 %v8, 2
  %v18 = icmp ne i32 %v16, 0
  %v19 = zext i1 %v18 to i32
  %v20 = call i32 (ptr, ...) @printf(ptr noundef @.str, i32 noundef %v8, i32 noundef %v13, i32 noundef %v19)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

attributes #0 = { "frame-pointer"="all" }

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
