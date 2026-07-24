ENTRY(_boot)

PHDRS
{
  text    PT_LOAD FLAGS(5);  /* RX */
  rodata  PT_LOAD FLAGS(4);  /* R */
  data    PT_LOAD FLAGS(6);  /* RW */
}

SECTIONS
{
  /* Kernel load address — Pi 5 firmware loads kernel8.img here */
  . = 0x80000;

  .text : {
    *(.text._boot)
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

  /* Stack area — 4 cores × 4MB each = 16MB (large enough for kernel_main stack probes) */
  . = ALIGN(4096);
  __stack_start = .;
  . += 0x4000000;  /* 16MB total */
  __stack_end = .;

  /DISCARD/ : {
    *(.comment)
    *(.note.*)
    *(.eh_frame*)
  }
}
