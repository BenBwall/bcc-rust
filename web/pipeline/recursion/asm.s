	.text
	.text
	.globl	factorial
	.p2align	4, 0x90
	.type	factorial,@function
factorial:
	.cfi_startproc
	pushq	%rbx
	.cfi_def_cfa_offset 16
	.cfi_offset %rbx, -16
.LBB0_0:
	movq	%rdi, %rbx
	cmpl	$1, %ebx
	jg	.LBB0_2
.LBB0_1:
	movq	$1, %rax
	.cfi_remember_state
	popq	%rbx
	.cfi_def_cfa_offset 8
	retq
	.cfi_restore_state
.LBB0_2:
	leaq	-1(%rbx), %rax
	movq	%rax, %rdi
	callq	factorial
	imull	%eax, %ebx
	movq	%rbx, %rax
	.cfi_remember_state
	popq	%rbx
	.cfi_def_cfa_offset 8
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	factorial, .-factorial
	.text
	.globl	main
	.p2align	4, 0x90
	.type	main,@function
main:
	.cfi_startproc
	subq	$8, %rsp
	.cfi_def_cfa_offset 16
.LBB1_0:
	movq	$4, %rdi
	callq	factorial
	.cfi_remember_state
	addq	$8, %rsp
	.cfi_def_cfa_offset 8
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	main, .-main
	.section	.note.GNU-stack,"",@progbits
