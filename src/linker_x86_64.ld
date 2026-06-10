ENTRY(_start_x86_64)

PHDRS
{
  text    PT_LOAD FLAGS(5);  /* RX */
  rodata  PT_LOAD FLAGS(4);  /* R */
  data    PT_LOAD FLAGS(6);  /* RW */
}

SECTIONS
{
  /* Kernel load address for x86_64 multiboot/multiboot2 */
  /* We use 1MB + 256KB as a safe default below kernel space */
  . = 0x100000;

  .multiboot_header : {
    *(.multiboot_header)
  } :text

  .text : {
    *(.text._start_x86_64)
    *(.text .text.*)
  } :text

  .rodata : ALIGN(4096) {
    *(.rodata .rodata.*)
  } :rodata

  .data : ALIGN(4096) {
    *(.data .data.*)
  } :data

  .bss : ALIGN(4096) {
    __bss_start = .;
    *(.bss .bss.*)
    *(COMMON)
    __bss_end = .;
  } :data

  . = ALIGN(4096);
  __kernel_end = .;

  /* Stack area — 4 cores × 16KB each = 64KB */
  . = ALIGN(4096);
  __stack_start = .;
  . += 0x10000;  /* 64KB total */
  __stack_end = .;

  /DISCARD/ : {
    *(.comment)
    *(.note.*)
    *(.eh_frame*)
  }
}