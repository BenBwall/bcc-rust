; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@.str = internal constant [21 x i8] c"total / count is %f\0A\00", align 1
@.str.1 = internal constant [29 x i8] c"(double)total / count is %f\0A\00", align 1
@.str.2 = internal constant [24 x i8] c"0.1 + 0.2 == 0.3 is %d\0A\00", align 1
@.str.3 = internal constant [20 x i8] c"0.1 + 0.2 is %.17g\0A\00", align 1
@.str.4 = internal constant [21 x i8] c"ten 0.1s make %.17g\0A\00", align 1
@.str.5 = internal constant [17 x i8] c"(int)%.2f is %d\0A\00", align 1

define i32 @main() {
block0:
  %v2 = sdiv i32 7, 2
  %v3 = sitofp i32 %v2 to double
  %v5 = call i32 (ptr, ...) @printf(ptr noundef @.str, double noundef %v3)
  %v8 = fdiv double 7.000000e+00, 2.000000e+00
  %v10 = call i32 (ptr, ...) @printf(ptr noundef @.str.1, double noundef %v8)
  %v14 = fcmp oeq double f0x3FD3333333333334, 3.000000e-01
  %v15 = zext i1 %v14 to i32
  %v16 = call i32 (ptr, ...) @printf(ptr noundef @.str.2, i32 noundef %v15)
  %v18 = call i32 (ptr, ...) @printf(ptr noundef @.str.3, double noundef f0x3FD3333333333334)
  br label %block1

block1:                                           ; preds = %block3, %block0
  %v21 = phi i32 [ 0, %block0 ], [ %v28, %block3 ]
  %v22 = phi double [ 0.000000e+00, %block0 ], [ %v26, %block3 ]
  %v24 = icmp slt i32 %v21, 10
  br i1 %v24, label %block2, label %block4

block2:                                           ; preds = %block1
  %v26 = fadd double %v22, 1.000000e-01
  br label %block3

block3:                                           ; preds = %block2
  %v28 = add nsw i32 %v21, 1
  br label %block1

block4:                                           ; preds = %block1
  %v30 = call i32 (ptr, ...) @printf(ptr noundef @.str.4, double noundef %v22)
  %v32 = fptosi double 2.990000e+00 to i32
  %v34 = call i32 (ptr, ...) @printf(ptr noundef @.str.5, double noundef 2.990000e+00, i32 noundef %v32)
  ret i32 0
}

declare i32 @printf(ptr noundef, ...)

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
