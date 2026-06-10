/*======================================================================
 * x86_64 Exception Stubs — Assembly glue for IDT handlers
 *======================================================================
 *
 * These stubs are called from the IDT. They:
 * 1. Save all registers
 * 2. Call the Rust handler
 * 3. Restore registers
 * 4. IRETQ back to interrupted context
 *
 * The IDT entries point to these stubs with appropriate IST (stack switching)
 *======================================================================*/

.section .text
.code64

/* Macro to save all general-purpose registers */
.macro SAVE_GPRS
    pushq %r15
    pushq %r14
    pushq %r13
    pushq %r12
    pushq %r11
    pushq %r10
    pushq %r9
    pushq %r8
    pushq %rsi
    pushq %rdi
    pushq %rbp
    pushq %rbx
    pushq %rdx
    pushq %rcx
    pushq %rax
.endm

/* Macro to restore all general-purpose registers */
.macro RESTORE_GPRS
    popq %rax
    popq %rcx
    popq %rdx
    popq %rbx
    popq %rbp
    popq %rdi
    popq %rsi
    popq %r8
    popq %r9
    popq %r10
    popq %r11
    popq %r12
    popq %r13
    popq %r14
    popq %r15
.endm

/*======================================================================
 * Exception stubs WITHOUT error code (push dummy error code for stack alignment)
 *======================================================================*/
.macro EXCEPTION_STUB_NO_ERR handler_name vector_num
.global \handler_name\()_stub
\handler_name\()_stub:
    /* Push vector number */
    pushq $\vector_num
    /* Push dummy error code for stack alignment */
    pushq $0
    SAVE_GPRS
    /* RDI = pointer to exception frame (current RSP) */
    movq %rsp, %rdi
    call \handler_name
    RESTORE_GPRS
    /* Pop dummy error code and vector */
    addq $16, %rsp
    iretq
.endm

/*======================================================================
 * Exception stubs WITH error code (CPU pushes it automatically)
 *======================================================================*/
.macro EXCEPTION_STUB_WITH_ERR handler_name vector_num
.global \handler_name\()_stub
\handler_name\()_stub:
    /* CPU has pushed error code */
    /* Push vector number */
    pushq $\vector_num
    SAVE_GPRS
    /* RDI = pointer to exception frame (current RSP) */
    movq %rsp, %rdi
    call \handler_name
    RESTORE_GPRS
    /* Pop vector */
    addq $8, %rsp
    /* Pop error code (already on stack from CPU) */
    iretq
.endm

/*======================================================================
 * Generate stubs for all exceptions
 *======================================================================*/

/* No error code */
EXCEPTION_STUB_NO_ERR handle_divide_error 0
EXCEPTION_STUB_NO_ERR handle_debug 1
EXCEPTION_STUB_NO_ERR handle_nmi 2
EXCEPTION_STUB_NO_ERR handle_breakpoint 3
EXCEPTION_STUB_NO_ERR handle_overflow 4
EXCEPTION_STUB_NO_ERR handle_bound_range 5
EXCEPTION_STUB_NO_ERR handle_invalid_opcode 6
EXCEPTION_STUB_NO_ERR handle_device_not_available 7

/* With error code */
EXCEPTION_STUB_WITH_ERR handle_double_fault 8
EXCEPTION_STUB_WITH_ERR handle_invalid_tss 10
EXCEPTION_STUB_WITH_ERR handle_segment_not_present 11
EXCEPTION_STUB_WITH_ERR handle_stack_segment_fault 12
EXCEPTION_STUB_WITH_ERR handle_general_protection 13
EXCEPTION_STUB_WITH_ERR handle_page_fault 14

/* No error code (15 reserved) */
EXCEPTION_STUB_NO_ERR handle_x87_fpu 16
EXCEPTION_STUB_WITH_ERR handle_alignment_check 17
EXCEPTION_STUB_NO_ERR handle_machine_check 18
EXCEPTION_STUB_NO_ERR handle_simd 19
EXCEPTION_STUB_NO_ERR handle_virtualization 20
EXCEPTION_STUB_WITH_ERR handle_control_protection 21

/*======================================================================
 * Syscall stub (INT 0x80) - no error code
 *======================================================================*/
.global syscall_entry_stub
syscall_entry_stub:
    pushq $0x80
    pushq $0
    SAVE_GPRS
    movq %rsp, %rdi
    call handle_syscall
    RESTORE_GPRS
    addq $16, %rsp
    iretq

/*======================================================================
 * Hardware IRQ stubs (for LAPIC/IOAPIC)
 * These are used for vectors 0x20-0xFF
 *======================================================================*/
.macro IRQ_STUB vector
.global irq\vector\()_stub
irq\vector\()_stub:
    pushq $\vector
    pushq $0
    SAVE_GPRS
    movq %rsp, %rdi
    call handle_irq
    RESTORE_GPRS
    addq $16, %rsp
    iretq
.endm

/* Generate IRQ stubs for vectors 0x20-0x2F (legacy IRQ0-15) */
IRQ_STUB 0x20
IRQ_STUB 0x21
IRQ_STUB 0x22
IRQ_STUB 0x23
IRQ_STUB 0x24
IRQ_STUB 0x25
IRQ_STUB 0x26
IRQ_STUB 0x27
IRQ_STUB 0x28
IRQ_STUB 0x29
IRQ_STUB 0x2A
IRQ_STUB 0x2B
IRQ_STUB 0x2C
IRQ_STUB 0x2D
IRQ_STUB 0x2E
IRQ_STUB 0x2F

/* Timer interrupt (LAPIC timer, vector 0xEF) */
IRQ_STUB 0xEF

/* Spurious interrupt (vector 0xFF) */
IRQ_STUB 0xFF

/*======================================================================
 * Generic IRQ handler (called from all IRQ stubs)
 *======================================================================*/
.global handle_irq
.type handle_irq, @function
handle_irq:
    /* RDI = pointer to exception frame */
    /* Vector is at [RSP + 8] after SAVE_GPRS (since we push vector then dummy) */
    /* Actually vector is at [RSP + 8*15 + 8] = [RSP + 128] after SAVE_GPRS */
    /* Let's extract it from the frame */
    movq 128(%rdi), %rsi    /* vector */
    call irq_dispatch
    ret

/* irq_dispatch is a Rust function that routes to the appropriate handler */
.global irq_dispatch
.type irq_dispatch, @function
irq_dispatch:
    /* This will be implemented in Rust */
    ret