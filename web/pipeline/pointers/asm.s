	.text
	.text
	.globl	sum
	.p2align	4, 0x90
	.type	sum,@function
sum:
	.cfi_startproc
.LBB0_0:
	movl	$0, %eax
.LBB0_1:
	cmpq	%rsi, %rdi
	je	.LBB0_3
	.p2align	4
.LBB0_2:
	movl	(%rdi), %ecx
	addl	%ecx, %eax
	leaq	4(%rdi), %rdi
	cmpq	%rsi, %rdi
	jne	.LBB0_2
.LBB0_3:
	.cfi_remember_state
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	sum, .-sum
	.text
	.globl	main
	.p2align	4, 0x90
	.type	main,@function
main:
	.cfi_startproc
	subq	$24, %rsp
	.cfi_def_cfa_offset 32
.LBB1_0:
	movl	$2, 0(%rsp)
	movl	$3, 4(%rsp)
	movl	$5, 8(%rsp)
	movl	$7, 12(%rsp)
	leaq	0(%rsp), %rdi
	leaq	16(%rsp), %rsi
	callq	sum
	.cfi_remember_state
	addq	$24, %rsp
	.cfi_def_cfa_offset 8
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	main, .-main
	.section	.note.GNU-stack,"",@progbits
