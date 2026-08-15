/* Linker script for LobsterOS user programs
 * Compiles to a minimal ELF with text at 0x10000000 (USER_CODE_START).
 * Uses minimal alignment to keep file size down for lobster-net transfer.
 */
ENTRY(_start)

SECTIONS
{
    . = 0x10000000;
    . = ALIGN(4);

    .text : {
        *(.text)
        *(.text.*)
    }

    .rodata : ALIGN(4) {
        *(.rodata)
        *(.rodata.*)
    }

    .data : ALIGN(4) {
        *(.data)
        *(.data.*)
    }

    .bss : ALIGN(4) {
        *(.bss)
        *(.bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.note.gnu.build-id)
        *(.note.*)
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.comment)
        *(.gnu.*)
        *(.ARM.*)
        *(.gnu.warning)
        *(.gnu_debuglink)
    }
}
