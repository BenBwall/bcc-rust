	.text
	.text
	.globl	main
	.p2align	4, 0x90
	.type	main,@function
main:
	.cfi_startproc
.LBB0_0:
	movl	$100, %eax
	addl	$100, %eax
	movl	$100, %ecx
	addl	$100, %ecx
	addl	$1, %ecx
	movsbl	%cl, %ecx
	subl	%ecx, %eax
	.cfi_remember_state
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	main, .-main
	.section	.note.GNU-stack,"",@progbits
