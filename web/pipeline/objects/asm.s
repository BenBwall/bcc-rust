	.text
	.text
	.globl	bump
	.p2align	4, 0x90
	.type	bump,@function
bump:
	.cfi_startproc
.LBB0_0:
	movl	(%rdi), %eax
	addl	$1, %eax
	movl	%eax, (%rdi)
	movl	(%rdi), %eax
	.cfi_remember_state
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	bump, .-bump
	.text
	.globl	main
	.p2align	4, 0x90
	.type	main,@function
main:
	.cfi_startproc
	pushq	%rbx
	.cfi_def_cfa_offset 16
	.cfi_offset %rbx, -16
	pushq	%r12
	.cfi_def_cfa_offset 24
	.cfi_offset %r12, -24
	pushq	%r13
	.cfi_def_cfa_offset 32
	.cfi_offset %r13, -32
	subq	$16, %rsp
	.cfi_def_cfa_offset 48
.LBB1_0:
	movl	$2, 0(%rsp)
	movl	$4, 4(%rsp)
	movl	$6, 8(%rsp)
	movl	$0, %ebx
	leaq	0(%rsp), %r12
	movl	$0, %r13d
.LBB1_1:
	cmpl	$3, %ebx
	jge	.LBB1_4
	.p2align	4
.LBB1_2:
	movq	%r12, %rdi
	callq	bump
	addl	%eax, %r13d
	leaq	4(%r12), %r12
.LBB1_3:
	addl	$1, %ebx
	cmpl	$3, %ebx
	jl	.LBB1_2
.LBB1_4:
	movq	%r13, %rax
	.cfi_remember_state
	addq	$16, %rsp
	.cfi_def_cfa_offset 32
	popq	%r13
	.cfi_def_cfa_offset 24
	popq	%r12
	.cfi_def_cfa_offset 16
	popq	%rbx
	.cfi_def_cfa_offset 8
	retq
	.cfi_restore_state
	.cfi_endproc
	.size	main, .-main
	.section	.note.GNU-stack,"",@progbits
