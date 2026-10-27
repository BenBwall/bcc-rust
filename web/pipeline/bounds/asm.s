	.text
	.text
	.globl	main
	.p2align	4, 0x90
	.type	main,@function
main:
	.cfi_startproc
.LBB0_0:
	movl	$2, -16(%rsp)
	movl	$4, -12(%rsp)
	movl	$6, -8(%rsp)
	movl	-4(%rsp), %eax
	.cfi_remember_state
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	main, .-main
	.section	.note.GNU-stack,"",@progbits
