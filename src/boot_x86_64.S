/*======================================================================
 * LobsterOS x86_64 Boot Entry — Multiboot2 compliant
 *======================================================================
 *
 * This boot.S handles x86_64 bare-metal boot via Multiboot2 (GRUB).
 * The firmware/bootloader loads us at 1MB (0x100000) with:
 *   - rax = Multiboot2 magic (0x36d76289)
 *   - rbx = Physical address of Multiboot2 info structure
 *
 * We need to:
 * 1. Verify Multiboot2 magic
 * 2. Parse memory map from Multiboot2 info
 * 3. Set up page tables (identity map for low mem, higher-half kernel)
 * 4. Enable long mode (if not already)
 * 5. Zero BSS
 * 6. Set up stack
 * 7. Enable SSE/FPU
 * 8. Set up IDT (Interrupt Descriptor Table)
 * 9. Call Rust kernel_init(multiboot_info_ptr)
 *
 * Multiboot2 specification: https://www.gnu.org/software/grub/manual/multiboot2/multiboot2.html
 *======================================================================*/

/* Multiboot2 header — placed at the very beginning of the kernel */
.section .multiboot_header
.balign 8
.global multiboot_header
multiboot_header:
    /* Magic number */
    .long 0xe85250d6              /* MULTIBOOT2_HEADER_MAGIC */
    /* Architecture: 0 = i386 (but we're 64-bit, so use 0 for protected mode entry) */
    .long 0                       /* MULTIBOOT_ARCH_I386 */
    /* Header length (bytes) */
    .long header_end - multiboot_header
    /* Checksum: magic + arch + len + checksum = 0 */
    .long 0x100000000 - (0xe85250d6 + 0 + (header_end - multiboot_header))

    /* Required tags */

    /* End tag */
    .short 0                      /* type = 0 (end) */
    .short 0                      /* flags = 0 */
    .long 8                       /* size = 8 */

header_end:

/*======================================================================
 * Boot entry point
 *======================================================================*/
.section .text._start_x86_64
.global _start_x86_64
.type _start_x86_64, @function

_start_x86_64:
    /*------------------------------------------------------------------
     * rax = Multiboot2 magic (should be 0x36d76289)
     * rbx = Physical address of Multiboot2 info structure
     *------------------------------------------------------------------*/
    cmp     $0x36d76289, %rax
    jne     .Lmultiboot_fail

    /* Save Multiboot info pointer for later */
    mov     %rbx, %r12            /* r12 = mboot_info_ptr (preserved across calls) */

    /*------------------------------------------------------------------
     * We are already in 64-bit long mode (GRUB enters long mode before
     * jumping to the kernel entry point for x86_64 kernels).
     * Verify we're in long mode.
     *------------------------------------------------------------------*/
    mov     $0x80000001, %eax
    cpuid
    test    $(1 << 29), %edx      /* LM bit (bit 29) */
    jz      .Lno_long_mode

    /*------------------------------------------------------------------
     * Set up temporary page tables for identity mapping the first 2MB
     * and mapping the kernel at higher-half (0xFFFF_8000_0000_0000).
     * We'll build a 4-level page table (PML4 -> PDPT -> PD -> PT).
     *------------------------------------------------------------------*/

    /* Clear BSS first (we need a stack for this, so set up a minimal one) */
    /* Use the end of the first 2MB as temporary stack */
    mov     $0x200000, %rsp       /* 2MB stack top */

    /* Zero BSS */
    mov     __bss_start(%rip), %rdi
    mov     __bss_end(%rip), %rsi
    cmp     %rdi, %rsi
    jbe     .Lbss_done
.Lzero_bss:
    mov     $0, (%rdi)
    add     $8, %rdi
    cmp     %rdi, %rsi
    ja      .Lzero_bss
.Lbss_done:

    /*------------------------------------------------------------------
     * Build page tables for higher-half kernel mapping.
     * Kernel is linked at 1MB (0x100000) but we want it accessible at
     * 0xFFFF_8000_0010_0000 (higher-half direct map).
     *
     * Page table layout:
     * PML4[256] = PDPT (kernel higher-half)  -> 0xFFFF_8000_0000_0000
     * PML4[511] = PDPT (recursive map)       -> for page table management
     * PML4[0]   = PDPT (identity map low 1GB)
     *------------------------------------------------------------------*/

    /* Allocate page tables in the first 2MB (after kernel) */
    /* We'll place them at 0x100000 + kernel_size, aligned to 4KB */
    /* For simplicity, use fixed locations */
    mov     $0x200000, %rdi       /* Start of page table area */
    and     $~0xFFF, %rdi         /* Align to 4KB */

    /* PML4 at %rdi */
    mov     %rdi, %rax
    add     $0x1000, %rdi         /* Next page for PDPT */

    /* PDPT for higher-half kernel (PML4[256]) at %rdi */
    mov     %rdi, %rcx
    add     $0x1000, %rdi         /* Next page for PD (higher-half) */

    /* PD for higher-half kernel (1GB huge pages) at %rdi */
    mov     %rdi, %rdx
    add     $0x1000, %rdi         /* Next page for PD (identity) */

    /* PD for identity map (PML4[0]) at %rdi */
    mov     %rdi, %r8
    add     $0x1000, %rdi         /* Next page for PT (if needed) */

    /* Clear all page table pages */
    mov     $4, %r9               /* 4 pages to clear */
    mov     %rax, %r10            /* Save PML4 address */
