; ModuleID = 'bcc'
source_filename = "bcc"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@calls = global [4 x i8] zeroinitializer, align 4
@origin = global [8 x i8] c"\01\00\00\00\02\00\00\00", align 4
@moved.total = internal global [4 x i8] zeroinitializer, align 4

define i64 @moved(i64 %v15, i32 noundef %v2) {
block0:
  %slot0 = alloca [8 x i8], align 8
  %v0 = alloca [16 x i8], align 8
  store i64 %v15, ptr %slot0, align 8
  %v5 = load i32, ptr @moved.total, align 4
  %v6 = add nsw i32 %v5, %v2
  store i32 %v6, ptr @moved.total, align 4
  %v9 = load i32, ptr @calls, align 4
  %v11 = add nsw i32 %v9, 1
  store i32 %v11, ptr @calls, align 4
  %v12 = load i32, ptr %slot0, align 4
  %v13 = add nsw i32 %v12, %v2
  store i32 %v13, ptr %slot0, align 4
  call void @llvm.memcpy.p0.p0.i64(ptr align 4 %v0, ptr align 4 %slot0, i64 8, i1 false)
  %0 = load <8 x i8>, ptr %v0, align 8
  %1 = freeze <8 x i8> %0
  %inst18.ret = bitcast <8 x i8> %1 to i64
  ret i64 %inst18.ret
}

define i32 @main() {
block0:
  %slot0 = alloca [8 x i8], align 4
  %slot1 = alloca [8 x i8], align 4
  %slot2 = alloca [8 x i8], align 4
  %slot3 = alloca [8 x i8], align 4
  %slot4 = alloca [8 x i8], align 4
  %slot5 = alloca [8 x i8], align 4
  %slot6 = alloca [8 x i8], align 4
  call void @llvm.memmove.p0.p0.i64(ptr align 4 %slot0, ptr align 4 @origin, i64 8, i1 false)
  call void @llvm.memcpy.p0.p0.i64(ptr align 4 %slot2, ptr align 4 %slot0, i64 8, i1 false)
  %v53 = load i32, ptr %slot2, align 4
  %v54 = freeze i32 %v53
  %v55 = zext i32 %v54 to i64
  %v57 = getelementptr inbounds i8, ptr %slot2, i64 4
  %v58 = load i32, ptr %v57, align 4
  %v59 = freeze i32 %v58
  %v60 = zext i32 %v59 to i64
  %v62 = shl i64 %v60, 32
  %v63 = or i64 %v55, %v62
  %inst21.ret = call i64 @moved(i64 %v63, i32 noundef 3)
  store i64 %inst21.ret, ptr %slot3, align 4
  call void @llvm.memmove.p0.p0.i64(ptr align 4 %slot1, ptr align 4 %slot3, i64 8, i1 false)
  %v12 = getelementptr inbounds i8, ptr %slot1, i64 4
  %v14 = getelementptr inbounds i8, ptr %slot1, i64 4
  %v15 = load i32, ptr %v14, align 4
  %v17 = add nsw i32 %v15, 10
  store i32 %v17, ptr %v12, align 4
  %v18 = load i32, ptr %slot1, align 4
  %v20 = mul nsw i32 %v18, 2
  store i32 %v20, ptr %slot1, align 4
  call void @llvm.memset.p0.i64(ptr align 4 %slot4, i8 0, i64 4, i1 false)
  %v27 = getelementptr inbounds i8, ptr %slot4, i64 4
  store i32 5, ptr %v27, align 4
  store i8 7, ptr %slot4, align 4
  call void @llvm.memcpy.p0.p0.i64(ptr align 4 %slot5, ptr align 4 %slot1, i64 8, i1 false)
  %v64 = load i32, ptr %slot5, align 4
  %v65 = freeze i32 %v64
  %v66 = zext i32 %v65 to i64
  %v68 = getelementptr inbounds i8, ptr %slot5, i64 4
  %v69 = load i32, ptr %v68, align 4
  %v70 = freeze i32 %v69
  %v71 = zext i32 %v70 to i64
  %v73 = shl i64 %v71, 32
  %v74 = or i64 %v66, %v73
  %inst69.ret = call i64 @moved(i64 %v74, i32 noundef 4)
  store i64 %inst69.ret, ptr %slot6, align 4
  call void @llvm.memmove.p0.p0.i64(ptr align 4 %slot1, ptr align 4 %slot6, i64 8, i1 false)
  %v39 = load i32, ptr %slot1, align 4
  %v42 = getelementptr inbounds i8, ptr %slot1, i64 4
  %v43 = load i32, ptr %v42, align 4
  %v44 = add nsw i32 %v39, %v43
  %v46 = load i32, ptr @calls, align 4
  %v47 = add nsw i32 %v44, %v46
  %v50 = getelementptr inbounds i8, ptr %slot4, i64 4
  %v51 = load i32, ptr %v50, align 4
  %v52 = add nsw i32 %v47, %v51
  ret i32 %v52
}

; Function Attrs: nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias writeonly captures(none), ptr noalias readonly captures(none), i64, i1 immarg) #0

; Function Attrs: nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memmove.p0.p0.i64(ptr writeonly captures(none), ptr readonly captures(none), i64, i1 immarg) #0

; Function Attrs: nocallback nofree nosync nounwind willreturn memory(argmem: write)
declare void @llvm.memset.p0.i64(ptr writeonly captures(none), i8, i64, i1 immarg) #1

attributes #0 = { nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }
attributes #1 = { nocallback nofree nosync nounwind willreturn memory(argmem: write) }

!llvm.module.flags = !{!0, !1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 7, !"PIE Level", i32 2}
