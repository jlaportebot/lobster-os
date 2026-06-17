/// LobsterOS Boot Entry — AArch64 bare metal on BCM2712 (Raspberry Pi 5)
///
/// The Pi 5 firmware + TF-A BL31 drops us at EL2 (or EL1 with armstub).
/// We need to:
/// 1. Check current EL
/// 2. Drop to EL1 if at EL2
/// 3. Zero BSS
/// 4. Set up stack for core 0
/// 5. Enable FP/SIMD
/// 6. Jump to Rust kernel_init()

    .section .text._boot
    .global _boot
    .type _boot, function

_boot:
    b       .Lel3_entry

.Lel3_entry:
    // -------------------------------------------------------
    // Step 1: Check which Exception Level we're at
    // -------------------------------------------------------
    // Mask all interrupts immediately at EL3
    msr     DAIFSet, #0xF         // Mask IRQ, FIQ, SError, Debug

    mrs     x8, CurrentEL
    lsr     x8, x8, #2          // CurrentEL[3:2] → x8
    
    // DEBUG: Store CurrentEL in a known location for inspection
    ldr     x9, =0x80000
    str     x8, [x9, #0x1000]
    
    cmp     x8, #3
    b.eq    .Ldrop_to_el2       // At EL3 → drop to EL2 (QEMU without TF-A)
    cmp     x8, #2
    b.eq    .Ldrop_to_el1       // At EL2 → drop to EL1
    cmp     x8, #1
    b.eq    .Lel1_setup         // Already at EL1 → proceed
    // Unexpected EL (EL0 or invalid) → hang
    b       .Lhang

.Ldrop_to_el2:
    // -------------------------------------------------------
    // Step 2a: EL3 → EL2 (when booted without TF-A, e.g., QEMU)
    // -------------------------------------------------------

    // Set up EL3 stack (use a small dedicated area)
    ldr     x8, =__stack_start
    add     x8, x8, #0x1000       // 4KB below main stack area
    mov     sp, x8

    // Set up EL3 exception vector table
    ldr     x8, =exception_vector_table
    msr     VBAR_EL3, x8
    isb

    // Set SCR_EL3: enable HCE (HVC), NS=1 (non-secure), RW=1 (AArch64 lower ELs)
    mrs     x8, SCR_EL3
    // SCR_EL3 bits: NS=1 (bit 0), RW=1 (bit 10), HCE=1 (bit 8), SMD=0 (bit 7)
    orr     x8, x8, #(1 << 0)   // NS = 1 (non-secure)
    orr     x8, x8, #(1 << 10)  // RW = 1 (lower ELs are AArch64)
    orr     x8, x8, #(1 << 8)   // HCE = 1 (enable HVC)
    msr     SCR_EL3, x8
    
    // DEBUG: Store SCR_EL3 after modification
    mrs     x8, SCR_EL3
    ldr     x9, =0x80000
    str     x8, [x9, #0x1008]

    // Set HCR_EL2 for EL1 AArch64
    mrs     x8, HCR_EL2
    ldr     x9, =0x80000000     // RW=1, VM=0
    orr     x8, x8, x9
    msr     HCR_EL2, x8

    // Set SCTLR_EL1 to known state
    msr     SCTLR_EL1, xzr

    // Set up EL2→EL1 transition
    ldr     x8, =.Lel1_setup
    msr     ELR_EL2, x8

    // SPSR_EL2: EL1h, IRQ/FIQ/SError masked, AArch64
    //   M[3:0] = 0100 (EL1h), DAIF = 1111
    mov     x8, #0x3C4           // DAIF=0xF, M=0x4 (EL1h)
    msr     SPSR_EL2, x8

    // SPSR_EL3 for EL3→EL2: EL2t, IRQ/FIQ/SError masked, AArch64
    //   M[3:0] = 1000 (EL2t), DAIF = 1111
    mov     x8, #0x3C8           // DAIF=0xF, M=0x8 (EL2t)
    msr     SPSR_EL3, x8

    // Set ELR_EL3 to EL2 entry point
    ldr     x8, =.Lel2_entry
    msr     ELR_EL3, x8

    eret                        // Drop to EL2!

.Lel2_entry:
    // DEBUG: Mark that we reached EL2 entry
    ldr     x8, =0x80000
    mov     x9, #0xDEAD
    str     x9, [x8, #0x1010]

    // DEBUG: Check CurrentEL at EL2 entry
    mrs     x9, CurrentEL
    lsr     x9, x9, #2
    ldr     x8, =0x80000
    str     x9, [x8, #0x1040]

    // Set HCR_EL2 for EL1 AArch64 using raw encoding
    // HCR_EL2: Op0=3, Op1=4, CRn=1, CRm=1, Op2=0 -> S3_4_C1_C1_0
    ldr     x8, =0x80000000     // RW=1, VM=0
    msr     S3_4_C1_C1_0, x8

    // DEBUG: Store a marker after HCR_EL2 write
    mov     x9, #0xFEED
    str     x9, [x8, #0x1030]

    msr     SCTLR_EL1, xzr

    ldr     x8, =.Lel1_setup
    msr     ELR_EL2, x8

    // SPSR_EL2: EL1t, IRQ/FIQ/SError masked, AArch64
    //   M[3:0] = 0100 (EL1t), DAIF = 1111
    mov     x8, #0x3C4           // DAIF=0xF, M=0x4 (EL1t)
    msr     SPSR_EL2, x8

    // DEBUG: Store a marker before eret
    ldr     x8, =0x80000
    mov     x9, #0xFACE
    str     x9, [x8, #0x1038]

    eret                        // Drop to EL1!

.Ldrop_to_el1:
    // -------------------------------------------------------
    // Step 2b: Configure EL2 and drop to EL1 (original path for EL2 entry)
    // -------------------------------------------------------

    // Set HCR_EL2: disable stage-2 translation, EL1 is AArch64
    mrs     x8, HCR_EL2
    // Clear VM bit (bit 0) to disable stage-2 MMU
    // Set RW bit (bit 31) so EL1 is AArch64
    ldr     x9, =0x80000000     // RW=1, VM=0, rest default
    orr     x8, x8, x9
    msr     HCR_EL2, x8

    // Set SCTLR_EL1 to known state before dropping
    msr     SCTLR_EL1, xzr      // Disable MMU, caches at EL1

    // Set the return address and SPSR for ERET
    ldr     x8, =.Lel1_setup
    msr     ELR_EL2, x8

    // SPSR_EL2: EL1h, IRQ/FIQ/SError masked, AArch64
    //   M[3:0] = 0100 (EL1h)
    //   DAIF = 1111 (all masked)
    mov     x8, #0x3C4           // DAIF=0xF, M=0x4 (EL1h)
    msr     SPSR_EL2, x8

    eret                        // Drop to EL1!

.Lel1_setup:
    // DEBUG: Mark that we reached EL1 setup
    ldr     x8, =0x80000
    mov     x9, #0xBEEF
    str     x9, [x8, #0x1018]
    
    // -------------------------------------------------------
    // Check core ID - only core 0 proceeds to kernel_init
    // Other cores spin waiting for PSCI cpu_on
    // -------------------------------------------------------
    mrs     x8, MPIDR_EL1
    and     x8, x8, #0xFF       // Get core ID (Aff0)
    cbnz    x8, .Lsecondary_hold  // If not core 0, go to holding pattern
    
    // -------------------------------------------------------
    // Step 3: EL1 setup (core 0 only)
    // -------------------------------------------------------

    // Disable MMU and caches (should be off already, but be sure)
    mrs     x8, SCTLR_EL1
    // Clear M (bit 0), C (bit 2), I (bit 12)
    bic     x8, x8, #0x1
    bic     x8, x8, #0x4
    bic     x8, x8, #(1 << 12)
    msr     SCTLR_EL1, x8

    // Invalidate caches
    ic      IALLUIS
    dsb     sy
    isb

    // TLB invalidation
    tlbi    VMALLE1IS
    dsb     sy
    isb

    // -------------------------------------------------------
    // Step 4: Zero BSS section
    // -------------------------------------------------------
    ldr     x8, =__bss_start
    ldr     x9, =__bss_end
.Lzero_bss:
    cmp     x8, x9
    b.ge    .Lbss_done
    str     xzr, [x8], #8
    b       .Lzero_bss
.Lbss_done:

    // -------------------------------------------------------
    // Step 5: Set up stack for core 0
    // Stack grows downward. We split the stack area:
    //   Core 0: __stack_start + 0x2000 (8KB offset)
    //   Core 1: __stack_start + 0x4000
    //   Core 2: __stack_start + 0x6000
    //   Core 3: __stack_start + 0x8000
    // -------------------------------------------------------
    mrs     x8, MPIDR_EL1
    and     x8, x8, #0xFF       // Get core ID (Aff0)
    lsl     x8, x8, #11         // core_id * 2048 (0x800)
    ldr     x9, =__stack_start
    add     x9, x9, x8
    add     x9, x9, #0x800      // Top of this core's stack
    mov     sp, x9

    // -------------------------------------------------------
    // Step 6: Enable FP/SIMD (CPACR_EL1.FPEN = 0b11)
    // -------------------------------------------------------
    mrs     x8, CPACR_EL1
    orr     x8, x8, #(3 << 20)  // FPEN = 11 (enable FP at EL0 and EL1)
    msr     CPACR_EL1, x8
    isb

    // -------------------------------------------------------
    // Step 7: Set up exception vector table
    // -------------------------------------------------------
    ldr     x8, =exception_vector_table
    msr     VBAR_EL1, x8
    isb

    // -------------------------------------------------------
    // Step 8: Call kernel_init(x0=DTB pointer)
    // -------------------------------------------------------
    // x0 still holds the DTB pointer from firmware
    bl      kernel_init

    // If kernel_init returns, hang
.Lhang:
    wfe
    b       .Lhang


// ============================================================
// Exception Vector Table (16 entries × 128 bytes each)
// ============================================================
    .balign 2048
    .global exception_vector_table
    .type exception_vector_table, function

exception_vector_table:
    // EL1 with SP0 (shouldn't happen in our kernel)
    .balign 128
    b       .Lexception_unexpected

    .balign 128
    b       .Lexception_unexpected

    .balign 128
    b       .Lexception_unexpected

    .balign 128
    b       .Lexception_unexpected

    // EL1 with SPn (SPx) — this is where kernel exceptions go
    .balign 128
    b       .Lsync_el1h         // Synchronous
    .balign 128
    b       .Lirq_el1h          // IRQ
    .balign 128
    b       .Lfiq_el1h          // FIQ
    .balign 128
    b       .Lserror_el1h       // SError

    // EL0 64-bit — for userspace (Phase 4)
    .balign 128
    b       .Lsync_el0_64
    .balign 128
    b       .Lirq_el0_64
    .balign 128
    b       .Lfiq_el0_64
    .balign 128
    b       .Lserror_el0_64

    // EL0 32-bit — not used
    .balign 128
    b       .Lexception_unexpected
    .balign 128
    b       .Lexception_unexpected
    .balign 128
    b       .Lexception_unexpected
    .balign 128
    b       .Lexception_unexpected


// ============================================================
// Exception Handlers
// ============================================================

// Save all general-purpose registers, call Rust handler, restore
.macro SAVE_CONTEXT
    sub     sp, sp, #272        // 31 × 8 bytes (x0-x30) + 8 padding
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x16, x17, [sp, #128]
    stp     x18, x19, [sp, #144]
    stp     x20, x21, [sp, #160]
    stp     x22, x23, [sp, #176]
    stp     x24, x25, [sp, #192]
    stp     x26, x27, [sp, #208]
    stp     x28, x29, [sp, #224]
    str     x30, [sp, #240]

    // Save special registers
    mrs     x0, SPSR_EL1
    mrs     x1, ELR_EL1
    mrs     x2, ESR_EL1
    mrs     x3, FAR_EL1

    // Pass context pointer and exception info to Rust handler
    mov     x4, sp              // x4 = pointer to saved regs
    // x0 = SPSR, x1 = ELR, x2 = ESR, x3 = FAR, x4 = regs ptr
.endm

.macro RESTORE_CONTEXT
    // Restore special registers
    ldr     x0, [sp, #248]      // We'll store ESR/FAR info separately
    ldp     x0, x1, [sp, #0]    // Actually just restore GPRs
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    ldp     x16, x17, [sp, #128]
    ldp     x18, x19, [sp, #144]
    ldp     x20, x21, [sp, #160]
    ldp     x22, x23, [sp, #176]
    ldp     x24, x25, [sp, #192]
    ldp     x26, x27, [sp, #208]
    ldp     x28, x29, [sp, #224]
    ldr     x30, [sp, #240]
    add     sp, sp, #272
.endm

.Lsync_el1h:
    SAVE_CONTEXT
    bl      handle_sync_exception
    RESTORE_CONTEXT
    eret

.Lirq_el1h:
    SAVE_CONTEXT
    bl      handle_irq
    RESTORE_CONTEXT
    eret

.Lfiq_el1h:
    SAVE_CONTEXT
    bl      handle_fiq
    RESTORE_CONTEXT
    eret

.Lserror_el1h:
    SAVE_CONTEXT
    bl      handle_serror
    RESTORE_CONTEXT
    eret

.Lsync_el0_64:
    SAVE_CONTEXT
    bl      handle_sync_exception_el0
    RESTORE_CONTEXT
    eret

.Lirq_el0_64:
    SAVE_CONTEXT
    bl      handle_irq
    RESTORE_CONTEXT
    eret

.Lfiq_el0_64:
    SAVE_CONTEXT
    bl      handle_fiq
    RESTORE_CONTEXT
    eret

.Lserror_el0_64:
    SAVE_CONTEXT
    bl      handle_serror
    RESTORE_CONTEXT
    eret

.Lexception_unexpected:
    // Unexpected exception — hang with diagnostic
    mrs     x0, ESR_EL1
    mrs     x1, FAR_EL1
    wfe
    b       .Lexception_unexpected


// ============================================================
// Secondary core holding pattern
// Non-core-0 cores spin here waiting for PSCI cpu_on
// ============================================================
.Lsecondary_hold:
    wfe
    b       .Lsecondary_hold

// ============================================================
// Secondary core entry point
// Called when PSCI cpu_on wakes a secondary core
// ============================================================
    .global secondary_entry
    .type secondary_entry, function
secondary_entry:
    // Set up stack for this core
    mrs     x8, MPIDR_EL1
    and     x8, x8, #0xFF       // Get core ID
    lsl     x8, x8, #11         // core_id * 2048
    ldr     x9, =__stack_start
    add     x9, x9, x8
    add     x9, x9, #0x800
    mov     sp, x9

    // Enable FP/SIMD
    mrs     x8, CPACR_EL1
    orr     x8, x8, #(3 << 20)
    msr     CPACR_EL1, x8
    isb

    // Set up exception vector table
    ldr     x8, =exception_vector_table
    msr     VBAR_EL1, x8
    isb

    // Call Rust secondary init with core ID
    mrs     x0, MPIDR_EL1
    and     x0, x0, #0xFF
    bl      secondary_core_init

    // If it returns, hang
    wfe
    b       secondary_entry

.section .data
.global __bss_start
.global __bss_end
.global __kernel_end
.global __stack_start
.global __stack_end