.Lclear_pt_loop:
    mov     $0, (%rax)
    add     $8, %rax
    cmp     %rdi, %rax
    jb      .Lclear_pt_loop
    sub     $0x1000, %rax         /* Back to start of this page */
    add     $0x1000, %rax         /* Next page */
    dec     %r9
    jnz     .Lclear_pt_loop

    /* Restore PML4 address */
    mov     %r10, %rax

    /*------------------------------------------------------------------
     * Build PML4 entries
     *------------------------------------------------------------------*/
    /* PML4[0] = identity map PDPT (covers 0..1GB) */
    mov     %r8, %r11             /* PD for identity */
    or      $0x03, %r11           /* Present + Write */
    mov     %r11, (%rax)          /* PML4[0] */

    /* PML4[256] = higher-half PDPT (covers 0xFFFF_8000_0000_0000 ..) */
    mov     %rcx, %r11            /* PDPT for higher-half */
    or      $0x03, %r11
    mov     %r11, 2048(%rax)      /* PML4[256] = 256 * 8 */

    /* PML4[511] = recursive mapping (PML4 points to itself) */
    or      $0x03, %rax
    mov     %rax, 4088(%rax)      /* PML4[511] = 511 * 8 */

    /*------------------------------------------------------------------
     * Build PDPT for higher-half (PML4[256])
     * Maps 1GB regions. We need 1 entry for kernel at 0xFFFF_8000_0000_0000
     * PDPT[0] -> PD for higher-half kernel
     *------------------------------------------------------------------*/
    mov     %rdx, %r11            /* PD for higher-half */
    or      $0x03, %r11
    mov     %r11, (%rcx)          /* PDPT[0] */

    /*------------------------------------------------------------------
     * Build PD for higher-half kernel (1GB huge pages)
     * Each entry maps 2MB (huge page) or 1GB (if PS=1 in PD)
     * We'll use 2MB huge pages for the kernel region.
     * PD[0] maps 0xFFFF_8000_0000_0000 .. 0xFFFF_8000_0020_0000
     *------------------------------------------------------------------*/
    /* Kernel physical start is 0x100000 */
    mov     $0x100000, %r11
    or      $0x83, %r11           /* Present + Write + PageSize (2MB) + Global */
    mov     %r11, (%rdx)          /* PD[0] */

    /* Map next few 2MB pages for kernel (up to 16MB should be enough) */
    mov     $0x300000, %r11
    or      $0x83, %r11
    mov     %r11, 8(%rdx)         /* PD[1] */
    mov     $0x500000, %r11
    or      $0x83, %r11
    mov     %r11, 16(%rdx)        /* PD[2] */
    mov     $0x700000, %r11
    or      $0x83, %r11
    mov     %r11, 24(%rdx)        /* PD[3] */
    mov     $0x900000, %r11
    or      $0x83, %r11
    mov     %r11, 32(%rdx)        /* PD[4] */
    mov     $0xB00000, %r11
    or      $0x83, %r11
    mov     %r11, 40(%rdx)        /* PD[5] */
    mov     $0xD00000, %r11
    or      $0x83, %r11
    mov     %r11, 48(%rdx)        /* PD[6] */
    mov     $0xF00000, %r11
    or      $0x83, %r11
    mov     %r11, 56(%rdx)        /* PD[7] */

    /*------------------------------------------------------------------
     * Build PD for identity map (PML4[0])
     * Identity map first 1GB using 2MB huge pages
     *------------------------------------------------------------------*/
    xor     %r11, %r11
.Lid_pd_loop:
    cmp     $512, %r11            /* 512 * 2MB = 1GB */
    jge     .Lid_pd_done
    mov     %r11, %r10
    shl     $21, %r10             /* r10 = r11 * 2MB */
    or      $0x83, %r10           /* Present + Write + PageSize + Global */
    mov     %r10, (%r8, %r11, 8)  /* PD[r11] */
    inc     %r11
    jmp     .Lid_pd_loop
