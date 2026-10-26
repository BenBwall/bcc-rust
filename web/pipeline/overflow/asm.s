	.text
	.text
	.globl	main
	.p2align	4, 0x90
	.type	main,@function
main:
	.cfi_startproc
.LBB0_0:
	movl	$0, %eax
	movl	$2147483646, %ecx
.LBB0_1:
	cmpl	$3, %eax
	jge	.LBB0_4
	.p2align	4
.LBB0_2:
	addl	$1, %ecx
.LBB0_3:
	addl	$1, %eax
	cmpl	$3, %eax
	jl	.LBB0_2
.LBB0_4:
	movq	%rcx, %rax
	.cfi_remember_state
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	main, .-main
	.section	.note.GNU-stack,"",@progbits
