// LobsterOS AArch64 start.s - AArch64 entry point for NCC programs

.section .text
.global _start
.type _start, %function

_start:
    // Stack setup for AArch64
    // x0 = argc (at sp)
    // x1 = argv (sp + 8)
    // x2 = envp (sp + 8 + argc*8 + 8)
    
    ldr x0, [sp], #8        // argc, advance stack
    mov x1, sp              // argv
    add x2, x1, x0, lsl #3  // argv + argc*8
    add x2, x2, #8          // skip NULL terminator -> envp
    
    ldr x12, =environ
    str x2, [x12]           // environ = envp

    bl main                  // call main
    
    // Exit with return value
    mov x0, x0
    bl __neatlibc_exit
    
    // Should not return - exit syscall
    mov x8, #1              // SYS_EXIT
    svc #0
    
.size _start, .-_start

.global environ
.bss
.align 8
environ:
    .quad 0