.Lid_pd_done:

    /*------------------------------------------------------------------
     * Load CR3 with PML4 physical address
     *------------------------------------------------------------------*/
    mov     %r10, %rax            /* PML4 physical address (from %r10 saved earlier) */
    /* Actually we need the physical address of PML4 */
    mov     %r10, %cr3            /* This won't work directly, need physical addr */

    /* The page tables are in identity-mapped low memory, so virtual = physical */
    mov     %r10, %cr3            /* Load PML4 */

    /*------------------------------------------------------------------
     * Enable long mode (should already be enabled by GRUB)
     * Set EFER.LME = 1 (bit 8)
     *------------------------------------------------------------------*/
    mov     $0xC0000080, %ecx     /* EFER MSR */
    rdmsr
    bts     $8, %eax              /* Set LME bit */
    wrmsr

    /*------------------------------------------------------------------
     * Enable paging (CR0.PG = 1) and protection (CR0.PE = 1)
     * Also enable WP (Write Protect) for kernel pages
     *------------------------------------------------------------------*/
    mov     %cr0, %rax
    or      $(1 << 31), %rax      /* PG (paging) */
    or      $(1 << 16), %rax      /* WP (write protect) */
    or      $1, %rax              /* PE (protection enable) */
    mov     %rax, %cr0

    /*------------------------------------------------------------------
     * Now we're in higher-half! The kernel is mapped at
     * 0xFFFF_8000_0010_0000. We need to jump there.
     *------------------------------------------------------------------*/
    /* Far jump to flush pipeline and enter higher-half */
    /* We'll just continue executing since we identity-mapped low mem too */
    /* But we should switch to higher-half stack */

    /* Set up kernel stack (higher-half) */
    mov     $0xFFFF800000200000, %rsp  /* 2MB into higher-half direct map */

    /*------------------------------------------------------------------
     * Enable SSE/FPU (for Rust's use of XMM registers)
     * CR0: clear EM (bit 2), set MP (bit 1)
     * CR4: set OSFXSR (bit 9), OSXMMEXCPT (bit 10)
     *------------------------------------------------------------------*/
    mov     %cr0, %rax
    and     $(~(1 << 2)), %rax    /* Clear EM */
    or      $(1 << 1), %rax       /* Set MP */
    mov     %rax, %cr0

    mov     %cr4, %rax
    or      $(1 << 9), %rax       /* OSFXSR */
    or      $(1 << 10), %rax      /* OSXMMEXCPT */
    mov     %rax, %cr4

    /* Initialize FPU/SSE */
    fninit
    /* FXSAVE/FXRSTOR will work now */

    /*------------------------------------------------------------------
     * Set up IDT (Interrupt Descriptor Table)
     *------------------------------------------------------------------*/
    lidt    idt_descriptor(%rip)

    /*------------------------------------------------------------------
     * Call Rust kernel_init with Multiboot2 info pointer
     *------------------------------------------------------------------*/
    mov     %r12, %rdi            /* First argument: mboot_info_ptr */
    call    kernel_init

    /* If kernel_init returns, halt */
.Lhang:
    hlt
    jmp     .Lhang

.Lmultiboot_fail:
    /* Multiboot magic mismatch — hang */
    mov     $0xDEAD, %eax
    hlt
    jmp     .Lmultiboot_fail

.Lno_long_mode:
    /* Long mode not supported — hang */
    mov     $0xBEEF, %eax
    hlt
    jmp     .Lno_long_mode


/*======================================================================
 * IDT (Interrupt Descriptor Table) — minimal setup
 *======================================================================*/
/* IDT has 256 entries, each 16 bytes (for 64-bit gate) */
.section .rodata
.balign 16
.global idt_descriptor
idt_descriptor:
    .word 256 * 16 - 1            /* Limit (size - 1) */
    .quad idt_table               /* Base address */

.balign 4096
.global idt_table
idt_table:
    /* 256 entries, initially all zero (will be filled by Rust) */
    .fill 256 * 16, 1, 0


/*======================================================================
 * GDT (Global Descriptor Table) — minimal for 64-bit
 *======================================================================*/
.section .rodata
.balign 16
.global gdt_descriptor
gdt_descriptor:
    .word gdt_end - gdt - 1       /* Limit */
    .quad gdt                     /* Base */

gdt:
    .quad 0                       /* Null descriptor */
    .quad 0x00AF9A000000FFFF      /* Kernel code: 64-bit, present, DPL=0, execute/read */
    .quad 0x00CF92000000FFFF      /* Kernel data: 64-bit, present, DPL=0, read/write */
    .quad 0x00AFFA000000FFFF      /* User code: 64-bit, present, DPL=3, execute/read */
    .quad 0x00CFF2000000FFFF      /* User data: 64-bit, present, DPL=3, read/write */
gdt_end:

/*======================================================================
 * Symbols from linker script
 *======================================================================*/
.section .data
.global __bss_start
.global __bss_end
.global __kernel_end
.global __stack_start
.global __stack_end